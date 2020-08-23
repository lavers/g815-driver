#![feature(entry_insert)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::io::{prelude::*, stdin, stdout};
use std::thread;
use std::time::Duration;

use serde::{Serialize, Deserialize};

use hidapi::HidApi;

use crate::windowsystem::ActiveWindowInfo;

mod windowsystem;
mod config;
mod macros;
mod g815;

pub struct SharedState
{
	window_system: Box<dyn windowsystem::WindowSystem>,
	active_window: Option<ActiveWindowInfo>,
	keyboard_state: KeyboardState
}

pub struct KeyboardState
{
	key_bitmasks: HashMap<KeyType, u8>,
	mode: u8,
	macro_recording: bool
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, Deserialize, Serialize)]
pub enum KeyType
{
	GKey,
	Mode,
	GameMode,
	MacroRecord,
	Light,
	MediaControl
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum MediaKey
{
	PlayPause = 0x01,
	Previous = 0x02,
	Next = 0x08,
	VolumeUp = 0x10,
	VolumeDown = 0x20,
	Mute = 0x40
}

#[derive(Debug)]
pub enum DeviceEvent
{
	KeyDown(KeyType, u8),
	KeyUp(KeyType, u8),
	MediaKeyUp(MediaKey),
	MediaKeyDown(MediaKey),
	BrightnessLevelChanged(u8)
}

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub enum Capability
{
	GKeys = 0x8010, // usual id = 0x0a
	ModeSwitching = 0x8020, // usual id = 0x0b
	MacroRecording = 0x8030, // usual id = 0x0c
	BrightnessAdjustment = 0x8040, // usual id = 0x0d
	GameMode = 0x4522, // usual id = 0x08

	// not sure what this one is but it's id (0xf) often comes around setting up lighting
	SomethingLightingRelated = 0x8071 
}

#[derive(Debug)]
pub struct CapabilityData
{
	id: u8,
	key_type: Option<KeyType>,
	key_count: Option<u8>,
	raw: Option<Vec<u8>>
}

impl CapabilityData
{
	pub fn no_capability() -> Self
	{
		CapabilityData
		{
			id: 0,
			key_type: None,
			key_count: None,
			raw: None
		}
	}
}

fn main()
{
	let window_system = windowsystem::get_window_system().unwrap();

	let hidapi = HidApi::new().unwrap();
	let mut kb = g815::G815Keyboard::new(&hidapi).unwrap();

	println!("Firmware Version: {}\nBootloader Version: {}", 
			 &kb.firmware_version().unwrap(),
			 &kb.bootloader_version().unwrap());

	kb.load_capabilities();

	let gkey_capability = kb.capability_data(Capability::GKeys);
	println!("GKey capability data: {:x?}", gkey_capability.unwrap());

	let mkey_capability = kb.capability_data(Capability::ModeSwitching);
	println!("MKey capability data: {:x?}", mkey_capability.unwrap());

	let gmkey_capability = kb.capability_data(Capability::GameMode);
	println!("game mode key capability data: {:x?}", gmkey_capability.unwrap());

	let mrkey_capability = kb.capability_data(Capability::MacroRecording);
	println!("game mode key capability data: {:x?}", mrkey_capability.unwrap());

	let kb = Arc::new(kb);

	println!("now attempting to enter software mode");
	kb.take_control();
	println!("should now be in software mode. dropping to debug prompt. watching for interrupts.");

	let thread_kb = Arc::clone(&kb);
	let (tx, rx) = channel();
	let state = Arc::new(Mutex::new(SharedState
	{
		keyboard_state: KeyboardState 
		{
			key_bitmasks: HashMap::new(),
			mode: 1,
			macro_recording: false
		},
		active_window: None,
		window_system: window_system
	}));

	let thread_shared_state = Arc::clone(&state);

	let child = std::thread::spawn(move || 
	{
		device_thread(thread_kb, thread_shared_state, rx);
	});

	let mut command = String::new();

		print!("> press any to return to hw mode");
		stdout().flush();

		let read = stdin().lock().read_line(&mut command).unwrap();
		println!("read {}, dump: {:#?}", read, &command);

	tx.send(true);
	child.join();

	println!("done watching for interrupts");

	kb.release_control();
	println!("should now be back in hardware mode.");

	let active_window = state.lock().unwrap().window_system.active_window_info().unwrap();

	println!(
		"Active Window:\n\tPID: {}\n\tTitle: {}\n\tExecutable: {}\n\tClass: {}\n\tClass Name: {}", 
		active_window.pid.unwrap_or(0),
		active_window.title.unwrap_or("unknown".into()),
		active_window.executable.unwrap_or("unknown".into()),
		active_window.class.unwrap_or("unknown".into()),
		active_window.class_name.unwrap_or("unknown".into()));
}

enum MainThreadEvent
{
	ActiveWindowChanged
}

fn active_window_watcher_thread(
	state: Arc<Mutex<SharedState>>, 
	rx: Receiver<bool>, 
	tx: Sender<MainThreadEvent>)
{
	while rx.try_recv().is_err()
	{
		let mut state = state.lock().unwrap();
		let active_window = state.window_system.active_window_info();

		if state.active_window != active_window
		{
			state.active_window = active_window;
			tx.send(MainThreadEvent::ActiveWindowChanged);
		}

		thread::sleep(Duration::from_millis(1500));
	}
}

fn device_thread(kb: Arc<g815::G815Keyboard>, state: Arc<Mutex<SharedState>>, rx: Receiver<bool>)
{
	loop
	{
		let mut state = state.lock().unwrap();
		let events = kb.poll_for_events(&mut state.keyboard_state);

		events.iter().for_each(|event| match event
		{
			DeviceEvent::KeyDown(KeyType::GKey, number) =>  
			{
				println!("gkey {} is down", number)
			},
			DeviceEvent::KeyUp(KeyType::GKey, number) => 
			{
				println!("gkey {} is up", number)
			},
			DeviceEvent::BrightnessLevelChanged(brightness) => 
			{
				println!("brightness level was changed to {}%", brightness)
			},
			DeviceEvent::KeyUp(KeyType::MacroRecord, _) => 
			{
				state.keyboard_state.macro_recording = !state.keyboard_state.macro_recording;
				kb.set_macro_record_mode(match state.keyboard_state.macro_recording 
				{
					true => g815::MacroRecordMode::Recording,
					false => g815::MacroRecordMode::Default
				});
			},
			DeviceEvent::KeyUp(KeyType::Mode, mode) => 
			{
				state.keyboard_state.mode = *mode;
				kb.set_mode(state.keyboard_state.mode);
			},
			DeviceEvent::MediaKeyDown(key) => state.window_system.send_key_combo_press(match key
			{
				MediaKey::Mute => "XF86AudioMute",
				MediaKey::PlayPause => "XF86AudioPlay",
				MediaKey::Next => "XF86AudioNext",
				MediaKey::Previous => "XF86AudioPrev",
				MediaKey::VolumeUp => "XF86AudioRaiseVolume",
				MediaKey::VolumeDown => "XF86AudioLowerVolume"
			}),
			_ => ()
		});

		if let Ok(message) = rx.try_recv()
		{
			break;
		}

		std::thread::sleep(std::time::Duration::from_millis(5));
	}
}
