#![feature(entry_insert)]
#![allow(unused_must_use)]
#![recursion_limit="512"]

use std::sync::{Arc, RwLock};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::io::{prelude::*, stdin, stdout};
use std::thread;
use std::time::Duration;

use serde::{Serialize, Deserialize};

use hidapi::HidApi;

use threadpool::ThreadPool;

use crate::windowsystem::{WindowSystem, ActiveWindowInfo};

mod windowsystem;
mod scancode;
mod config;
mod macros;
mod g815;

pub struct SharedState
{
	window_system: Box<dyn WindowSystem>,
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

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
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
	let hidapi = HidApi::new().unwrap();
	let window_system = windowsystem::get_window_system().unwrap();
	let mut kb = g815::G815Keyboard::new(&hidapi).unwrap();

	let pool = ThreadPool::new(10);

	kb.load_capabilities();
	kb.take_control();

	let kb = Arc::new(RwLock::new(kb));
	let state = Arc::new(RwLock::new(SharedState
	{
		window_system: window_system,
		macro_recording: false
	}));

	let (main_thread_tx, main_thread_rx) = channel();
	let (device_thread_tx, device_thread_rx) = channel();
	let (ww_thread_tx, ww_thread_rx) = channel();

	let window_watcher_thread = 
	{
		let state = Arc::clone(&state);
		let main_thread_tx = main_thread_tx.clone();

		thread::spawn(move || active_window_watcher_thread(state, ww_thread_rx, main_thread_tx))
	};

	scancode::Scancode::iter_variants()
		.for_each(|scancode|
		{
			let mut data = Vec::new();
			data.push((scancode.to_rgb_id(), g815::Color::new(0, 0, 255)));
			let kb = kb.read().unwrap();
			println!("scancode: {:#?}", scancode);
			kb.set_4(&data);
			kb.commit();
			thread::sleep(Duration::from_millis(500));
		});

	let device_thread = 
	{
		let state = Arc::clone(&state);
		let device = Arc::clone(&kb);
		//let main_thread_tx = main_thread_tx.clone();

		thread::spawn(move || device_thread(device, state, device_thread_rx))
	};

	let mut command = String::new();
	print!("> press any to return to hw mode");
	stdout().flush();
	let read = stdin().lock().read_line(&mut command).unwrap();
	println!("read {}, dump: {:#?}", read, &command);

	device_thread_tx.send(());
	device_thread.join();
	println!("device thread exited");

	ww_thread_tx.send(());
	window_watcher_thread.join();
	println!("wnidow watcher thread exited");

	kb.read().unwrap().release_control();
}

enum MainThreadEvent
{
	ActiveWindowChanged(Option<ActiveWindowInfo>)
}

fn active_window_watcher_thread(
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

		thread::sleep(Duration::from_millis(1500));
	}
}

fn device_thread(kb: Arc<RwLock<g815::G815Keyboard>>, state: Arc<RwLock<SharedState>>, rx: Receiver<()>)
{
	while rx.try_recv().is_err()
	{
		let mut device = kb.write().unwrap();
		let mut state = state.write().unwrap();
		let events = device.poll_for_events();

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
				state.macro_recording = !state.macro_recording;
				device.set_macro_recording(state.macro_recording);
			},
			DeviceEvent::KeyUp(KeyType::Mode, mode) => 
			{
				device.set_mode(*mode);
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

		std::thread::sleep(std::time::Duration::from_millis(5));
	}
}
