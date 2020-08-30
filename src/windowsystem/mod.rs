use std::time::Duration;
use std::env;
use std::sync::{Arc, RwLock};
use std::sync::mpsc::{Sender, Receiver};

use serde::{Serialize, Deserialize};

use crate::{SharedState, MainThreadEvent};
use crate::config::ActiveWindowConditions;

mod x11;
// TODO support wayland?

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum MouseButton
{
	#[serde(rename = "left")]
	Left,
	#[serde(rename = "middle")]
	Middle,
	#[serde(rename = "right")]
	Right
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct ActiveWindowInfo
{
	pub title: Option<String>,
	pub executable: Option<String>,
	pub class: Option<String>,
	pub class_name: Option<String>
}

#[derive(Debug)]
pub enum WindowSystemError
{
	NotSupported,
	NotDetected
}

pub trait WindowSystem where Self: Send + Sync
{
	fn send_key_combo(&self, key_combo: &str, pressed: bool, delay: Duration);
	fn send_mouse_button(&self, button: MouseButton, pressed: bool);
	fn active_window_info(&self) -> Option<ActiveWindowInfo>;
}

impl dyn WindowSystem where Self: Send + Sync
{
	pub fn new() -> Result<Box<dyn WindowSystem>, WindowSystemError>
	{
		if env::var("WAYLAND_DISPLAY").is_ok()
		{
			Err(WindowSystemError::NotSupported)
		}
		else if env::var("DISPLAY").is_ok()
		{
			Ok(Box::new(x11::X11Interface::new()))
		}
		else
		{
			Err(WindowSystemError::NotDetected)
		}
	}

	pub fn send_key_combo_press(&self, key_combo: &str)
	{
		let duration = Duration::from_millis(6);
		self.send_key_combo(key_combo, true, duration);
		self.send_key_combo(key_combo, false, duration);
	}

	pub fn send_mouse_click(&self, button: MouseButton)
	{
		self.send_mouse_button(button, true);
		self.send_mouse_button(button, false);
	}

	pub fn active_window_watcher_thread(
		state: Arc<RwLock<SharedState>>, 
		rx: Receiver<()>, 
		tx: Sender<MainThreadEvent>)
	{
		let mut last_active_window = None;

		// receiving anything should be interpreted as a shutdown event
		while rx.try_recv().is_err()
		{
			let active_window = {
				// make sure state is not locked for any longer
				// than it needs to use the window_system to prevent locks
				let state = state.read().unwrap();
				state.window_system.active_window_info()
			};

			if last_active_window != active_window
			{
				tx.send(MainThreadEvent::ActiveWindowChanged(active_window.clone()));
				last_active_window = active_window;
			}

			std::thread::sleep(Duration::from_millis(400));
		}
	}
}

impl ActiveWindowInfo
{
	pub fn matches_conditions(&self, conditions: &ActiveWindowConditions) -> bool
	{
		if conditions.title.as_ref()
			.or(conditions.executable.as_ref())
			.or(conditions.class.as_ref())
			.or(conditions.class_name.as_ref())
			.is_none()
		{
			return false
		}

		let mut matches = true;

		if let Some(ref regex) = conditions.title
		{
			matches = matches && self.title
				.as_ref()
				.map(|title| regex.is_match(title))
				.unwrap_or(false)
		}

		if let Some(ref regex) = conditions.executable
		{
			matches = matches && self.executable
				.as_ref()
				.map(|executable| regex.is_match(executable))
				.unwrap_or(false)
		}

		if let Some(ref regex) = conditions.class
		{
			matches = matches && self.class
				.as_ref()
				.map(|class| regex.is_match(class))
				.unwrap_or(false)
		}

		if let Some(ref regex) = conditions.class_name
		{
			matches = matches && self.class_name
				.as_ref()
				.map(|class_name| regex.is_match(class_name))
				.unwrap_or(false)
		}

		matches
	}
}
