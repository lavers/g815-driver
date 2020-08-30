use serde::{Serialize, Deserialize};

pub mod g815;
pub mod scancode;
pub mod rgb;

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
	GameMode = 0x4522, // usual id = 0x08

	// not sure what this one is but it's id (0xf) often comes around setting up lighting
	SomethingLightingRelated = 0x8071 
}

#[derive(Debug)]
pub struct CapabilityData
{
	id: u8,
	key_type: Option<KeyType>,
	key_count: Option<u8>,
	raw: Option<Vec<u8>>
}

impl CapabilityData
{
	pub fn no_capability() -> Self
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
