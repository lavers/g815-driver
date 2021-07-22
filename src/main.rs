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
use log::{error, info, trace};
use crossbeam::channel::unbounded;
use clap::{Arg, App};

use config::Configuration;
use device::thread::DeviceSignal;

mod windowsystem;
mod dbus;
mod device;
mod config;
mod macros;
mod media;

pub struct SharedState
{
	config: RwLock<Configuration>,
	macro_recording: AtomicBool,
	active_profile: RwLock<config::Profile>,
	media_state: RwLock<media::MediaState>
}

pub enum MainThreadSignal
{
	ActiveWindowChanged(Option<windowsystem::ActiveWindowInfo>),
	RunMacroInPool(Box<dyn FnOnce() + Send>),
	MediaStateChanged(media::MediaState)
}

fn main()
{
	pretty_env_logger::init();

	let args = App::new("g815-driver")
		.version(env!("CARGO_PKG_VERSION"))
		.author(env!("CARGO_PKG_AUTHORS"))
		.about(env!("CARGO_PKG_DESCRIPTION"))
		.arg(Arg::with_name("palette")
			 .short("p"))
		.get_matches();

	let config = Configuration::load().unwrap();
	// shouldnt ever need more than 20 threads, as that can handle all
	// 15 possible simultaneous macros + the device/watcher threads
	let pool = ThreadPool::new(20);
	let hidapi = HidApi::new().unwrap();
	let devices = device::find_devices(hidapi);
	let initial_profile = config.default_profile().clone();

	let state = Arc::new(SharedState
	{
		macro_recording: AtomicBool::new(false),
		config: RwLock::new(config),
		active_profile: RwLock::new(initial_profile),
		media_state: RwLock::new(media::MediaState::default())
	});

	let should_exit = Arc::new(AtomicBool::new(false));
	let (main_thread_tx, main_thread_rx) = channel();
	let (device_thread_tx, device_thread_rx) = unbounded();
	let (dbus_thread_tx, dbus_thread_rx) = channel();
	let (ww_thread_tx, ww_thread_rx) = channel();
	let (config_watcher_tx, config_watcher_rx) = channel();
	let (media_watcher_tx, media_watcher_rx) = channel();

	let mut config_watcher = notify::watcher(config_watcher_tx, Duration::from_secs(3)).unwrap();
	let mut config_file = Configuration::file_path();
	// get the folder containing the config file for watching as
	// some editors (vim) will delete the file and write a new one
	// when saving, killing the watcher
	config_file.pop();
	use notify::Watcher;
	config_watcher.watch(config_file, notify::RecursiveMode::NonRecursive).unwrap();

	ctrlc::set_handler(
	{
		let should_exit = should_exit.clone();
		move || should_exit.store(true, Ordering::Relaxed)
	});

	if args.is_present("palette")
	{
		let mut current = hsl::HSL { h: 0_f64, s: 1_f64, l: 0.5_f64 };

		ncurses::initscr();
		ncurses::addstr("you're in color palette (tester) mode.\n");
		ncurses::addstr("Press h/l to decrease/increase hue by 1 (capital for 10), q to quit.\n\n");
		ncurses::refresh();

		let mut devices = devices;
		let mut kb = devices.pop().unwrap();
		kb.take_control();

		loop
		{
			let color = current.into();
			kb.set_all(color);
			kb.commit();

			ncurses::addstr(format!("\rCurrent hue: {}deg (#{:x})", current.h, color).as_str());

			match ncurses::getch() as u8 as char
			{
				'h' => current.h -= 1_f64,
				'H' => current.h -= 10_f64,
				'l' => current.h += 1_f64,
				'L' => current.h += 10_f64,
				'q' => break,
				_ => ()
			};
		}

		ncurses::endwin();
		kb.release_control();
		should_exit.store(true, Ordering::Relaxed)
	}
	else
	{
		pool.execute(
		{
			let main_thread_tx = main_thread_tx.clone();
			move || dbus::Server::new(dbus_thread_rx, main_thread_tx).run()
		});

		pool.execute(
		{
			let main_thread_tx = main_thread_tx.clone();
			move || windowsystem::WindowSystem::new().unwrap().run(ww_thread_rx, main_thread_tx)
		});

		pool.execute(
		{
			let main_thread_tx = main_thread_tx.clone();
			move || media::MediaWatcher::new().unwrap().run(media_watcher_rx, main_thread_tx)
		});

		for device in devices
		{
			pool.execute(
			{
				let state = Arc::clone(&state);
				let main_thread_tx = main_thread_tx.clone();
				let device_thread_rx = device_thread_rx.clone();
				let dbus_thread_tx = dbus_thread_tx.clone();
				let ww_thread_tx = ww_thread_tx.clone();
				move || device::thread::DeviceThread::new(
					device,
					state,
					dbus_thread_tx,
					ww_thread_tx,
					main_thread_tx)
					.event_loop(device_thread_rx)
			});
		}
	}

	info!("ready!");
	trace!("startup complete, now in main event loop");

	let mut last_active_window = None;

	while !should_exit.load(Ordering::Relaxed)
	{
		thread::sleep(Duration::from_millis(10));

		if let Ok(notify::DebouncedEvent::Create(path))
			| Ok(notify::DebouncedEvent::NoticeWrite(path)) = config_watcher_rx.try_recv()
		{
			if path.file_name() == Some(Configuration::config_filename().as_ref())
			{
				info!("configuration file has been changed, will reload");

				match Configuration::load()
				{
					Ok(new_config) =>
					{
						info!("new config loaded OK, notifying devices");
						*(state.config.write().unwrap()) = new_config;
						device_thread_tx.send(DeviceSignal::ConfigurationReloaded);
						main_thread_tx.send(MainThreadSignal::ActiveWindowChanged(
							last_active_window.clone()));
					},
					Err(config_error) => error!(
						"changed configuration cannot be loaded: {}",
						&config_error)
				}
			}
		}

		match main_thread_rx.try_recv()
		{
			Ok(MainThreadSignal::RunMacroInPool(closure)) => pool.execute(closure),
			Ok(MainThreadSignal::MediaStateChanged(new)) =>
			{
				*state.media_state.write().unwrap() = new;
				device_thread_tx.send(DeviceSignal::MediaStateChanged);
			},
			Ok(MainThreadSignal::ActiveWindowChanged(active_window)) =>
			{
				let config = state.config.read().unwrap();
				let (name, profile) = config.profile_for_active_window(&active_window);

				info!("active window has changed\n\twindow: {}\n\tapplying profile: {}",
					  active_window
						.as_ref()
						.map(|window| format!("{}", window))
						.unwrap_or_else(|| "[no active window]".into()),
					  &name);

				*(state.active_profile.write().unwrap()) = profile.clone();
				device_thread_tx.send(DeviceSignal::ProfileChanged);
				last_active_window = active_window;
			},
			Err(_) => ()
		}
	}

	trace!("notifying threads of shutdown");

	device_thread_tx.send(DeviceSignal::Shutdown);
	ww_thread_tx.send(windowsystem::WindowSystemSignal::Shutdown);
	dbus_thread_tx.send(dbus::DBusSignal::Shutdown);
	media_watcher_tx.send(media::MediaWatcherSignal::Shutdown);
	pool.join();

	trace!("threadpool shutdown");
}
