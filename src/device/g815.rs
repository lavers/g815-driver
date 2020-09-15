use std::collections::{HashMap, VecDeque};
use std::fmt;

use hidapi::{HidDevice, HidError};
use log::{trace, debug};

use super::{DeviceEvent, KeyType, MediaKey, Capability, CapabilityData, CommandResult, CommandError};
use super::rgb::{Color, EffectConfiguration, EffectGroup};
use super::scancode::Scancode;

/*
 * Note: on startup, ghub seems to send an initializer/session nibble
 * that is then used as the lower nibble of the lower byte of every command
 * All commands in the enum are defined as if the initializer of 11 ff 00 1a
 * has been sent (as a result the last nibble of each command ends in a)
 * If you change InitializeSession, you have to update all other constants
 */

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
enum Command
{
	InitializeSession = 0x001a,
	SetModeLeds = 0x0b1a, // followed by bitmask of mode key leds
	Set13 = 0x106a, // followed by r, g, b, [13 keycodes]
	Set4 = 0x101a, // followed by (keycode, r, g, b){1,4}, [ff terminator if < 4]
	SetEffect = 0x0f1a, // followed by group, effect, r, g, b, [period h..l], [00..00..01]
	Commit = 0x107a,
	ResetGameMode = 0x083a, // removes all non-default game mode key disables
	GameModeAddKeys = 0x081a, // followed by (usb scancode){1,15}
	SetMacroRecordMode = 0x0c0a, // followed by 00 or 01 for MR led off/on
	SetControlMode = 0x111a, // 01 for hardware, 02 for software
	SetGKeysMode = 0x0a2a, // 00 G-keys in F-key mode, 01 in software mode
	GetVersion = 0x021a,
	CapabilityInfo = 0x000a, // OR this with (capabilityid << 8) to get capability info, otherwise id
	LightingEnabled = 0x0f7a,
	EffectsEnabled = 0x0f5a
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum ControlMode
{
	Hardware = 0x01,
	Software = 0x02
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum GKeysMode
{
	Default = 0x00,
	Software = 0x01
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum Effect
{
	None = 0x00,
	Static = 0x01,
	Breathing = 0x02, // 0x03 for logo?!
	Cycle = 0x03, // 0x02 for logo?!
	ColorWave = 0x04,
	Ripple = 0x05
}

impl From<HidError> for CommandError
{
	fn from(error: HidError) -> Self
	{
		CommandError::HidError(error)
	}
}

impl From<String> for CommandError
{
	fn from(message: String) -> Self
	{
		CommandError::Failure(message)
	}
}

pub struct G815Keyboard
{
	device: HidDevice,
	capabilities: HashMap<Capability, CapabilityData>,
	capability_id_cache: HashMap<u8, Capability>,
	key_bitmasks: HashMap<KeyType, u8>,
	mode_leds: u8,
	interrupt_queue: VecDeque<Vec<u8>>
}

impl G815Keyboard
{
	pub fn new(device: HidDevice) -> Box<dyn super::Device>
	{
		let mut keyboard = G815Keyboard
		{
			device,
			capabilities: HashMap::new(),
			capability_id_cache: HashMap::new(),
			key_bitmasks: HashMap::new(),
			interrupt_queue: VecDeque::new(),
			mode_leds: 0x0
		};

		keyboard.load_capabilities();
		Box::new(keyboard)
	}

	pub fn serial_number(&self) -> String
	{
		self.device
			.get_serial_number_string()
			.unwrap()
			.unwrap()
	}

	fn write(&mut self, command: u16, data: &[u8]) -> CommandResult<Vec<u8>>
	{
		let command_bytes = command as u16;

		let mut buffer = Vec::new();
		buffer.push(0x11);
		buffer.push(0xff);
		buffer.push((command_bytes >> 8) as u8);
		buffer.push(command_bytes as u8);
		buffer.extend(data);
		buffer.resize(20, 0);

		let mut expected_return = [0; 4];
		expected_return.clone_from_slice(&buffer[..4]);

		self.device.set_blocking_mode(true)?;
		self.device.write(&buffer)?;

		trace!("OUT {:02x?}", &buffer);

		for _ in 0..30
		{
			buffer.clear();
			buffer.resize(20, 0);
			let bytes_read = self.device.read(&mut buffer)?;
			buffer.truncate(bytes_read);

			if bytes_read >= 4 && buffer[..4] == expected_return
			{
				trace!("ACK {:02x?}", &buffer);

				buffer.drain(0..std::cmp::min(bytes_read, 4));
				self.device.set_blocking_mode(false)?;
				return Ok(buffer);
			}
			else if bytes_read >= 5
			{
				let mut error_response = expected_return.to_vec();
				error_response.insert(2, 0xff);

				if &buffer[..5] == error_response.as_slice()
				{
					trace!("ERR {:02x?}", &buffer);
					return Err(CommandError::Failure(
						format!("device didn't like command {:#?}", &expected_return)))
				}
			}

			trace!("IN {:02x?}", &buffer);
			self.interrupt_queue.push_back(buffer.clone());
		}

		panic!("device sent 30 interrupts without an acknowledgement or error response");
	}

	fn execute(&mut self, command: Command, data: &[u8]) -> CommandResult<Vec<u8>>
	{
		self.write(command as u16, data)
	}

	fn version(&mut self, firmware_bank: u8) -> CommandResult<String>
	{
		let data = self.execute(Command::GetVersion, &[firmware_bank])?;

		let name = String::from_utf8_lossy(&data[1..4]);
		let major = 100 + (10 * (data[4] as u16 >> 4)) + (data[4] as u16 & 0xf);
		let minor = (10 * (data[5] as u16 >> 4)) + (data[5] as u16 & 0xf);
		let build = (10 * (data[7] as u16 >> 4)) + (data[7] as u16 & 0xf);

		Ok(format!("{}: {}.{}.{}", name.trim(), major, minor, build))
	}

	fn capability_data(&self, capability: Capability) -> CommandResult<&CapabilityData>
	{
		match self.capabilities.get(&capability)
		{
			Some(capability_data) => Ok(capability_data),
			None => Err(CommandError::LogicError(format!(
				"attempt to get capability_data for non initialized capability '{}'",
				capability as u8)))
		}
	}

	fn load_capabilities(&mut self) -> CommandResult<()>
	{
		let capabilities = [
			Capability::GKeys,
			Capability::ModeSwitching,
			Capability::GameMode,
			Capability::MacroRecording,
			Capability::BrightnessAdjustment
		];

		let capabilities = capabilities
			.iter()
			.map(|capability| self.load_capability_data(*capability).map(|_| ()))
			.collect();

		trace!("capability id cache: {:#0x?}", &self.capability_id_cache);
		capabilities
	}

	fn load_capability_data(&mut self, capability: Capability) -> CommandResult<&CapabilityData>
	{
		let id_result = self.execute(
			Command::CapabilityInfo,
			&[((capability as u16) >> 8) as u8, capability as u8])?;

		debug!("loading data for capability {:?}, id is: {:#04x}", capability, id_result[0]);

		let capability_data = match id_result[0]
		{
			0 => CapabilityData::default(),
			capability_id =>
			{
				let data_command = ((capability_id as u16) << 8) | (Command::CapabilityInfo as u16);
				let data = self.write(data_command, &[0; 0])?;

				debug!("capability data: {:02x?}", &data);

				let mut cap_data = CapabilityData
				{
					id: capability_id,
					raw: None,
					key_count: match capability
					{
						Capability::GKeys => Some(data[0]),
						Capability::ModeSwitching => Some(data[0]),
						Capability::GameMode => Some(1),
						Capability::MacroRecording => Some(1),
						Capability::BrightnessAdjustment => Some(1)
					},
					key_type: match capability
					{
						Capability::GKeys => Some(KeyType::GKey),
						Capability::ModeSwitching => Some(KeyType::Mode),
						Capability::GameMode => Some(KeyType::GameMode),
						Capability::MacroRecording => Some(KeyType::MacroRecord),
						Capability::BrightnessAdjustment => Some(KeyType::Light)
					}
				};

				cap_data.raw = Some(data);
				cap_data
			}
		};

		self.capability_id_cache.insert(capability_data.id, capability);

		let data_ref = self.capabilities
			.entry(capability)
			.insert(capability_data)
			.into_mut();

		Ok(data_ref)
	}

	fn has_capability(&self, capability: Capability) -> bool
	{
		match self.capabilities.get(&capability)
		{
			Some(data) => data.id > 0,
			None => false
		}
	}

	fn bootloader_version(&mut self) -> CommandResult<String>
	{
		self.version(0x00)
	}

	fn firmware_version(&mut self) -> CommandResult<String>
	{
		self.version(0x01)
	}

	fn set_control_mode(&mut self, mode: ControlMode) -> CommandResult<()>
	{
		self.execute(Command::SetControlMode, &[mode as u8; 1]).map(|_| ())
	}

	fn set_gkeys_mode(&mut self, mode: GKeysMode) -> CommandResult<()>
	{
		self.execute(Command::SetGKeysMode, &[mode as u8; 1]).map(|_| ())
	}

	fn events_from_interrupt(&mut self, buffer: &[u8]) -> Vec<DeviceEvent>
	{
		if buffer[0] == 0x03
		{
			return self.handle_media_key_interrupt(buffer[1])
		}

		// if it's not a media key or a capability key then ignore it
		// note: 11 ff 0f 10 [00/01] comes in regularly, seems to be effect cycle done/restarting?

		if buffer.len() < 3 || buffer[0] != 0x11 || buffer[1] != 0xff
		{
			return Vec::new()
		}

		match self.capability_id_cache.get(&buffer[2])
		{
			Some(capability) =>
			{
				let cap_id = *capability;
				self.handle_capability_key_interrupt(cap_id, &buffer[4..])
			},
			None => Vec::new()
		}
	}

	fn handle_media_key_interrupt(&mut self, current_bitmask: u8) -> Vec<DeviceEvent>
	{
		let previous_bitmask = self.key_bitmasks.get(&KeyType::MediaControl).unwrap_or(&0);
		let change_bitmask = previous_bitmask ^ current_bitmask;

		debug!(
			"media key interrupt:\n\tprevious {:08b}\n\tcurrent  {:08b}\n\tchange   {:08b}",
			&previous_bitmask,
			&current_bitmask,
			&change_bitmask);

		self.key_bitmasks.insert(KeyType::MediaControl, current_bitmask);

		(0..7)
			.filter(|bit| (change_bitmask >> bit) & 0x1 == 0x1)
			.filter_map(|bit|
			{
				let key = match 1 << bit
				{
					0x01 => Some(MediaKey::Next),
					0x02 => Some(MediaKey::Previous),
					0x08 => Some(MediaKey::PlayPause),
					0x10 => Some(MediaKey::VolumeUp),
					0x20 => Some(MediaKey::VolumeDown),
					0x40 => Some(MediaKey::Mute),
					_ => None
				};

				key.map(|key| (bit, key))
			})
			.map(|(bit, key)| match (current_bitmask >> bit) & 0x1 == 0x1
			{
				true => DeviceEvent::MediaKeyDown(key),
				false => DeviceEvent::MediaKeyUp(key)
			})
			.collect()
	}

	/// Handles conversion of a capability key interrupt (gkeys, mode keys etc)
	/// into a list of device events based on previous and current bitmasks
	fn handle_capability_key_interrupt(&mut self, capability: Capability, data: &[u8])
		-> Vec<DeviceEvent>
	{
		let capability_data = self.capability_data(capability).unwrap();
		let key_type = capability_data.key_type.unwrap();

		match key_type
		{
			KeyType::Light => vec![DeviceEvent::BrightnessLevelChanged(data[1])],
			KeyType::GKey
				| KeyType::GameMode
				| KeyType::MacroRecord
				| KeyType::Mode =>
			{
				let key_count = capability_data.key_count.unwrap();
				let current_bitmask = data[0];
				let previous_bitmask = self.key_bitmasks.get(&key_type).unwrap_or(&0);
				let change_bitmask = previous_bitmask ^ current_bitmask;

				debug!("capability key interrupt:\n\tcapability {:?}\n\t \
					previous {:08b}\n\tcurrent  {:08b}\n\tchange   {:08b}",
					&capability,
					&previous_bitmask,
					&current_bitmask,
					&change_bitmask);

				self.key_bitmasks.insert(key_type, current_bitmask);

				(0..key_count)
					.filter(|key| (change_bitmask >> key) & 0x1 == 0x1)
					.map(|key| match (current_bitmask >> key) & 0x1 == 0x1
					{
						false => DeviceEvent::KeyUp(key_type, key + 1),
						true => DeviceEvent::KeyDown(key_type, key + 1)
					})
					.collect()
			},
			_ => Vec::new()
		}
	}
}

impl fmt::Display for G815Keyboard
{
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result
	{
		write!(f, "{}\nSerial: {}",
			self.device.get_product_string()
				.unwrap_or_else(|e| Some(format!("{:?}", e)))
				.unwrap_or_else(|| "unknown product string".to_string()),
			self.serial_number())
	}
}

impl super::Device for G815Keyboard
{
	fn firmware_info(&mut self) -> String
	{
		format!(
			"Firmware: {}\nBootloader: {}",
			self.firmware_version().unwrap_or_else(|e| format!("{:?}", e)),
			self.bootloader_version().unwrap_or_else(|e| format!("{:?}", e)))
	}

	fn take_control(&mut self) -> CommandResult<()>
	{
		self.execute(Command::InitializeSession, &[0; 0])?;
		self.set_control_mode(ControlMode::Software)?;
		self.set_gkeys_mode(GKeysMode::Software)?;
		self.set_macro_recording(false)?;
		self.set_mode(1)?;
		self.reset_game_mode_keys()?;
		self.execute(Command::LightingEnabled, &[1; 1])?;
		// TODO don't know what these numbers do, last byte can be 0x03, 0x05, 0x07
		// none of them seem to have any visual effect on a running effect but it must
		// be called for effects to work
		self.execute(Command::EffectsEnabled, &[0x01, 0x03, 0x03])?;
		self.stop_effects();
		self.clear();
		Ok(())
	}

	fn release_control(&mut self) -> CommandResult<()>
	{
		self.set_macro_recording(false)?;
		self.set_gkeys_mode(GKeysMode::Default)?;
		self.set_control_mode(ControlMode::Hardware)
	}

	fn mode_count(&self) -> CommandResult<u8>
	{
		self.capability_data(Capability::ModeSwitching)
			.map(|data| data.key_count.unwrap_or(0))
	}

	fn set_4(&mut self, keys: &[(Scancode, Color)]) -> CommandResult<()>
	{
		keys.chunks(4).map(|keys|
		{
			let mut data: Vec<u8> = keys
				.iter()
				.map(|(key, color)| vec![key.rgb_id(), color.r, color.g, color.b])
				.flatten()
				.collect();

			if keys.len() < 4
			{
				data.push(0xff);
			}

			self.execute(Command::Set4, &data).map(|_| ())
		})
		.collect()
	}

	fn set_13(&mut self, color: Color, keys: &[Scancode]) -> CommandResult<()>
	{
		let mut data = [0; 16];
		data[0] = color.r;
		data[1] = color.g;
		data[2] = color.b;

		keys
			.chunks(13)
			.map(|chunk|
			{
				chunk
					.iter()
					.enumerate()
					.for_each(|(i, scancode)| data[i + 3] = scancode.rgb_id());

				self.execute(Command::Set13, &data).map(|_| ())
			})
			.collect()
	}

	fn commit(&mut self) -> CommandResult<()>
	{
		self.execute(Command::Commit, &[0; 0]).map(|_| ())
	}

	fn set_mode_leds(&mut self, mask: u8) -> CommandResult<()>
	{
		match self.mode_leds ^ mask
		{
			0 => Ok(()),
			_=>
			{
				self.mode_leds = mask;
				self.execute(Command::SetModeLeds, &[self.mode_leds; 1]).map(|_| ())
			}
		}
	}

	fn set_macro_recording(&mut self, recording: bool) -> CommandResult<()>
	{
		self.execute(Command::SetMacroRecordMode, &[recording as u8; 1]).map(|_| ())
	}

	fn set_effect(&mut self, group: EffectGroup, effect: &EffectConfiguration)
		-> CommandResult<()>
	{
		let mut data = [
			group as u8,
			0, // effect id
			0, // r
			0, // g
			0, // b
			0, // [5] duration high byte for breathing, 0x02 for fixed, otherwise 0
			0, // duration low byte for breathing
			0, // duration high byte for cycle, ripple (only one byte)
			0, // duration low byte for cycle, color wave & brightness for breathing
			0, // brightness for cycle, direction for color wave
			0, // brightness for color wave
			0, // duration high for color wave
			// always ends with this
			1, 0, 0, 0
		];

		match effect
		{
			EffectConfiguration::None =>
			{
				data[1] = Effect::None as u8;
			},
			EffectConfiguration::Static { color } =>
			{
				data[1] = Effect::Static as u8;
				data[2] = color.r;
				data[3] = color.g;
				data[4] = color.b;
				data[5] = 0x02;
			},
			EffectConfiguration::Breathing { color, duration, brightness } =>
			{
				data[1] = Effect::Breathing as u8;
				data[2] = color.r;
				data[3] = color.g;
				data[4] = color.b;
				data[5] = (duration >> 8) as u8;
				data[6] = *duration as u8;
				data[7] = *brightness;
			},
			EffectConfiguration::Cycle { duration, brightness } =>
			{
				data[1] = Effect::Cycle as u8;
				data[7] = (duration >> 8) as u8;
				data[8] = *duration as u8;
				data[9] = *brightness;
			},
			EffectConfiguration::ColorWave { direction, duration, brightness } =>
			{
				data[1] = Effect::ColorWave as u8;
				data[8] = *duration as u8;
				data[9] = *direction as u8;
				data[10] = *brightness;
				data[11] = (duration >> 8) as u8;
			}
			EffectConfiguration::Ripple { color, duration } =>
			{
				// this is ghubs limit so we'll also use it
				if *duration > 200
				{
					return Err(CommandError::Failure("duration for ripple must be <= 200".into()))
				}

				data[1] = Effect::Ripple as u8;
				data[2] = color.r;
				data[3] = color.g;
				data[4] = color.b;
				data[7] = *duration as u8;
			}
		}

		self.execute(Command::SetEffect, &data).map(|_| ())
	}

	fn add_game_mode_keys(&mut self, scancodes: &[Scancode]) -> CommandResult<()>
	{
		scancodes
			.iter()
			.filter_map(|code| match code
			{
				// ghub doesn't let you add these to game mode so we probably
				// shouldnt either

				Scancode::LeftMeta
					| Scancode::RightMeta
					| Scancode::ContextMenu
					| Scancode::Mute
					| Scancode::Light
					| Scancode::G1
					| Scancode::G2
					| Scancode::G3
					| Scancode::G4
					| Scancode::G5
					| Scancode::Logo
					| Scancode::MediaPrevious
					| Scancode::MediaNext
					| Scancode::MediaPlayPause => None,
				code => Some(*code as u8)
			})
			.collect::<Vec<u8>>()
			.chunks(15) // last byte always seems to be 00 even if there are more than 15
			.map(|scancodes| self.write(Command::GameModeAddKeys as u16, scancodes).map(|_| ()))
			.collect()
	}

	fn reset_game_mode_keys(&mut self) -> CommandResult<()>
	{
		self.write(Command::ResetGameMode as u16, &[0; 0]).map(|_| ())
	}

	fn get_events(&mut self) -> Vec<DeviceEvent>
	{
		let mut interrupt_buffers: Vec<Vec<u8>> = self.interrupt_queue.drain(..).collect();
		let mut buffer = [0; 20];
		let bytes_read = self.device.read(&mut buffer).unwrap_or(0);

		if !interrupt_buffers.is_empty() || bytes_read > 0
		{
			debug!("device polled: {} buffers in the interrupt queue, {} bytes read just now",
			   interrupt_buffers.len(),
			   bytes_read);
		}

		if bytes_read > 0
		{
			trace!("IN {:02x?}", &buffer);
			interrupt_buffers.push(buffer.to_vec());
		}

		interrupt_buffers
			.iter()
			.map(|interrupt_data| self.events_from_interrupt(&interrupt_data))
			.flatten()
			.collect()
	}
}
