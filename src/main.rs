#![feature(entry_insert)]
#![allow(unused_must_use)]
#![recursion_limit="512"]

use std::sync::{Arc, RwLock};
use std::sync::mpsc::{channel, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use hidapi::HidApi;

use threadpool::ThreadPool;

use config::Configuration;
use windowsystem::{WindowSystem, ActiveWindowInfo};
use device::g815;
use device::scancode::Scancode;
use device::rgb::{Color, ScancodeAssignments};
use device::{DeviceEvent, KeyType, MediaKey};

mod windowsystem;
mod device;
mod config;
mod macros;

pub struct SharedState
{
	window_system: Box<dyn WindowSystem>,
	config: Configuration,
	macro_recording: bool
}

fn main()
{
	let config = Configuration::load().unwrap();
	let hidapi = HidApi::new().unwrap();
	let pool = ThreadPool::new(10);
	let window_system = WindowSystem::new().unwrap();
	let mut kb = g815::G815Keyboard::new(&hidapi).unwrap();

	kb.load_capabilities();
	kb.take_control();

	let kb = Arc::new(RwLock::new(kb));
	let state = Arc::new(RwLock::new(SharedState
	{
		window_system: window_system,
		macro_recording: false,
		config
	}));

	let (main_thread_tx, main_thread_rx) = channel();
	let (device_thread_tx, device_thread_rx) = channel();
	let (ww_thread_tx, ww_thread_rx) = channel();
	let should_exit = Arc::new(AtomicBool::new(false));

	{
		let state = Arc::clone(&state);
		let main_thread_tx = main_thread_tx.clone();
		pool.execute(move || WindowSystem::active_window_watcher_thread(state, ww_thread_rx, main_thread_tx))
	}
	{
		let should_exit = should_exit.clone();
		ctrlc::set_handler(move || 
		{
			println!("got ctrl-c");
			should_exit.store(true, Ordering::Relaxed);
		});
	}
	{
		let state = Arc::clone(&state);
		let device = Arc::clone(&kb);
		//let main_thread_tx = main_thread_tx.clone();

		pool.execute(move || device_thread(device, state, device_thread_rx))
	}

	println!("> now in main event loop, send ctrl-c to shutdown");

	while !should_exit.load(Ordering::Relaxed)
	{
		std::thread::sleep(Duration::from_millis(10));

		if let Ok(message) = main_thread_rx.try_recv()
		{
			match message
			{
				MainThreadEvent::ActiveWindowChanged(active_window) => 
				{
					let config = &state.read().unwrap().config;
					let profile_name = config.profile_for_active_window(&active_window);

					println!("active window has changed, new profile: {}\n{:#?}", &profile_name, &active_window);

					if let Some(profile) = config.profiles.get(&profile_name)
					{
						if let Some(ref theme_name) = profile.theme
						{
							if let Some(scancode_assignments) = config.theme_scancode_assignments(&theme_name)
							{
								device_thread_tx.send(DeviceThreadSignal::SetScancodes(scancode_assignments));
							}
						}
					}
				}
			}
		}
	}

	println!("notifying threads of shutdown");

	device_thread_tx.send(DeviceThreadSignal::Shutdown);
	ww_thread_tx.send(());

	pool.join();
	println!("threadpool shutdown");

	kb.read().unwrap().release_control();
}

pub enum MainThreadEvent
{
	ActiveWindowChanged(Option<ActiveWindowInfo>)
}

enum DeviceThreadSignal
{
	Shutdown,
	SetScancodes(ScancodeAssignments)
}

fn device_thread(
	kb: Arc<RwLock<g815::G815Keyboard>>, 
	state: Arc<RwLock<SharedState>>, 
	rx: Receiver<DeviceThreadSignal>)
{
	loop
	{
		let mut device = kb.write().unwrap();
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
				let mut state = state.write().unwrap();
				state.macro_recording = !state.macro_recording;
				device.set_macro_recording(state.macro_recording);
			},
			DeviceEvent::KeyUp(KeyType::Mode, mode) => 
			{
				device.set_mode(*mode);
			},
			DeviceEvent::MediaKeyDown(key) => 
			{
				state.read().unwrap().window_system.send_key_combo_press(match key
				{
					MediaKey::Mute => "XF86AudioMute",
					MediaKey::PlayPause => "XF86AudioPlay",
					MediaKey::Next => "XF86AudioNext",
					MediaKey::Previous => "XF86AudioPrev",
					MediaKey::VolumeUp => "XF86AudioRaiseVolume",
					MediaKey::VolumeDown => "XF86AudioLowerVolume"
				})
			},
			_ => ()
		});

		if let Ok(signal) = rx.try_recv()
		{
			match signal
			{
				DeviceThreadSignal::Shutdown => break,
				DeviceThreadSignal::SetScancodes(scancodes) => device.set_scancodes(scancodes)
			}
		}

		std::thread::sleep(std::time::Duration::from_millis(5));
	}
}

fn test_all_scancodes(kb: &Arc<RwLock<g815::G815Keyboard>>)
{
	Scancode::iter_variants()
		.for_each(|scancode|
		{
			let mut data = Vec::new();
			data.push((scancode, Color::new(255, 0, 0)));
			let kb = kb.read().unwrap();
			println!("scancode: {:#?}", scancode);
			kb.set_4(&data);
			kb.commit();
			thread::sleep(Duration::from_millis(500));
		});
}
