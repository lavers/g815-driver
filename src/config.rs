use std::collections::HashMap;

use serde::{Serialize, Deserialize, Serializer, Deserializer, de::Error};

use regex::Regex;

use crate::windowsystem::ActiveWindowInfo;
use crate::device::scancode::Scancode;
use crate::device::rgb::{Theme, ScancodeAssignments};
use crate::macros::Macro;

#[derive(Debug)]
pub enum ConfigError
{
	UnableToOpen(std::io::Error),
	UnableToWrite(std::io::Error),
	ParseError(serde_yaml::Error),
	SerializeError(serde_yaml::Error),
	InvalidConfiguration(String)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MacroKeyAssignment
{
	#[serde(rename = "simple_action")]
	SimpleAction(crate::macros::Action),
	#[serde(rename = "run_macro")]
	NamedMacro(String)
}

impl MacroKeyAssignment
{
	pub fn expand(&self, config: &Configuration) -> Option<Macro>
	{
		match self
		{
			Self::SimpleAction(action) => Some(Macro::from_action(action.clone())),
			Self::NamedMacro(macro_name) => config.macros
				.as_ref()
				.and_then(|macros| macros.get(macro_name))
				.cloned()
		}
	}
}

pub type Keygroup = Vec<Scancode>;
pub type Keygroups = HashMap<String, Keygroup>;

pub type GkeyAssignments = Option<HashMap<u8, MacroKeyAssignment>>;
pub type GkeySets = Option<Vec<String>>;

#[derive(Serialize, Deserialize, Debug)]
pub struct ModeProfile
{
	theme: Option<String>,
	gkey_sets: GkeySets,
	gkeys: GkeyAssignments
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Profile
{
	conditions: Option<ActiveWindowConditions>,
	pub theme: Option<String>,
	gkey_sets: GkeySets,
	gkeys: GkeyAssignments,
	modes: Option<HashMap<u8, ModeProfile>>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Configuration
{
	pub profiles: HashMap<String, Profile>,
	pub themes: HashMap<String, Theme>,
	pub keygroups: Keygroups,
	pub gkey_sets: Option<HashMap<String, HashMap<u8, MacroKeyAssignment>>>,
	pub macros: Option<HashMap<String, Macro>>
}

trait ProfileKeyAssignment
{
	fn gkey_sets(&self) -> &GkeySets;
	fn gkeys(&self) -> &GkeyAssignments;

	fn gkey_assignment<'a>(&'a self, config: &'a Configuration, key: u8) -> Option<&'a MacroKeyAssignment>
	{
		self.gkeys()
			.as_ref()
			.and_then(|gkey_assignments| gkey_assignments.get(&key))
			.or_else(|| self.gkey_set_assignment(config, key))
	}

	fn gkey_set_assignment<'a>(&'a self, config: &'a Configuration, key: u8) -> Option<&'a MacroKeyAssignment>
	{
		self.gkey_sets().as_ref().and_then(|gkey_sets| 
		{
			for gkey_set_name in gkey_sets.iter().rev()
			{
				if let Some(assignment) = config.gkey_set_assignment(gkey_set_name, key)
				{
					return Some(assignment)
				}
			}

			None
		})
	}
}

impl ProfileKeyAssignment for Profile
{
	fn gkey_sets(&self) -> &GkeySets
	{
		&self.gkey_sets
	}

	fn gkeys(&self) -> &GkeyAssignments
	{
		&self.gkeys
	}
}

impl ProfileKeyAssignment for ModeProfile
{
	fn gkey_sets(&self) -> &GkeySets
	{
		&self.gkey_sets
	}

	fn gkeys(&self) -> &GkeyAssignments
	{
		&self.gkeys
	}
}

impl Configuration
{
	pub fn config_file_location() -> &'static str
	{
		// TODO use xdg_config_dir for non-debug builds
		"config.yaml"
	}

	pub fn load() -> Result<Self, ConfigError>
	{
		std::fs::read_to_string(Self::config_file_location())
			.map_err(ConfigError::UnableToOpen)
			.and_then(|yaml_string| serde_yaml::from_str(&yaml_string)
				.map_err(ConfigError::ParseError))
	}

	pub fn save(&self) -> Result<(), ConfigError>
	{
		serde_yaml::to_string(self)
			.map_err(ConfigError::SerializeError)
			.and_then(|yaml_string| std::fs::write(Self::config_file_location(), yaml_string)
				.map_err(ConfigError::UnableToWrite))
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
				.unwrap_or_else(|| "default".into()),
			None => "default".into()
		}
	}

	pub fn theme_scancode_assignments(&self, theme: &str) -> Option<ScancodeAssignments>
	{
		self.themes.get(theme)
			.and_then(|theme| theme.scancode_assignments(&self.keygroups))
	}

	pub fn macro_for_gkey(&self, current_profile: &str, mode: u8, gkey: u8) -> Option<Macro>
	{
		self.profiles
			.get(current_profile)
			.and_then(|profile| 
			{
				profile.modes
					.as_ref()
					.and_then(|modes| modes
						.get(&mode)
						.and_then(|mode_profile| mode_profile
							.gkey_assignment(self, gkey)))
					.or_else(|| profile.gkey_assignment(self, gkey))
					.and_then(|assignment| assignment.expand(self))
			})
	}

	pub fn gkey_set_assignment(&self, gkey_set: &str, key: u8) -> Option<&MacroKeyAssignment>
	{
		self.gkey_sets
			.as_ref()
			.and_then(|gkey_sets| gkey_sets
				.get(gkey_set)
				.and_then(|gkey_set| gkey_set.get(&key)))
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
