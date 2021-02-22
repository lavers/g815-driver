use std::convert::{TryFrom, TryInto};
use std::fmt;

use hsl::HSL;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use serde::de::{Visitor, Error};

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

impl From<HSL> for Color
{
	fn from(hsl: HSL) -> Self
	{
		let (r, g, b) = hsl.to_rgb();
		Self::new(r, g, b)
	}
}

impl From<u32> for Color
{
	fn from(color: u32) -> Self
	{
		Color::new(
			(color >> 16 & 0xff) as u8,
			(color >> 8 & 0xff) as u8,
			color as u8)
	}
}

impl TryFrom<&str> for Color
{
	type Error = std::num::ParseIntError;

	fn try_from(string: &str) -> Result<Self, Self::Error>
	{
		u32::from_str_radix(string, 16)
			.map(|color| color.into())
	}
}

impl std::fmt::Display for Color
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error>
	{
		f.write_fmt(format_args!("rgb({}, {}, {})", self.r, self.g, self.b))
	}
}

impl std::fmt::LowerHex for Color
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error>
	{
		f.write_fmt(format_args!("{:02x}{:02x}{:02x}", self.r, self.g, self.b))
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
		formatter.write_str("a hex color code: 00ff00")
	}

	fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
	where
		E: Error
	{
		if value.len() == 6
		{
			value
				.try_into()
				.map_err(|e: std::num::ParseIntError| E::custom(format!(
					"parse error for hex code {} - {}",
					value,
					e.to_string())))
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
		Ok((value as u32).into())
	}

	fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
	where
		E: Error
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
