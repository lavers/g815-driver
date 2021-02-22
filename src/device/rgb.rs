use serde::{Serialize, Deserialize};

use crate::device::scancode::Scancode;
use crate::config::Keygroups;
pub use crate::device::color::Color;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Effect
{
	Off,
	Static,
	Cycle = 0x03, // or 0x02 when on the logo group?!
	ColorWave = 0x04 // doesn't seem to set the logo at all?
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum EffectGroup
{
	Logo = 0x00,
	Keys = 0x01
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySelection
{
	Single(Scancode),
	Multiple(Vec<Scancode>),
	Keygroup(String)
}

impl KeySelection
{
	pub fn scancodes(&self, keygroups: &Keygroups) -> Vec<Scancode>
	{
		match self
		{
			Self::Single(scancode) => vec![*scancode],
			Self::Multiple(scancodes) => scancodes.clone(),
			Self::Keygroup(group_name) => keygroups
				.get(group_name)
				.cloned()
				.unwrap_or_default()
		}
	}
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDirection
{
	Horizontal = 0x01,
	Vertical = 0x02,
	CenterOut = 0x03,
	CenterIn = 0x08,
	ReverseHorizontal = 0x06,
	ReverseVertical = 0x07
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectConfiguration
{
	None,
	Static { color: Color },
	Breathing { color: Color, duration: u16, brightness: u8 },
	Cycle { duration: u16, brightness: u8 },
	ColorWave { direction: EffectDirection, duration: u16, brightness: u8 },
	Ripple { color: Color, duration: u16 }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ColorAssignment
{
	color: Color,
	keys: Vec<KeySelection>
}

impl ColorAssignment
{
	pub fn scancodes(&self, keygroups: &Keygroups) -> Vec<Scancode>
	{
		self.keys
			.iter()
			.map(|selection| selection.scancodes(keygroups))
			.flatten()
			.collect()
	}
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Theme
{
	Static(Vec<ColorAssignment>),
	Effect(EffectConfiguration)
}

pub type ScancodeAssignments = Vec<(Color, Vec<Scancode>)>;

impl Theme
{
	/// Turns this theme's set of color to user-friendly keyselections assignments
	/// into a device-friendly map of color -> scancodes. If this theme is an Effect
	/// theme, this will return None.
	pub fn scancode_assignments(&self, keygroups: &Keygroups) -> Option<ScancodeAssignments>
	{
		match self
		{
			Self::Static(assignments) => Some(assignments
				.iter()
				.map(|assignment| (assignment.color, assignment.scancodes(keygroups)))
				.collect()),
			Self::Effect(_effect) => None
		}
	}
}
