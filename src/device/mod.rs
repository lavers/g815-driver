use serde::{Serialize, Deserialize};

use scancode::Scancode;
use rgb::{EffectConfiguration, EffectGroup};
use color::Color;
use log::{error, info};

pub mod g815;
pub mod scancode;
pub mod rgb;
pub mod thread;
pub mod color;

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, Deserialize, Serialize)]
pub enum KeyType
{
	GKey,
	Mode,
	GameMode,
	MacroRecord,
	Light,
	MediaControl
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum MediaKey
{
	Next,
	Previous,
	PlayPause,
	VolumeUp,
	VolumeDown,
	Mute
}

#[derive(Debug)]
pub enum DeviceEvent
{
	KeyDown(KeyType, u8),
	KeyUp(KeyType, u8),
	MediaKeyUp(MediaKey),
	MediaKeyDown(MediaKey),
	BrightnessLevelChanged(u8)
}

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub enum Capability
{
	GKeys = 0x8010, // usual id = 0x0a
	ModeSwitching = 0x8020, // usual id = 0x0b
	MacroRecording = 0x8030, // usual id = 0x0c
	BrightnessAdjustment = 0x8040, // usual id = 0x0d
	GameMode = 0x4522 // usual id = 0x08
}

#[derive(Debug)]
pub struct CapabilityData
{
	id: u8,
	key_type: Option<KeyType>,
	key_count: Option<u8>,
	raw: Option<Vec<u8>>
}

impl Default for CapabilityData
{
	fn default() -> Self
	{
		CapabilityData
		{
			id: 0,
			key_type: None,
			key_count: None,
			raw: None
		}
	}
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug)]
pub enum CommandError
{
	HidError(hidapi::HidError),
	LogicError(String),
	Failure(String)
}

pub fn find_devices(hidapi: hidapi::HidApi) -> Vec<Box<dyn Device>>
{
    hidapi
        .device_list()
		.filter_map(|dev|
		{
			let initializer: Option<&dyn Fn(hidapi::HidDevice) -> Box<dyn Device>> =
				match (dev.vendor_id(), dev.product_id(), dev.interface_number())
				{
					(0x046d, 0xc33f, 1) => Some(&g815::G815Keyboard::init),
					_ => None
				};

			let device_name = dev.product_string().unwrap_or("unknown");

			initializer
				.and_then(|initializer| dev
					.open_device(&hidapi)
					.map_err(|e|
					{
						error!("Failed to open target device '{}': {:?}", &device_name, e);
					})
					.map(|device|
					{
						let mut device = initializer(device);
						info!("Successfully opened '{}'\n{}", &device_name, device.firmware_info());
						device
					})
					.ok())
		})
        .collect()
}

pub trait Device where Self: std::fmt::Display + Send
{
	fn take_control(&mut self) -> CommandResult<()>;
	fn release_control(&mut self) -> CommandResult<()>;
	fn mode_count(&self) -> CommandResult<u8>;
	fn set_4(&mut self, keys: &[(Scancode, Color)]) -> CommandResult<()>;
	fn set_13(&mut self, color: Color, keys: &[Scancode]) -> CommandResult<()>;
	fn commit(&mut self) -> CommandResult<()>;
	fn set_mode_leds(&mut self, leds: u8) -> CommandResult<()>;
	fn set_macro_recording(&mut self, recording: bool) -> CommandResult<()>;
	fn set_effect(&mut self, group: EffectGroup, effect: &EffectConfiguration)
		-> CommandResult<()>;
	fn add_game_mode_keys(&mut self, scancodes: &[Scancode]) -> CommandResult<()>;
	fn reset_game_mode_keys(&mut self) -> CommandResult<()>;
	fn get_events(&mut self) -> Vec<DeviceEvent>;
	fn firmware_info(&mut self) -> String;

	fn set_mode(&mut self, mode: u8) -> CommandResult<()>
	{
		self.set_mode_leds(1 << (mode - 1))
	}

	fn apply_scancode_assignments(&mut self, color_map: &[(Color, Vec<Scancode>)])
	{
		for (color, scancodes) in color_map.iter()
		{
			self.set_13(*color, &scancodes);
		}
	}

	fn stop_effects(&mut self)
	{
		self.set_effect(EffectGroup::Keys, &EffectConfiguration::None);
		self.set_effect(EffectGroup::Logo, &EffectConfiguration::None);
	}

	fn clear(&mut self) -> CommandResult<()>
	{
		self.stop_effects();
		self.set_all(Color::black())
	}

	fn set_all(&mut self, color: Color) -> CommandResult<()>
	{
		self.set_13(color, &Scancode::iter_variants().collect::<Vec<Scancode>>())
	}
}

