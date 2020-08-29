use std::time::Duration;
use std::env;

mod x11;
// TODO support wayland?

pub trait WindowSystem where Self: Send + Sync
{
	fn send_key_combo(&self, key_combo: &str, pressed: bool, delay: Duration);
	fn active_window_info(&self) -> Option<ActiveWindowInfo>;
}

impl dyn WindowSystem where Self: Send + Sync
{
	pub fn send_key_combo_press(&self, key_combo: &str)
	{
		let duration = Duration::from_millis(6);
		self.send_key_combo(key_combo, true, duration);
		self.send_key_combo(key_combo, false, duration);
	}
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ActiveWindowInfo
{
	pub pid: Option<i32>,
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

pub fn get_window_system() -> Result<Box<dyn WindowSystem>, WindowSystemError>
{
	if env::var("WAYLAND_DISPLAY").is_ok()
	{
		return Err(WindowSystemError::NotSupported)
	}

	if env::var("DISPLAY").is_ok()
	{
		return Ok(Box::new(x11::X11Interface::new()))
	}

	return Err(WindowSystemError::NotDetected)
}
