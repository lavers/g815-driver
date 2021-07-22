use std::collections::HashMap;
use std::path::PathBuf;
use std::borrow::Cow;
use std::fmt;

use serde::{Serialize, Deserialize, Serializer, Deserializer, de::Error};

use regex::Regex;

use crate::windowsystem::ActiveWindowInfo;
use crate::device::scancode::Scancode;
use crate::device::rgb::Theme;
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

impl fmt::Display for ConfigError
{
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
	{
		match self
		{
			ConfigError::UnableToOpen(io_error) =>
				write!(f, "unable to read the config file: {}", io_error),
			ConfigError::UnableToWrite(io_error) =>
				write!(f, "unable to write the config file: {}", io_error),
			ConfigError::ParseError(serde_error) =>
				write!(f, "your configuration file cannot be parsed: {}", serde_error),
			ConfigError::SerializeError(serde_error) =>
				write!(f, "your configuration could not be serialized: {}", serde_error),
			ConfigError::InvalidConfiguration(reason) =>
				write!(f, "your configuration is invalid: {}", reason)
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MacroKeyAssignment
{
	SimpleAction(crate::macros::Action),
	NamedMacro(String)
}

impl MacroKeyAssignment
{
	pub fn expand<'a>(&'a self, config: &'a Configuration) -> Option<Cow<'a, Macro>>
	{
		match self
		{
			Self::SimpleAction(action) => Some(Cow::Owned(Macro::from_action(action.clone()))),
			Self::NamedMacro(macro_name) => config.macros
				.as_ref()
				.and_then(|macros| macros.get(macro_name))
				.map(|_macro| Cow::Borrowed(_macro))
		}
	}
}

pub type Keygroup = Vec<Scancode>;
pub type Keygroups = HashMap<String, Keygroup>;

pub type GkeyAssignments = Option<HashMap<u8, MacroKeyAssignment>>;
pub type GkeySets = Option<Vec<String>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeProfile
{
	theme: Option<String>,
	gkey_sets: GkeySets,
	gkeys: GkeyAssignments
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile
{
	conditions: Option<ActiveWindowConditions>,
	theme: Option<String>,
	gkey_sets: GkeySets,
	gkeys: GkeyAssignments,
	pub game_mode_keys: Option<Vec<Scancode>>,
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
	pub const fn config_filename() -> &'static str
	{
		"config.yml"
	}

	pub fn config_folder() -> PathBuf
	{
		let mut config_home = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| "".to_string());

		if config_home.is_empty()
		{
			let home = std::env::var("HOME").expect("$XDG_CONFIG_HOME is not defined, and \
				neither is $HOME. No idea where to find your config.yml file!");
			config_home = format!("{}/.config", home);
		}

		let mut path = PathBuf::from(config_home);
		path.push("g815d");
		std::fs::canonicalize(path)
			.expect("unable to convert config file location to an absolute path")
	}

	pub fn file_path() -> PathBuf
	{
		let mut path = Self::config_folder();
		path.push(Self::config_filename());
		path
	}

	pub fn load() -> Result<Self, ConfigError>
	{
		std::fs::read_to_string(Self::file_path())
			.map_err(ConfigError::UnableToOpen)
			.and_then(|yaml_string| serde_yaml::from_str(&yaml_string)
				.map_err(ConfigError::ParseError))
			.and_then(|config: Configuration| match config.profiles.contains_key("default")
			{
				true => Ok(config),
				false => Err(ConfigError::InvalidConfiguration("there is no default profile".into()))
			})
			.and_then(|config: Configuration| match config.themes.contains_key("default")
			{
				true => Ok(config),
				false => Err(ConfigError::InvalidConfiguration("there is no default theme".into()))
			})
	}

	pub fn save(&self) -> Result<(), ConfigError>
	{
		serde_yaml::to_string(self)
			.map_err(ConfigError::SerializeError)
			.and_then(|yaml_string| std::fs::write(Self::file_path(), yaml_string)
				.map_err(ConfigError::UnableToWrite))
	}

	pub fn default_profile(&self) -> &Profile
	{
		self.profiles.get("default").unwrap()
	}

	pub fn default_theme(&self) -> &Theme
	{
		self.themes.get("default").unwrap()
	}

	pub fn profile_for_active_window(&self, window: &Option<ActiveWindowInfo>) -> (&str, &Profile)
	{
		window
			.as_ref()
			.and_then(|window| self.profiles
				.iter()
				.filter(|(name, _profile)| name.as_str() != "default")
				.find_map(|(name, profile)| profile.conditions
					.as_ref()
					.and_then(|conditions| window
						.matches_conditions(conditions)
						.then_some((name.as_str(), profile)))))
			.unwrap_or_else(|| ("default", self.default_profile()))
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

impl Profile
{
	pub fn theme<'a>(&'a self, config: &'a Configuration, mode: u8) -> &'a Theme
	{
		self.modes
			.as_ref()
			.and_then(|modes| modes
				.get(&mode)
				.and_then(|mode_profile| mode_profile.theme.as_ref()))
			.or_else(|| self.theme.as_ref())
			.and_then(|theme_name| config.themes.get(theme_name))
			.unwrap_or_else(|| config.default_theme())
	}

	pub fn macro_for_gkey<'a>(&'a self, config: &'a Configuration, mode: u8, gkey: u8)
		-> Option<Cow<'a, Macro>>
	{
		self.modes
			.as_ref()
			.and_then(|modes| modes
				.get(&mode)
				.and_then(|mode_profile| mode_profile.gkey_assignment(config, gkey)))
			.or_else(|| self.gkey_assignment(config, gkey))
			.and_then(|assignment| assignment.expand(config))
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
