use hidapi::{HidApi, HidDevice, HidResult, HidError};

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{DeviceEvent, KeyType, MediaKey, Capability, CapabilityData};

static VID: u16 = 0x046d;
static PID: u16 = 0xc33f;

/*
 * Note: on startup, ghub seems to send an initializer/session nibble 
 * that is then used as the lower nibble of the lower byte of every command
 * All commands in the enum are defined as if the initializer of 11 ff 00 1a
 * has been sent (as a result the last nibble of each command ends in a)
 * If you change InitializeSession, you have to update all other constants
 */
enum Command
{
	InitializeSession = 0x001a,
	SetModeLeds = 0x0b1a, // followed by bitmask of mode key
	Set13 = 0x106a, // followed by r, g, b, [13 keycodes]
	Set4 = 0x101a, // followed by keycode, r, g, b, [ff terminator if < 4]
	// note: set4 also seems to sometimes be 0x101d followed by a commit of 0x107d
	SetEffect = 0x0f1a, // followed by group, effect, r, g, b, [period h..l], [00..00..01]
	Commit = 0x107a,
	ResetGameMode = 0x083a, // removes all non-default game mode key disables
	GameModeAddKeys = 0x081a, // add 1-15 to add keycodes to game mode, send multiple for more
	SetMacroRecordMode = 0x0c0a, // followed by 00 or 01 for in/out of record mode
	SetControlMode = 0x111a, // 01 for hardware, 02 for software
	SetGKeysMode = 0x0a2a, // 00 G-keys in F-key mode, 01 in software mode
	GetVersion = 0x021a,
	CapabilityInfo = 0x000a, // OR this with (capabilityid << 8) to get capability info, or 00 to get capability id
	LightingEnabled = 0x0f7a
}


pub enum ControlMode
{
	Hardware = 0x01,
	Software = 0x02
}

pub enum GKeysMode
{
	Default = 0x00,
	Software = 0x01
}

pub enum EffectGroup
{
	Logo = 0x00,
	Keys = 0x01
}

#[derive(Debug)]
pub enum CommandError
{
	HidError(HidError),
	LogicError(String),
	Failure(String)
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

pub type CommandResult<T> = Result<T, CommandError>;

pub struct G815Keyboard
{
	device: Mutex<HidDevice>,
	capabilities: HashMap<Capability, CapabilityData>,
	capability_id_cache: HashMap<u8, Capability>,
	key_bitmasks: HashMap<KeyType, u8>,
	mode: u8
}

pub struct Color
{
	r: u8,
	g: u8,
	b: u8
}

impl Color
{
	pub fn new(r: u8, g: u8, b: u8) -> Self
	{
		Color { r, g, b }
	}
}

impl G815Keyboard
{
	pub fn new(hidapi: &HidApi) -> HidResult<Self>
	{
		hidapi
			.device_list()
			.find(|dev_info| 
			{
				return dev_info.vendor_id() == VID
					&& dev_info.product_id() == PID
					&& dev_info.interface_number() == 1;
			})
			.ok_or(HidError::OpenHidDeviceError)
			.and_then(|dev_info| dev_info.open_device(&hidapi))
			.map(|device| G815Keyboard 
			{
				device: Mutex::new(device), 
				capabilities: HashMap::new(),
				capability_id_cache: HashMap::new(),
				key_bitmasks: HashMap::new(),
				mode: 1
			})
	}

	fn serial_number(&self) -> String
	{
		self.device
			.lock()
			.unwrap()
			.get_serial_number_string()
			.unwrap()
			.unwrap()
	}

	fn write(&self, command: u16, data: &[u8]) -> CommandResult<Vec<u8>>
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

		// no idea why but this command (something to do with enabling media keys)
		// seems to be the only one that doesn't send a mirrored ACK back, so we 
		// have to watch out for it specifically
		// ^ that was nonsense

		/*
		if command == Command::MediaKeysEnabled as u16
		{
			expected_return[2] = 0xff;
			expected_return[3] = 0x0f;
		}
		*/

		let device = self.device.lock().unwrap();

		device.set_blocking_mode(true)?;
		device.write(&buffer)?;

		println!("OUT(20) > {:0x?}", &buffer);

