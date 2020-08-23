use std::collections::HashMap;

use serde::{Serialize, Deserialize};

use crate::{KeyType, macros::Macro};

#[derive(Serialize, Deserialize)]
struct KeyBinding
{

}

#[derive(Serialize, Deserialize)]
struct WindowFilter
{

}

#[derive(Serialize, Deserialize)]
enum Effect
{

}

#[derive(Serialize, Deserialize)]
enum Theme
{
	Static(HashMap<String, u32>),
	Effect(Effect)
}

#[derive(Serialize, Deserialize)]
struct Configuration
{
	// map of binding name -> key type -> key number -> KeyBinding
	bindings: HashMap<String, HashMap<KeyType, HashMap<u8, KeyBinding>>>,
	// map of binding name -> list of window filters
	assignments: HashMap<String, Vec<WindowFilter>>,
	macros: HashMap<String, Macro>,
	themes: HashMap<String, Theme>
}

