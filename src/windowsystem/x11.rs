use std::convert::TryFrom;
use std::os::raw::{c_long, c_ulong, c_int, c_uint, c_char, c_uchar, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use std::time::Duration;
use std::sync::Mutex;

use x11::{xlib, xtest};
use x11::xlib::{Display, Window, KeyCode, XFree};

use crate::windowsystem::{ActiveWindowInfo, WindowSystem, MouseButton};

#[derive(Debug)]
pub enum GetWindowPropertyError
{
	BadWindow,
	BadAtom,
	BadValue,
	UnknownError
}

pub struct WindowClassHint
{
	pub name: String,
	pub class: String
}

pub struct X11Interface
{
	display: Mutex<*mut Display>,
	min_keycode: KeyCode,
	max_keycode: KeyCode
}

unsafe impl Send for X11Interface {}
unsafe impl Sync for X11Interface {}

impl X11Interface
{
	pub fn new() -> Self
	{
		unsafe
		{
			let display = xlib::XOpenDisplay(ptr::null());

			let mut min_keycode = 0;
			let mut max_keycode = 0;
			xlib::XDisplayKeycodes(display, &mut min_keycode, &mut max_keycode);
			
			X11Interface
			{
				display: Mutex::new(display),
				// the X11 spec says these are never outside 8..255 so this
				// cast should be fine
				min_keycode: min_keycode as KeyCode,
				max_keycode: max_keycode as KeyCode
			}
		}
	}

	pub fn get_active_window(&self) -> Option<Window>
	{
		unsafe
		{
			let root_window = xlib::XDefaultRootWindow(*self.display.lock().unwrap());

			self.get_window_property(root_window, "_NET_ACTIVE_WINDOW")
				.ok()
				.and_then(|property| property.map(|data| 
				{
					let window_id = u64::try_from(*(data as *mut c_long) as c_long).unwrap();
					XFree(data as *mut c_void);
					window_id
				}))
		}
	}

	pub fn get_window_pid(&self, window: Window) -> Result<Option<i32>, GetWindowPropertyError>
	{
		unsafe
		{
			self.get_window_property(window, "_NET_WM_PID")
				.map(|property| property.map(|data| 
				{
					let pid = i32::try_from(*(data as *mut c_long) as c_long).unwrap();
					XFree(data as *mut c_void);
					pid
				}))
		}
	}

	pub fn get_window_name(&self, window: Window) -> Result<Option<String>, GetWindowPropertyError>
	{
		unsafe
		{
			self.get_window_property(window, "_NET_WM_NAME")
				.or_else(|_| self.get_window_property(window, "WM_NAME"))
				.map(|property| property.map(|data|
				{
					let window_name = CStr::from_ptr(data as *mut c_char)
						.to_string_lossy()
						.into();
					XFree(data as *mut c_void);
					window_name
				}))
		}
	}

	pub fn get_window_class_hint(&self, window: Window) 
		-> Result<WindowClassHint, GetWindowPropertyError>
	{
		unsafe
		{
			let display = *self.display.lock().unwrap();
			let class_hint = xlib::XAllocClassHint();
			let status = xlib::XGetClassHint(display, window, class_hint);

			if status == 0 || status == xlib::BadWindow as i32
			{
				XFree(class_hint as *mut c_void);

				Err(match status
				{
				   0 => GetWindowPropertyError::UnknownError,
				   _ => GetWindowPropertyError::BadWindow
				})
			}
			else
			{
				let hint = WindowClassHint
				{
					name: CStr::from_ptr((*class_hint).res_name).to_string_lossy().into(),
					class: CStr::from_ptr((*class_hint).res_class).to_string_lossy().into()
				};

				XFree((*class_hint).res_name as *mut c_void);
				XFree((*class_hint).res_class as *mut c_void);
				XFree(class_hint as *mut c_void);

				Ok(hint)
			}
		}
	}

	unsafe fn get_window_property(&self, window: Window, property: &str) 
		-> Result<Option<*mut c_uchar>, GetWindowPropertyError>
	{
		let display = *self.display.lock().unwrap();

		let mut property_type = 0 as c_ulong;
		let mut format = 0 as c_int;
		let mut item_count = 0 as c_ulong;
		let mut bytes_after = 0 as c_ulong;
		let mut result_pointer = ptr::null_mut();

		let property = CString::new(property).unwrap();
		let property_atom = xlib::XInternAtom(display, property.as_ptr() as *const i8, 0);

		let status = xlib::XGetWindowProperty(
			display, 
			window, 
			property_atom, 
			0, // offset
			!(0 as c_long), // length
			0, // delete
			x11::xlib::AnyPropertyType as c_ulong, 
			&mut property_type,
			&mut format,
			&mut item_count,
			&mut bytes_after,
			&mut result_pointer);

		match status as u8
		{
			x11::xlib::BadWindow => Err(GetWindowPropertyError::BadWindow),
			x11::xlib::BadValue => Err(GetWindowPropertyError::BadValue),
			x11::xlib::BadAtom => Err(GetWindowPropertyError::BadAtom),
			x11::xlib::Success => Ok(match item_count 
			{
				0 => None,
				_ => Some(result_pointer)
			}),
			status => panic!("status from XGetWindowProperty unknown: {}", status)
		}
	}

	fn find_unused_keycode(&self) -> Option<KeyCode>
	{
		unsafe
		{
			let mut symbols_per_keycode = 0;

			let keysyms = xlib::XGetKeyboardMapping(
				*self.display.lock().unwrap(), 
				self.min_keycode, 
				(self.max_keycode - self.min_keycode) as i32,
				&mut symbols_per_keycode);

			let free_keycode = (self.min_keycode..self.max_keycode)
				.find(|keycode| 
				{
					let offset = (keycode - self.min_keycode) as i32 * symbols_per_keycode;

					(0..symbols_per_keycode)
						.all(|i| *keysyms.offset((offset + i) as isize) == 0)
				});

			XFree(keysyms as *mut c_void);

			free_keycode
		}
	}

	fn key_name_to_symbol(&self, key: &str) -> Option<c_uint>
	{
		let key = match key
		{
			"alt" => "Alt_L",
			"ctrl" => "Control_L",
			"meta" => "Meta_L",
			"super" => "Super_L",
			"win" => "Super_L",
			"shift" => "Shift_L",
			key => key
		};

		let key_string = CString::new(key).unwrap();

		unsafe
		{
			// obviously don't actually need the display for this call however
			// we'll aquire the mutex just to be sure we're not going to break
			// anything as libx11 isn't thread safe
			let _aquire_mutex = self.display.lock().unwrap();
			let symbol = xlib::XStringToKeysym(key_string.as_ptr());

			match symbol != x11::xlib::NoSymbol as c_ulong
			{
				true => Some(symbol as c_uint),
				false => None
			}
		}
	}

	fn key_combo_to_keysym_sequence(&self, combo: &str) -> Option<Vec<c_uint>>
	{
		combo
			.split("+")
			.map(|key_string| self.key_name_to_symbol(key_string))
			.collect()
	}

	/// Simulates the pressing of a given set of KeySym's.
	///
	/// Ideally this would take a slice of &[KeySym] however
	/// all of the KeySym constants are defined as c_uint
	/// but KeySym is defined as c_ulong for some reason
	fn send_keysym_sequence(&self, sequence: &[c_uint], pressed: bool, delay: Duration)
	{
		unsafe
		{
			let mut temporary_keycode = None;
			let display = *self.display.lock().unwrap();

			for symbol in sequence
			{
				let mut keycode = xlib::XKeysymToKeycode(display, (*symbol) as u64);

				if keycode == 0
				{
					keycode = *temporary_keycode
						.get_or_insert(self.find_unused_keycode().unwrap());

					let mut symbol = *symbol as u64;
					xlib::XChangeKeyboardMapping(display, keycode as i32, 1, &mut symbol, 1);
					xlib::XSync(display, 0);
				}

				xtest::XTestFakeKeyEvent(display, keycode as u32, pressed as i32, xlib::CurrentTime);
				xlib::XSync(display, xlib::False);
				xlib::XFlush(display);

				if delay.as_micros() > 0
				{
					std::thread::sleep(delay);
				}
			}

			if let Some(temporary_keycode) = temporary_keycode
			{
				let mut symbol = 0;
				xlib::XChangeKeyboardMapping(display, temporary_keycode as i32, 1, &mut symbol, 1);
				xlib::XFlush(display);
			}
		}
	}
}

impl Drop for X11Interface
{
	fn drop(&mut self)
	{
		unsafe
		{
			XFree(*self.display.lock().unwrap() as *mut c_void);
		}
	}
}

impl WindowSystem for X11Interface 
{
	fn active_window_info(&self) -> Option<ActiveWindowInfo>
	{
		self.get_active_window().map(|window| 
		{
			let pid = self.get_window_pid(window).unwrap_or(None);
			let class_hint = self.get_window_class_hint(window).ok();

			ActiveWindowInfo
			{
				title: self.get_window_name(window).unwrap_or(None),
				executable: pid
					.and_then(|pid| std::fs::read_link(format!("/proc/{}/exe", pid)).ok())
					.map(|exe_path| exe_path.to_string_lossy().into()),
				class: class_hint.as_ref().map(|hint| hint.class.clone()),
				class_name: class_hint.as_ref().map(|hint| hint.name.clone())
			}
		})
	}

	fn send_mouse_button(&self, button: MouseButton, pressed: bool)
	{
		unsafe
		{
			let display = *self.display.lock().unwrap();

			xtest::XTestFakeButtonEvent(
				display, 
				match button 
				{
					MouseButton::Left => xlib::Button1,
					MouseButton::Middle => xlib::Button2,
					MouseButton::Right => xlib::Button3
				}, 
				pressed as c_int, 
				xlib::CurrentTime);
		}
	}

	fn send_key_combo(&self, key_combo: &str, pressed: bool, delay: Duration)
	{
		self.key_combo_to_keysym_sequence(key_combo)
			.map(|sequence| self.send_keysym_sequence(&sequence, pressed, delay));
	}
}
