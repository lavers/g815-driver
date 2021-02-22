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
use std::path::Path;

use hidapi::HidApi;
use threadpool::ThreadPool;
use notify::{Watcher, watcher};
use log::{error, info, debug, trace};
use crossbeam::channel::unbounded;
use clap::{Arg, App};

use config::Configuration;
use windowsystem::{WindowSystem, ActiveWindowInfo};
use device::thread::DeviceThreadSignal;

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
	ActiveWindowChanged(Option<ActiveWindowInfo>),
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
	let hidapi = HidApi::new().unwrap();
	// shouldnt ever need more than 20 threads, as that can handle all
	// 15 possible simultaneous macros + the device/watcher threads
	let pool = ThreadPool::new(20);

    let socket_file = Path::new("g815d.sock");

    if socket_file.exists()
    {
        std::fs::remove_file(socket_file);
    }

    let socket = std::os::unix::net::UnixListener::bind(socket_file).unwrap();
    socket.set_nonblocking(true);

    let devices: Vec<Box<dyn device::Device>> = hidapi
        .device_list()
        .filter_map(|dev_info|
        {
            let dev = match (dev_info.vendor_id(), dev_info.product_id(), dev_info.interface_number())
            {
                (0x046d, 0xc33f, 1) => Some(dev_info.open_device(&hidapi)
                    .map(|device| device::g815::G815Keyboard::new(device))),
                _ => None
            };

            dev.map(|dev| (dev_info.product_string().unwrap_or("unknown"), dev))
        })
        .filter_map(|(product_name, device_result)| match device_result
        {
            Ok(mut device) =>
            {
                info!("Successfully opened '{}'\n{}", product_name, device.firmware_info());
                Some(device)
            },
            Err(error) =>
            {
                error!("Failed to open target device '{}': {}", product_name, error);
                None
            }
        })
        .collect();

	let initial_profile = config.default_profile().clone();

	let (dbus_thread_tx, dbus_thread_rx) = channel();

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
	let (ww_thread_tx, ww_thread_rx) = channel();
	let (config_watcher_tx, config_watcher_rx) = channel();
	let (media_watcher_tx, media_watcher_rx) = channel();

	let mut config_watcher = watcher(config_watcher_tx, Duration::from_secs(3)).unwrap();
	let mut config_file = Configuration::config_file_location();
	// get the folder containing the config file for watching as
	// some editors will delete the file, killing the watcher
	config_file.pop();
	config_watcher.watch(config_file, notify::RecursiveMode::NonRecursive).unwrap();

	// watch for ctrl-c and SIGTERM, and stop everything nicely
	ctrlc::set_handler(
	{
		let should_exit = should_exit.clone();
		move || should_exit.store(true, Ordering::Relaxed)
	});

	if args.is_present("palette")
	{

		let mut current = hsl::HSL { h: 0_f64, s: 1_f64, l: 0.5_f64 };

		ncurses::initscr();
		ncurses::addstr(format!("you're in color palette (tester) mode.\n").as_str());
		ncurses::addstr(format!("Press h/l to decrease/increase hue by 1 (capital for 10), q to quit.\n\n").as_str());
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
			move || dbus::Server::new(main_thread_tx, dbus_thread_rx).run()
		});

		pool.execute(
		{
			let main_thread_tx = main_thread_tx.clone();
			let window_system = WindowSystem::new().unwrap();
			move || window_system.event_loop(ww_thread_rx, main_thread_tx)
		});

		pool.execute(
		{
			let main_thread_tx = main_thread_tx.clone();
			move || media::MediaWatcher::new()
				.unwrap()
				.run(media_watcher_rx, main_thread_tx)
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
	debug!("startup complete, now in main event loop");

	let mut last_active_window = None;

	while !should_exit.load(Ordering::Relaxed)
	{
		thread::sleep(Duration::from_millis(10));

		match config_watcher_rx.try_recv()
		{
			Ok(notify::DebouncedEvent::NoticeRemove(_path)) => (),
			Ok(notify::DebouncedEvent::Create(path))
				| Ok(notify::DebouncedEvent::NoticeWrite(path)) =>
			{
				if let Some(file_name) = path.file_name()
				{
					if file_name == "config.yml"
					{
						info!("configuration file was changed, will reload");

						match Configuration::load()
						{
							Ok(new_config) =>
							{
								info!("new config loaded OK, notifying devices");
								*(state.config.write().unwrap()) = new_config;
								device_thread_tx.send(DeviceThreadSignal::ConfigurationReloaded);
								main_thread_tx.send(MainThreadSignal::ActiveWindowChanged(
									last_active_window.clone()));
							},
							Err(config_error) => error!("new configuration cannot be loaded: {}", &config_error)
						}
					}
				}
			},
			_ => ()
		}

        if let Ok((_client_stream, _address)) = socket.accept()
        {

        }

		if let Ok(signal) = main_thread_rx.try_recv()
		{
			match signal
			{
				MainThreadSignal::RunMacroInPool(closure) => pool.execute(closure),
				MainThreadSignal::MediaStateChanged(new) =>
				{
					let mut current = state.media_state.write().unwrap();
					debug!("media state changed: {:?} => {:?}", &current, &new);
					*current = new;
					device_thread_tx.send(DeviceThreadSignal::MediaStateChanged);
				},
				MainThreadSignal::ActiveWindowChanged(active_window) =>
				{
					let config = &state.config.read().unwrap();
					let (name, profile) = config.profile_for_active_window(&active_window);

					trace!("active window changed: {:#?}\napplying profile: {:#?}",
						   &active_window,
						   &profile);
					info!("active window has changed\n\twindow: {}\n\tapplying profile: {}",
						  active_window
							.as_ref()
							.map(|window| format!("{}", window))
							.unwrap_or_else(|| "[no active window]".into()),
						  &name);

					*(state.active_profile.write().unwrap()) = profile.clone();
					device_thread_tx.send(DeviceThreadSignal::ProfileChanged);
					last_active_window = active_window;
				}
			}
		}
	}

	debug!("notifying threads of shutdown");

	device_thread_tx.send(DeviceThreadSignal::Shutdown);
	ww_thread_tx.send(windowsystem::WindowSystemSignal::Shutdown);
	dbus_thread_tx.send(dbus::DBusSignal::Shutdown);
	media_watcher_tx.send(media::MediaWatcherSignal::Shutdown);
	pool.join();

	debug!("threadpool shutdown");
}