		for _ in 0..10
		{
			buffer.clear();
			buffer.resize(20, 0);
			let bytes_read = device.read(&mut buffer)?;
			buffer.truncate(bytes_read);

			if bytes_read >= 5 
			{
				if &buffer[..4] == expected_return
				{
					println!("ACK({:2}) > {:0x?}", bytes_read, &buffer);

					buffer.drain(0..std::cmp::min(bytes_read, 4));
					device.set_blocking_mode(false)?;
					return Ok(buffer);
				}
				else
				{
					let mut error_response = expected_return.to_vec();
					error_response.insert(2, 0xff);

					if &buffer[..5] == error_response.as_slice()
					{
						println!("ERR({:2}) > {:0x?}", bytes_read, &buffer);
						return Err(CommandError::Failure(
							format!("device didn't like command {:#?}", &expected_return).into()))
					}
				}
			}
			else
			{
				println!("IN ({:2}) > {:0x?}", bytes_read, &buffer);
			}
		}

		panic!("device sent 10 interrupts that seem to be nonsense");
	}

	fn execute(&self, command: Command, data: &[u8]) -> CommandResult<Vec<u8>>
	{
		self.write(command as u16, data)
	}

	fn version(&self, firmware_bank: u8) -> CommandResult<String>
	{
		let data = self.execute(Command::GetVersion, &vec![firmware_bank])?;

		let name = String::from_utf8_lossy(&data[1..4]);
		let major = 100 + (10 * (data[4] as u16 >> 4)) + (data[4] as u16 & 0xf);
		let minor = (10 * (data[5] as u16 >> 4)) + (data[5] as u16 & 0xf);
		let build = (10 * (data[7] as u16 >> 4)) + (data[7] as u16 & 0xf);

		Ok(format!("{}: {}.{}.{}", name.trim(), major, minor, build).to_string())
	}

	pub fn capability_data(&self, capability: Capability) -> CommandResult<&CapabilityData>
	{
		match self.capabilities.get(&capability)
		{
			Some(capability_data) => Ok(capability_data),
			None => Err(CommandError::LogicError(format!(
				"attempt to get capability_data for non initialized capability '{}'",
				capability as u8)))
		}
	}

	pub fn load_capabilities(&mut self) -> CommandResult<()>
	{
		let capabilities = vec![
			Capability::GKeys,
			Capability::ModeSwitching,
			Capability::GameMode,
			Capability::MacroRecording,
			Capability::BrightnessAdjustment
		];

		let caps = capabilities
			.iter()
			.map(|capability| self.load_capability_data(*capability).map(|_| ()))
			.collect();

		println!("{:#?}", self.capability_id_cache);

		caps
	}

