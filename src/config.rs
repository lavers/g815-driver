use std::collections::HashMap;

use serde::{Serialize, Deserialize, Serializer, Deserializer, de::Error};

use regex::Regex;

use crate::windowsystem::ActiveWindowInfo;
use crate::device::scancode::Scancode;
use crate::device::rgb::{Theme, Color, ScancodeAssignments};

#[derive(Debug)]
pub enum ConfigError
{
	UnableToOpen(std::io::Error),
	UnableToWrite(std::io::Error),
	ParseError(serde_yaml::Error),
	SerializeError(serde_yaml::Error),
	InvalidConfiguration(String)
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MacroKeyAssignment
{
	#[serde(rename = "simple_action")]
	SimpleAction(crate::macros::Action),
	#[serde(rename = "run_macro")]
	RunMacro(String)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ModeProfile
{
	theme: Option<String>,
	gkey_sets: Option<Vec<String>>,
	gkeys: Option<HashMap<u8, MacroKeyAssignment>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Profile
{
	conditions: Option<ActiveWindowConditions>,
	pub theme: Option<String>,
	gkey_sets: Option<Vec<String>>,
	gkeys: Option<HashMap<u8, MacroKeyAssignment>>,
	modes: Option<HashMap<u8, ModeProfile>>
}

pub type Keygroup = Vec<Scancode>;
pub type Keygroups = HashMap<String, Keygroup>;

#[derive(Debug, Serialize, Deserialize)]
pub struct Configuration
{
	pub profiles: HashMap<String, Profile>,
	pub themes: HashMap<String, Theme>,
	pub keygroups: Keygroups,
	pub gkey_sets: Option<HashMap<String, HashMap<u8, MacroKeyAssignment>>>,
	pub macros: Option<HashMap<String, crate::macros::Macro>>
}

impl<'a> Configuration
{
	pub fn config_file_location() -> &'static str
	{
		// TODO use xdg_config_dir for non-debug builds
		"config.yaml"
	}

	pub fn load() -> Result<Self, ConfigError>
	{
		std::fs::read_to_string(Self::config_file_location())
			.map_err(|e| ConfigError::UnableToOpen(e))
			.and_then(|yaml_string| serde_yaml::from_str(&yaml_string)
				.map_err(|e| ConfigError::ParseError(e)))
	}

	pub fn save(&self) -> Result<(), ConfigError>
	{
		serde_yaml::to_string(self)
			.map_err(|e| ConfigError::SerializeError(e))
			.and_then(|yaml_string| std::fs::write(Self::config_file_location(), yaml_string)
				.map_err(|e| ConfigError::UnableToWrite(e)))
	}

	pub fn profile_for_active_window(&self, window: &Option<ActiveWindowInfo>) -> String
	{
		match window
		{
			Some(window) => self.profiles
				.iter()
				.filter(|(name, _profile)| name.as_str() != "default")
				.find(|(_name, profile)| match profile.conditions
				{
					Some(ref conditions) => window.matches_conditions(conditions),
					None => false
				})
				.map(|(name, _profile)| name.clone())
				.unwrap_or("default".into()),
			None => "default".into()
		}
	}

	pub fn theme_scancode_assignments(&self, theme: &str) -> Option<ScancodeAssignments>
	{
		self.themes.get(theme)
			.and_then(|theme| theme.scancode_assignments(&self.keygroups))
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveWindowConditions
{
	#[serde(with = "RegexSerializer")]
	#[serde(default)]
	pub title: Option<Regex>,

	#[serde(with = "RegexSerializer")]
	#[serde(default)]
	pub executable: Option<Regex>,

	#[serde(with = "RegexSerializer")]
	#[serde(default)]
	pub class: Option<Regex>,

	#[serde(with = "RegexSerializer")]
	#[serde(default)]
	pub class_name: Option<Regex>
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
struct RegexWrapper<T>(T);

impl Serialize for RegexWrapper<&Regex>
{
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		serializer.serialize_str(self.0.as_str())
	}
}

impl Serialize for RegexWrapper<&Option<Regex>>
{
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		match self.0
		{
			Some(ref regex) => serializer.serialize_some(&RegexWrapper(regex)),
			None => serializer.serialize_none()
		}
	}
}

impl<'de> Deserialize<'de> for RegexWrapper<Regex>
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>
	{
		match <std::borrow::Cow<str>>::deserialize(deserializer)?.parse()
		{
			Ok(regex) => Ok(RegexWrapper(regex)),
			Err(e) => Err(D::Error::custom(e))
		}
	}
}

impl<'de> Deserialize<'de> for RegexWrapper<Option<Regex>>
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>
	{
		Ok(match Option::<RegexWrapper<Regex>>::deserialize(deserializer)?
		{
			Some(RegexWrapper(regex)) => RegexWrapper(Some(regex)),
			None => RegexWrapper(None),
		})
	}
}

struct RegexSerializer;

impl RegexSerializer
{
	pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
		for<'a> RegexWrapper<&'a T>: Serialize
	{
		RegexWrapper(value).serialize(serializer)
	}

	pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
	where
		D: Deserializer<'de>,
		RegexWrapper<T>: Deserialize<'de>
	{
		RegexWrapper::deserialize(deserializer).map(|wrapper| wrapper.0)
	}
}
