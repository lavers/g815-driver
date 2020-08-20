#![feature(entry_insert)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver};
use std::io::{prelude::*, stdin, stdout};

use hidapi::HidApi;

mod x11i;
mod g815;

pub struct KeyboardState
{
	key_bitmasks: HashMap<KeyType, u8>,
	mode: u8,
	macro_recording: bool
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
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

impl KeyboardState
{
	pub fn new() -> Self
	{
		KeyboardState 
		{ 
			mode: 0, 
			key_bitmasks: HashMap::new(),
			macro_recording: false 
		}
	}
}

fn main()
{
	let x11i = x11i::X11Interface::new();
	let hidapi = HidApi::new().unwrap();
	let mut kb = g815::G815Keyboard::new(&hidapi).unwrap();

	println!("Firmware Version: {}\nBootloader Version: {}", 
			 &kb.firmware_version().unwrap(),
			 &kb.bootloader_version().unwrap());

	kb.load_capabilities();

	let gkey_capability = kb.capability_data(g815::Capability::GKeys);
	println!("GKey capability data: {:x?}", gkey_capability.unwrap());

	let mkey_capability = kb.capability_data(g815::Capability::ModeSwitching);
	println!("MKey capability data: {:x?}", mkey_capability.unwrap());

	let gmkey_capability = kb.capability_data(g815::Capability::GameMode);
	println!("game mode key capability data: {:x?}", gmkey_capability.unwrap());

	let mrkey_capability = kb.capability_data(g815::Capability::MacroRecording);
	println!("game mode key capability data: {:x?}", mrkey_capability.unwrap());

	let kb = Arc::new(kb);

	println!("now attempting to enter software mode");
	kb.take_control();
	println!("should now be in software mode. dropping to debug prompt. watching for interrupts.");

	let thread_kb = Arc::clone(&kb);
	let (tx, rx) = channel();
	let state = Arc::new(Mutex::new(KeyboardState::new()));
	let x11i = Arc::new(x11i);
	let thread_x11i = Arc::clone(&x11i);

	let child = std::thread::spawn(move || 
	{
		device_thread(thread_kb, state, thread_x11i, rx);
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

	let active_window = x11i.get_active_window_info().unwrap();

	println!(
		"Active Window:\n\tPID: {}\n\tTitle: {}\n\tExecutable: {}", 
		active_window.pid.unwrap_or(0),
		active_window.title.unwrap_or("unknown".to_string()),
		active_window.executable.unwrap_or("unknown".to_string()));

	if let Some(hint) = active_window.class_hint
	{
		println!("\tClass Hint Name: {}\n\tClass Hint Class: {}", hint.name, hint.class);
	}
}

fn device_thread(kb: Arc<g815::G815Keyboard>, state: Arc<Mutex<KeyboardState>>, x11i: Arc<x11i::X11Interface>, rx: Receiver<bool>)
{
	loop
	{
		let events = kb.poll_for_events(&state);
		let mut state = state.lock().unwrap();

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
				kb.set_macro_record_mode(match state.macro_recording 
				{
					true => g815::MacroRecordMode::Recording,
					false => g815::MacroRecordMode::Default
				});
			},
			DeviceEvent::KeyUp(KeyType::Mode, mode) => 
			{
				state.mode = *mode;
				kb.set_mode(state.mode);
			},
			DeviceEvent::MediaKeyDown(key) => x11i.send_key_press(match key
			{
				MediaKey::Mute => x11::keysym::XF86XK_AudioMute,
				MediaKey::PlayPause => x11::keysym::XF86XK_AudioPlay,
				MediaKey::Next => x11::keysym::XF86XK_AudioNext,
				MediaKey::Previous => x11::keysym::XF86XK_AudioPrev,
				MediaKey::VolumeUp => x11::keysym::XF86XK_AudioRaiseVolume,
				MediaKey::VolumeDown => x11::keysym::XF86XK_AudioLowerVolume
			}),
			_ => ()
		});

		if let Ok(message) = rx.try_recv()
		{
			break;
		}

		std::thread::sleep(std::time::Duration::from_millis(1));
	}
}
