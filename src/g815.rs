use hidapi::{HidApi, HidDevice, HidResult, HidError};

use std::collections::HashMap;

static VID: u16 = 0x046d;
static PID: u16 = 0xc33f;

enum Command
{
	ActivateMode = 0x0b1a, // followed by bitmask of mode key
	Set13 = 0x106a, // followed by r, g, b, [13 keycodes]
	Set4 = 0x101a, // followed by keycode, r, g, b, [ff terminator if < 4]
	SetEffect = 0x0f1a, // followed by group, effect, r, g, b, [period h..l], [00..00..01]
	Commit = 0x107a,
	MarkStart = 0x083a, // usually before sending group of effects
	MarkEnd = 0x081a, // usually after sending effects
	SetMacroRecordMode = 0x0c0a, // followed by 00 or 01 for in/out of record mode
	SetControlMode = 0x111a, // 01 for hardware, 02 for software
	SetGKeysMode = 0x0a2a, // 00 G-keys in F-key mode, 01 in software mode
	GetVersion = 0x021a,
	CapabilityInfo = 0x000a, // OR this with (capabilityid << 8) to get capability info, or 00 to get capability id
	LightingEnabled = 0xf7a
}

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub enum Capability
{
	GKeys = 0x8010, // usual id = 0x0a
	ModeKeys = 0x8020, // usual id = 0x0b
	RecordMacroKey = 0x8030, // usual id = 0x0c
	BrightnessKey = 0x8040, // usual id = 0x0d
	GameModeKey = 0x4522, // usual id = 0x08

	// not sure what this one is but it's id (0xf) often comes around setting up lighting
	SomethingLightingRelated = 0x8071 
}

enum ControlMode
{
	Hardware = 0x01,
	Software = 0x02
}

enum GKeysMode
{
	Default = 0x00,
	Software = 0x01
}

enum EffectGroup
{
	Logo = 0x00,
	Keys = 0x01
}

pub enum MacroRecordMode
{
	Default = 0x00,
	Recording = 0x01
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
	device: HidDevice,
	capability_ids: HashMap<Capability, u8>
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
				device, 
				capability_ids: HashMap::new() 
			})
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

		println!("executing: {:x?}", &buffer);

		self.device.write(&buffer)?;

		buffer.clear();
		buffer.resize(20, 0);

		let bytes_read = self.device.read(&mut buffer)?;

		buffer.truncate(bytes_read);

		println!("received ({} bytes): {:x?}", bytes_read, &buffer);

		buffer.drain(0..std::cmp::min(bytes_read, 4));

		// TODO compare removed elements against 11 ff <command> to make sure we got a
		// reply to what we sent..?

		Ok(buffer)
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

	fn capability_id(&mut self, capability: Capability) -> CommandResult<u8>
	{
		let id = capability as u16;

		self
			.execute(Command::CapabilityInfo, &vec![(id >> 8) as u8, id as u8])
			.map(|data| 
			{
				let capability_id = data[0];
				self.capability_ids.insert(capability, capability_id);
				capability_id
			})
	}

	pub fn capability_data(&mut self, capability: Capability) -> CommandResult<Vec<u8>>
	{
		let capability_id = match self.capability_ids.get(&capability)
		{
			Some(capability_id) => *capability_id,
			None => self.capability_id(capability)?
		} as u16;

		let command = (capability_id << 8) | (Command::CapabilityInfo as u16);

		self.write(command, &[0; 0])
	}

	pub fn has_capability(&self, capability: Capability) -> bool
	{
		match self.capability_ids.get(&capability)
		{
			Some(capability_id) => *capability_id > 0,
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

	pub fn activate_mode(&self, mode: u8) -> CommandResult<()>
	{
		self.execute(Command::ActivateMode, &[mode; 1]).map(|_| ())
	}

	pub fn set_control_mode(&self, mode: ControlMode) -> CommandResult<()>
	{
		self.execute(Command::SetControlMode, &[mode as u8; 1]).map(|_| ())
	}

	pub fn set_macro_record_mode(&self, mode: MacroRecordMode) -> CommandResult<()>
	{
		self.execute(Command::SetMacroRecordMode, &[mode as u8; 1]).map(|_| ())
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

	fn consume_interrupts(&mut self, count: u8) -> CommandResult<()>
	{
		let mut buffer = [0; 20];

		for i in 0..count
		{
			self.device.read(&mut buffer)?;
			println!("discarding: {:x?}", &buffer);
		}

		Ok(())
	}

	fn solid_color(&self, group: EffectGroup, color: Color) -> CommandResult<()>
	{
		self.set_effect(group, 1, color, 0x2000)
	}

	pub fn take_control(&mut self) -> CommandResult<()>
	{
		self.set_control_mode(ControlMode::Software)?;
		self.consume_interrupts(1)?;
		self.set_gkeys_mode(GKeysMode::Software)?;
		self.consume_interrupts(5)?;
		self.activate_mode(1)?;
		self.consume_interrupts(2)?;
		self.solid_color(EffectGroup::Keys, Color::new(255, 0, 0))?;
		self.solid_color(EffectGroup::Logo, Color::new(0, 0, 255))?;
		self.write(0x0f7a, &vec![1]).map(|_| ())
		//self.write(0x0f5a, &vec![1, 3, 3]).map(|_| ())
	}

	pub fn release_control(&mut self) -> CommandResult<()>
	{
		self.set_gkeys_mode(GKeysMode::Default)?;
		self.consume_interrupts(4)?;
		self.set_control_mode(ControlMode::Hardware)?;
		self.consume_interrupts(2)
	}

	pub fn interrupt_watch(&mut self, duration: std::time::Duration)
	{
		let start = std::time::Instant::now();
		self.device.set_blocking_mode(false);

		while start.elapsed() < duration
		{
			let bytes_read = self.device.read(&mut self.scratch_buffer).unwrap();

			if bytes_read > 0
			{
				println!("interrupt {:0x?}", &self.scratch_buffer);
			}

			std::thread::sleep(std::time::Duration::from_millis(1));
		}

		self.device.set_blocking_mode(true);

		println!("done")
	}
}
