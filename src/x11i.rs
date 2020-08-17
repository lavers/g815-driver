use std::convert::TryFrom;
use std::os::raw::{c_ulong, c_long, c_int, c_uchar, c_char, c_void};
use std::ffi::{CStr, CString};
use std::ptr;

use x11::xlib::{
	Display,
	Window,
	XOpenDisplay, 
	XInternAtom, 
	XDefaultRootWindow, 
	XGetWindowProperty, 
	XFree};

#[derive(Debug)]
pub enum GetWindowPropertyError
{
	BadWindow,
	BadAtom,
	BadValue
}

pub struct ActiveWindowInfo
{
	pub pid: Option<i32>,
	pub title: Option<String>,
	pub executable: Option<String>
}

pub struct X11Interface
{
	display: *mut Display
}

impl X11Interface
{
	pub fn new() -> Self
	{
		unsafe
		{
			X11Interface
			{
				display: XOpenDisplay(ptr::null())
			}
		}
	}

	pub fn get_active_window_info(&self) -> Option<ActiveWindowInfo>
	{
		self.get_active_window()
			.map(|window| 
			{
				let pid = self.get_window_pid(window).unwrap_or(None);

				ActiveWindowInfo
				{
					pid,
					title: self.get_window_name(window).unwrap_or(None),
					executable: pid
						.and_then(|pid| std::fs::read_link(format!("/proc/{}/exe", pid)).ok())
						.map(|exe_path| exe_path.to_string_lossy().into())
				}
			})
	}

	pub fn get_active_window(&self) -> Option<Window>
	{
		unsafe
		{
			let root_window = XDefaultRootWindow(self.display);

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

	pub unsafe fn get_window_property(&self, window: Window, property: &str) 
		-> Result<Option<*mut c_uchar>, GetWindowPropertyError>
	{
		let mut property_type = 0 as c_ulong;
		let mut format = 0 as c_int;
		let mut item_count = 0 as c_ulong;
		let mut bytes_after = 0 as c_ulong;
		let mut result_pointer = ptr::null_mut();

		let property = CString::new(property).unwrap();
		let property_atom = XInternAtom(self.display, property.as_ptr() as *const i8, 0);

		let status = XGetWindowProperty(
			self.display, 
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
}

impl Drop for X11Interface
{
	fn drop(&mut self)
	{
		unsafe
		{
			XFree(self.display as *mut c_void);
		}
	}
}
