#![feature(entry_insert)]
#![allow(unused_must_use)]
#![feature(bool_to_option)]
#![recursion_limit="512"]
#![allow(clippy::suspicious_else_formatting)]

use std::sync::{Arc, RwLock};
use std::sync::mpsc::channel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;

use hidapi::HidApi;
use threadpool::ThreadPool;
use notify::{Watcher, watcher};

use config::Configuration;
use windowsystem::{WindowSystem, ActiveWindowInfo};
use device::g815;
use device::DeviceThreadSignal;

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

pub enum MainThreadSignal
{
	ActiveWindowChanged(Option<ActiveWindowInfo>),
	RunMacroInPool(Box<dyn FnOnce() + Send>)
}

fn main()
{
	let config = Configuration::load().unwrap();
	let hidapi = HidApi::new().unwrap();
	let pool = ThreadPool::new(10);
	let window_system = WindowSystem::new().unwrap();

	let mut device = g815::G815Keyboard::new(&hidapi).unwrap();
	device.load_capabilities();
	device.take_control();

	let state = Arc::new(SharedState
	{
		window_system,
		macro_recording: AtomicBool::new(false),
		config: RwLock::new(config),
		active_profile: RwLock::new("default".into())
	});

	let should_exit = Arc::new(AtomicBool::new(false));
	{
		let should_exit = should_exit.clone();
		ctrlc::set_handler(move || 
		{
			println!("got ctrl-c");
			should_exit.store(true, Ordering::Relaxed);
		});
	}

	let (main_thread_tx, main_thread_rx) = channel();
	let (device_thread_tx, device_thread_rx) = channel();
	let (ww_thread_tx, ww_thread_rx) = channel();

	let (config_watcher_tx, config_watcher_rx) = channel();
	let mut config_watcher = watcher(config_watcher_tx, Duration::from_secs(3)).unwrap();
	let mut config_file = Configuration::config_file_location();
	// get the folder containing the config file for watching as
	// some editors will delete the file, killing the watcher
	config_file.pop();
	config_watcher.watch(config_file, notify::RecursiveMode::NonRecursive).unwrap();

	{
		let state = Arc::clone(&state);
		let main_thread_tx = main_thread_tx.clone();
		pool.execute(move || WindowSystem::active_window_watcher(state, ww_thread_rx, main_thread_tx))
	}

	{
		let state = Arc::clone(&state);
		let main_thread_tx = main_thread_tx.clone();
		pool.execute(move || 
		{
			device::DeviceThread::new(device, state, main_thread_tx)
				.event_loop(device_thread_rx);
		})
	}

	println!("> now in main event loop, send ctrl-c to shutdown");

	while !should_exit.load(Ordering::Relaxed)
	{
		thread::sleep(Duration::from_millis(10));

		match config_watcher_rx.try_recv()
		{
			Ok(notify::DebouncedEvent::NoticeRemove(_path)) => (),
			Ok(event) => 
			{
				println!("{:#?}", &event);
				let mut config = state.config.write().unwrap();
				
				match Configuration::load()
				{
					Ok(new_config) => 
					{
						*config = new_config;
						println!("new config loaded: {:#?}", &config);
						device_thread_tx.send(DeviceThreadSignal::ConfigurationReloaded);
						let active_window = state.window_system.active_window_info();
						main_thread_tx.send(MainThreadSignal::ActiveWindowChanged(active_window));
					},
					Err(config_error) => println!("error loading new config: {:#?}", &config_error)
				}
			},
			_ => ()
		}

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
}
