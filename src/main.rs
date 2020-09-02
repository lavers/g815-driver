#![feature(entry_insert)]
#![allow(unused_must_use)]
#![recursion_limit="512"]

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::sync::{Arc, RwLock};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;

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
	// don't need a rwlock on window_system as it maintains 
	// it's own mutex for thread safety
	window_system: Box<dyn WindowSystem>,
	config: RwLock<Configuration>,
	macro_recording: AtomicBool,
	active_profile: RwLock<String>
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
	let state = Arc::new(SharedState
	{
		window_system: window_system,
		macro_recording: AtomicBool::new(false),
		config: RwLock::new(config),
		active_profile: RwLock::new("default".into())
	});

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
		let main_thread_tx = main_thread_tx.clone();

		pool.execute(move || device_thread(device, state, device_thread_rx, main_thread_tx))
	}

	println!("> now in main event loop, send ctrl-c to shutdown");

	while !should_exit.load(Ordering::Relaxed)
	{
		std::thread::sleep(Duration::from_millis(10));

		if let Ok(message) = main_thread_rx.try_recv()
		{
			match message
			{
				MainThreadSignal::ActiveWindowChanged(active_window) => 
				{
					let config = &state.config.read().unwrap();
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

					device_thread_tx.send(DeviceThreadSignal::ProfileChanged(profile_name.clone()));

					{
						*(state.active_profile.write().unwrap()) = profile_name;
					}
				},
				MainThreadSignal::RunMacroInPool(closure) => pool.execute(closure)
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

pub enum MainThreadSignal
{
	ActiveWindowChanged(Option<ActiveWindowInfo>),
	RunMacroInPool(Box<dyn FnOnce() + Send>)
}

enum DeviceThreadSignal
{
	Shutdown,
	ProfileChanged(String),
	SetScancodes(ScancodeAssignments)
}

fn device_thread(
	kb: Arc<RwLock<g815::G815Keyboard>>, 
	state: Arc<SharedState>, 
	rx: Receiver<DeviceThreadSignal>,
	tx: Sender<MainThreadSignal>)
{
	// map of mode number -> gkey number = (sender to the macro thread, is a toggle macro?)
	let mut macro_states: HashMap<u8, HashMap<u8, (Sender<macros::Signal>, bool)>> = HashMap::new();
	let mut reset_keys = HashSet::new();

	let mut blink_timer = 0;
	let mut blink_state = false;
	let poll_interval = 5;
	let blink_every = 500; // ms
	let mut last_scancodes = None;

	loop
	{
		let mut device = kb.write().unwrap();
		let events = device.poll_for_events();
		let mode = device.mode();
		let mode_states = macro_states.entry(mode).or_default();

		events.iter().for_each(|event| match event
		{
			DeviceEvent::KeyDown(KeyType::GKey, number) =>  
			{
				let mut start_macro = true;

				if let Entry::Occupied(entry) = mode_states.entry(*number)
				{
					let (tx, is_toggle_macro) = entry.get();

					if *is_toggle_macro
					{
						println!("stopping toglge macro");
						tx.send(macros::Signal::Stop);
						start_macro = false;
						reset_keys.insert(*number);
					}

					entry.remove_entry();
				}

				if start_macro
				{
					let macro_ = state.config.read().unwrap().macro_for_gkey(
						state.active_profile.read().unwrap().as_str(), mode, *number);

					if let Some(macro_) = macro_
					{
						let (macro_tx, macro_rx) = channel();
						mode_states.insert(*number, (macro_tx, macro_.is_toggle()));
						println!("starting macro: {:#?}", &macro_);

						let state = Arc::clone(&state);
						tx.send(MainThreadSignal::RunMacroInPool(Box::new(move || 
						{
							macros::Macro::execution_thread(macro_, state, macro_rx)
						})));
					}
				}
			},
			DeviceEvent::KeyUp(KeyType::GKey, number) => 
			{
				if let Entry::Occupied(entry) = mode_states.entry(*number)
				{
					let (macro_tx, is_toggle) = entry.get();

					if !is_toggle
					{
						println!("stopping macro");
						macro_tx.send(macros::Signal::Stop);
						reset_keys.insert(*number);
						entry.remove_entry();
					}
				}
			},
			DeviceEvent::BrightnessLevelChanged(brightness) => 
			{
				println!("brightness level was changed to {}%", brightness)
			},
			DeviceEvent::KeyUp(KeyType::MacroRecord, _) => 
			{
				let new_state = !state.macro_recording.load(Ordering::Relaxed);
				state.macro_recording.store(new_state, Ordering::Relaxed);
				device.set_macro_recording(new_state);
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

		if let Ok(signal) = rx.try_recv()
		{
			match signal
			{
				DeviceThreadSignal::Shutdown => break,
				DeviceThreadSignal::SetScancodes(scancodes) => 
				{
					device.clear_colors();
					device.set_scancodes(&scancodes);
					device.commit();
					last_scancodes = Some(scancodes);
				},
				DeviceThreadSignal::ProfileChanged(_profile) => 
				{
					// stop all macros
				}
			}
		}

		blink_timer += poll_interval;

		if blink_timer >= blink_every
		{
			blink_timer = 0;
			blink_state = !blink_state;

			let color = Color::new(if blink_state { 255 } else { 0 }, 0, 0);
			let override_scancodes: Vec<Scancode> = mode_states.keys()
				.filter_map(|active_macro_number| Scancode::for_gkey(*active_macro_number))
				.collect();

			let mut should_commit = override_scancodes.len() > 0;

			if should_commit
			{
				device.set_13(color, &override_scancodes);
			}

			let reset_scancodes: HashSet<Scancode> = reset_keys.drain()
				.filter_map(|gkey_number| Scancode::for_gkey(gkey_number))
				.collect();

			if let Some(ref last_scancodes) = last_scancodes
			{
				// TODO is it maybe more efficient to just redraw the full keyboard..?
				// or keep a cache of just gkey values...?

				let reset_data: Vec<(Scancode, Color)> = reset_scancodes
					.iter()
					.map(|scancode| (*scancode, last_scancodes
						.iter()
						.find(|(_color, scancodes)| scancodes.contains(scancode))
						.map(|(color, _scancode)| *color)
						.unwrap_or(Color::black())))
					.collect();

				if reset_data.len() > 0
				{
					should_commit = true;
					device.set_4(&reset_data);
				}
			}
				
			if should_commit
			{
				device.commit();
			}
		}

		std::thread::sleep(std::time::Duration::from_millis(poll_interval));
	}
}