	pub fn load_capability_data(&mut self, capability: Capability) -> CommandResult<&CapabilityData>
	{
		let id_result = self.execute(
			Command::CapabilityInfo, 
			&vec![((capability as u16) >> 8) as u8, capability as u8])?;

		let capability_data = match id_result[0]
		{
			0 => CapabilityData::no_capability(),
			capability_id => 
			{
				let data_command = ((capability_id as u16) << 8) | (Command::CapabilityInfo as u16);
				let data = self.write(data_command, &[0; 0])?;
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
						Capability::BrightnessAdjustment => Some(1),
						_ => None
					},
					key_type: match capability 
					{
						Capability::GKeys => Some(KeyType::GKey),
						Capability::ModeSwitching => Some(KeyType::Mode),
						Capability::GameMode => Some(KeyType::GameMode),
						Capability::MacroRecording => Some(KeyType::MacroRecord),
						Capability::BrightnessAdjustment => Some(KeyType::Light),
						_ => None
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

	pub fn has_capability(&self, capability: Capability) -> bool
	{
		match self.capabilities.get(&capability)
		{
			Some(data) => data.id > 0,
			None => false
		}
	}

	pub fn bootloader_version(&self) -> CommandResult<String>
	{
		self.version(0x00)
	}

	pub fn firmware_version(&self) -> CommandResult<String>
	{
		self.version(0x01)
	}

	pub fn set_13(&self, color: Color, keys: &[u8]) -> CommandResult<()>
	{
		let mut data = vec![color.r, color.g, color.b];
		data.extend(keys);
		self.execute(Command::Set13, &data).map(|_| ())
	}

	pub fn set_4(&self, keys: &[(u8, Color)]) -> CommandResult<()>
	{
		let mut data: Vec<u8> = keys
			.iter()
			.map(|(key, color)| vec![*key, color.r, color.g, color.b])
			.flatten()
			.collect();

		if keys.len() < 4
		{
			data.push(0xff);
		}

		self.execute(Command::Set4, &data).map(|_| ())
	}

	pub fn commit(&self) -> CommandResult<()>
	{
		self.execute(Command::Commit, &[0; 0]).map(|_| ())
	}

	pub fn set_mode(&mut self, mode: u8) -> CommandResult<()>
	{
		let mask = 1 << (mode - 1);
		self.execute(Command::SetModeLeds, &[mask; 1])
			.and_then(|_| 
			{
				self.mode = mode; 
				Ok(())
			})
	}

	pub fn set_control_mode(&self, mode: ControlMode) -> CommandResult<()>
	{
		self.execute(Command::SetControlMode, &[mode as u8; 1]).map(|_| ())
	}

	/// Turns the MR (macro record) light on or off on the keyboard
	/// (doesn't appear to have any effect other than the led)
	pub fn set_macro_recording(&self, recording: bool) -> CommandResult<()>
	{
		self.execute(Command::SetMacroRecordMode, &[recording as u8; 1]).map(|_| ())
	}

	pub fn set_gkeys_mode(&self, mode: GKeysMode) -> CommandResult<()>
	{
		self.execute(Command::SetGKeysMode, &[mode as u8; 1]).map(|_| ())
	}

	pub fn set_effect(&self, group: EffectGroup, effect: u8, color: Color, duration: u16) -> CommandResult<()>
	{
		self.execute(Command::SetEffect, &vec![
			group as u8,
			effect,
			color.r,
			color.g,
			color.b,
			(duration >> 8) as u8,
			duration as u8,
			0x00,
			0x00,
			0x00,
			0x00,
			0x00,
			0x01
		]).map(|_| ())
	}

	pub fn solid_color(&self, group: EffectGroup, color: Color) -> CommandResult<()>
	{
		self.set_effect(group, 1, color, 0x2000)
	}

	pub fn reset_game_mode_keys(&self) -> CommandResult<()>
	{
		self.write(Command::ResetGameMode as u16, &[0; 0]).map(|_| ())
	}

	pub fn add_game_mode_keys(&self, keys: &[u8]) -> CommandResult<()>
	{
		keys
			.chunks(15)
			.map(|key_chunk| self.write(Command::ResetGameMode as u16, key_chunk).map(|_| ()))
			.collect()
	}

	pub fn clear(&self)
	{
		self.execute(Command::SetEffect, &[0; 1]);
		self.execute(Command::SetEffect, &[1; 1]);
	}

	pub fn take_control(&mut self) -> CommandResult<()>
	{
		self.execute(Command::InitializeSession, &[0; 0]);
		self.set_control_mode(ControlMode::Software)?;
		self.set_gkeys_mode(GKeysMode::Software)?;
		self.set_macro_recording(false)?;
		self.set_mode(1)?;
		self.reset_game_mode_keys()?;
		self.execute(Command::LightingEnabled, &[1; 1])?;
		self.clear();
		Ok(())
		//self.solid_color(EffectGroup::Keys, Color::new(255, 0, 0))?;
		//self.solid_color(EffectGroup::Logo, Color::new(0, 0, 255)).map(|_| ())
		//self.write(0x0f5a, &vec![01, 03, 03]).map(|_| ())
		/*
		self.write(0x0f5a, &vec![01, 03, 05])?;
		self.write(Command::MarkStart as u16, &vec![])?;
		self.write(0x0f5a, &vec![01, 03, 05])?;
		self.write(Command::MarkEnd as u16, &vec![]).map(|_| ())
		//self.write(Command::MediaKeysEnabled as u16, &vec![00, 00, 01])?;

		//self.write(0x0f5a, &vec![01, 03, 03]).map(|_| ())
		*/
	}

	pub fn release_control(&self) -> CommandResult<()>
	{
		self.set_gkeys_mode(GKeysMode::Default)?;
		self.set_control_mode(ControlMode::Hardware)
	}

	pub fn poll_for_events(&mut self) -> Vec<DeviceEvent>
	{
		let mut buffer = [0; 20];
		let bytes_read = self.device.lock().unwrap().read(&mut buffer).unwrap_or(0);

		if bytes_read < 1
		{
			return Vec::new()
		}

		if buffer[0] == 0x03
		{
			return self.handle_media_key_interrupt(buffer[1])
		}

		// if it's not a media key or a capability key then ignore it
		// note: 11 ff 0f 10 [00/01] comes up a lot but idk what it means

		if buffer[0] != 0x11 || buffer[1] != 0xff
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

		self.key_bitmasks.insert(KeyType::MediaControl, current_bitmask);

		(0..7)
			.filter(|bit| (change_bitmask >> bit) & 0x1 == 0x1)
			.filter_map(|bit| 
			{
				let key = match 1 << bit
				{
					0x01 => Some(MediaKey::PlayPause),
					0x02 => Some(MediaKey::Previous),
					0x08 => Some(MediaKey::Next),
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
