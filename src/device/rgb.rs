use std::fmt;
use std::collections::HashMap;

use serde::{Serialize, Deserialize, Serializer, Deserializer};
use serde::de::{Visitor, Error};

use crate::device::scancode::Scancode;
use crate::config::Keygroups;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Color
{
	pub r: u8,
	pub g: u8,
	pub b: u8
}

impl Color
{
	pub fn new(r: u8, g: u8, b: u8) -> Self
	{
		Self { r, g, b }
	}

	pub fn black() -> Self
	{
		Self::new(0, 0, 0)
	}
}

impl Default for Color
{
	fn default() -> Self
	{
		Color::black()
	}
}

impl Serialize for Color
{
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		serializer.serialize_str(format!(
			"{:02x}{:02x}{:02x}",
			self.r,
			self.g,
			self.b).as_str())
	}
}

struct ColorVisitor;

impl<'de> Visitor<'de> for ColorVisitor
{
	type Value = Color;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result
	{
		formatter.write_str("a hex color code: #000000")
	}

	fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
	where
		E: Error
	{
		if value.len() == 6
		{
			u64::from_str_radix(value, 16)
				.map_err(|e| E::custom(format!(
					"parse error for hex code {} - {}",
					value,
					e.to_string())))
				.and_then(|number| self.visit_u64(number))
		}
		else
		{
			Err(E::custom(format!("invalid hex color code: {}", value)))
		}
	}

	fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
	where
		E: Error
	{
		Ok(Color::new(
			(value >> 16 & 0xff) as u8,
			(value >> 8 & 0xff) as u8,
			value as u8))
	}

	fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
	where
		E: serde::de::Error
	{
		if value < 0
		{
			Err(E::custom("color as i64 cannot be negative"))
		}
		else
		{
			self.visit_u64(value as u64)
		}
	}
}

impl<'de> Deserialize<'de> for Color
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>
	{
		deserializer.deserialize_str(ColorVisitor)
	}
}

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
pub enum KeySelection
{
	#[serde(rename = "single")]
	Single(Scancode),
	#[serde(rename = "multiple")]
	Multiple(Vec<Scancode>),
	#[serde(rename = "keygroup")]
	Keygroup(String)
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
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
pub enum EffectConfiguration
{
	#[serde(rename = "none")]
	None,
	#[serde(rename = "static")]
	Static { color: Color },
	#[serde(rename = "breathing")]
	Breathing { color: Color, duration: u16, brightness: u8 },
	#[serde(rename = "cycle")]
	Cycle { duration: u16, brightness: u8 },
	#[serde(rename = "color_wave")]
	ColorWave { direction: EffectDirection, duration: u16, brightness: u8 },
	#[serde(rename = "ripple")]
	Ripple { color: Color, duration: u16 }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Theme
{
	#[serde(rename = "custom")]
	Custom(HashMap<Color, Vec<KeySelection>>),
	#[serde(rename = "effect")]
	Effect(EffectConfiguration)
}

pub type ScancodeAssignments = HashMap<Color, Vec<Scancode>>;

impl Theme
{
	/// Turns this theme's set of color to user-friendly keyselections assignments
	/// into a device-friendly map of color -> scancodes. If this theme is an Effect
	/// theme, this will return None.
	pub fn scancode_assignments(&self, keygroups: &Keygroups) -> Option<ScancodeAssignments>
	{
		match self
		{
			Self::Custom(selection_map) => Some(selection_map
				.iter()
				.map(|(color, key_selections)|
				{
					let scancodes = key_selections
						.iter()
						.map(|selection| match selection
						{
							KeySelection::Single(scancode) => vec![*scancode],
							KeySelection::Multiple(scancodes) => scancodes.clone(),
							KeySelection::Keygroup(ref group_name) => keygroups
								.get(group_name)
								.cloned()
								.unwrap_or_default()
						})
						.flatten()
						.collect();

					(*color, scancodes)
				})
				.collect()
			),
			Self::Effect(_effect) => None
		}
	}
}
