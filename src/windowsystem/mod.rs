use std::time::Duration;
use std::env;
use std::sync::mpsc::{Sender, Receiver, TryRecvError};
use std::fmt;

use serde::{Serialize, Deserialize};
use log::debug;

use crate::MainThreadSignal;
use crate::config::ActiveWindowConditions;

mod x11;
// TODO support wayland?

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton
{
	Left,
	Middle,
	Right
}

#[derive(Debug)]
pub enum WindowSystemError
{
	NotSupported,
	NotDetected
}

pub enum WindowSystemSignal
{
	Shutdown,
	SendClick(MouseButton),
	SendKeyCombo(String)
}

pub trait WindowSystem where Self: Send
{
	fn send_key_combo(&self, key_combo: &str, pressed: bool, delay: Duration);
	fn send_mouse_button(&self, button: MouseButton, pressed: bool);
	fn active_window_info(&self) -> Option<ActiveWindowInfo>;
}

impl dyn WindowSystem where Self: Send
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

	pub fn run(
		&self,
		rx: Receiver<WindowSystemSignal>,
		tx: Sender<MainThreadSignal>)
	{
		let mut last_active_window = None;

		// receiving anything should be interpreted as a shutdown event
		loop
		{
			match rx.try_recv()
			{
				Ok(WindowSystemSignal::Shutdown)
					| Err(TryRecvError::Disconnected) => break,

				Err(TryRecvError::Empty) => (),

				Ok(WindowSystemSignal::SendClick(button)) => self.send_mouse_click(button),
				Ok(WindowSystemSignal::SendKeyCombo(combo)) => self.send_key_combo_press(&combo)
			}

			let active_window = self.active_window_info();

			if last_active_window != active_window
			{
				debug!(
					"active window has changed: {:?} => {:?}",
					&last_active_window,
					&active_window);

				tx.send(MainThreadSignal::ActiveWindowChanged(active_window.clone()));
				last_active_window = active_window;
			}

			std::thread::sleep(Duration::from_millis(400));
		}
	}
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct ActiveWindowInfo
{
	pub title: Option<String>,
	pub executable: Option<String>,
	pub class: Option<String>,
	pub class_name: Option<String>
}

impl ActiveWindowInfo
{
	pub fn matches_conditions(&self, conditions: &ActiveWindowConditions) -> bool
	{
		if conditions.title.as_ref()
			.or_else(|| conditions.executable.as_ref())
			.or_else(|| conditions.class.as_ref())
			.or_else(|| conditions.class_name.as_ref())
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

impl fmt::Display for ActiveWindowInfo
{
	fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error>
	{
		write!(formatter,
			"[{}] {}",
			self.class.as_deref().unwrap_or("unknown class"),
			self.title.as_deref().unwrap_or("no title"))
	}
}
