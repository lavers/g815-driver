use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;

use serde::{Serialize, Deserialize};

use crate::{SharedState, MainThreadSignal};
use crate::macros::{Macro, MacroSignal, ActivationType};
use scancode::Scancode;
use rgb::{Color, ScancodeAssignments};

pub mod g815;
pub mod scancode;
pub mod rgb;

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
	Next,
	Previous,
	PlayPause,
	VolumeUp,
	VolumeDown,
	Mute
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

impl Default for CapabilityData
{
	fn default() -> Self
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

pub enum DeviceThreadSignal
{
	Shutdown,
	ProfileChanged(String),
	SetScancodes(ScancodeAssignments),
	ConfigurationReloaded
}

type MacroState = (Sender<MacroSignal>, Arc<AtomicBool>, ActivationType);

pub struct DeviceThread
{
	device: g815::G815Keyboard, 
	state: Arc<SharedState>, 
	main_thread_tx: Sender<MainThreadSignal>,
	// map of mode number -> gkey number = Current macro state
	macro_states: HashMap<u8, HashMap<u8, MacroState>>,
	blink_timer: u64,
	blink_state: bool,
	active_mode: u8,
	mode_count: u8,
	last_color_data: Option<ScancodeAssignments>
}

impl DeviceThread
{
	// both these in milliseconds
	const POLL_INTERVAL: u64 = 5;
	const BLINK_DELAY: u64 = 400;

	pub fn new(
		device: g815::G815Keyboard, 
		state: Arc<SharedState>, 
		main_thread_tx: Sender<MainThreadSignal>) -> Self
	{
		let mode_count = device.mode_count().unwrap_or(0);

		DeviceThread
		{
			device,
			state,
			main_thread_tx,
			macro_states: HashMap::new(),
			blink_timer: 0,
			blink_state: false,
			active_mode: 1,
			mode_count,
			last_color_data: None
		}
	}

	fn current_mode_macro_states<'a>(&'a mut self) 
		-> &'a mut HashMap<u8, MacroState>
	{
		self.macro_states.entry(self.active_mode).or_default()
	}

	fn macro_for_gkey(&self, gkey_number: u8) -> Option<Macro>
	{
		self.state.config.read().unwrap()
			.macro_for_gkey(
				self.state.active_profile.read().unwrap().as_str(), 
				self.active_mode, 
				gkey_number)
	}

	fn last_color_for_scancode(&self, scancode: Scancode) -> Color
	{
		self.last_color_data	
			.as_ref()
			.and_then(|color_data| color_data
				.iter()
				.find(|(_color, scancodes)| scancodes.contains(&scancode))
				.map(|(color, _scancodes)| *color))
			.unwrap_or_else(Color::black)
	}

	/// Main event loop for a connected device. General flow is:
	///    - Poll for events from the device, then handle them
	///    - Handle any signals from other threads
	///    - Update indicators on the keyboard as a result of any state changes
	pub fn event_loop(&mut self, rx: Receiver<DeviceThreadSignal>)
	{
		loop
		{
			self.device
				.poll_for_events()
				.iter()
				.for_each(|event| self.handle_event(event));

			match rx.try_recv()
			{
				Err(TryRecvError::Empty) => (),

				Err(TryRecvError::Disconnected)
					| Ok(DeviceThreadSignal::Shutdown) => 
				{
					self.device.release_control();
					return;
				},

				Ok(DeviceThreadSignal::SetScancodes(scancodes)) => 
				{
					self.device.clear_colors();
					self.device.set_scancodes(&scancodes);
					self.device.commit();
					self.last_color_data = Some(scancodes);
				},

				Ok(DeviceThreadSignal::ConfigurationReloaded)
					| Ok(DeviceThreadSignal::ProfileChanged(_)) => 
				{
					self.blink_timer = Self::BLINK_DELAY;
					self.stop_and_remove_all_macros();
				}
			}

			self.update_macro_indicators();

			thread::sleep(Duration::from_millis(Self::POLL_INTERVAL));
		}
	}

	fn handle_event(&mut self, event: &DeviceEvent)
	{
		match event
		{
			DeviceEvent::KeyDown(KeyType::GKey, number) => self.macro_keydown(*number),
			DeviceEvent::KeyUp(KeyType::GKey, number) => self.macro_keyup(*number),

			DeviceEvent::BrightnessLevelChanged(brightness) => 
			{
				// not sure what use this will be yet, maybe send a notification
				// via dbus..?
				println!("brightness level was changed to {}%", brightness)
			},

			DeviceEvent::KeyUp(KeyType::MacroRecord, _) => 
			{
				let new_state = !self.state.macro_recording.load(Ordering::Relaxed);
				self.state.macro_recording.store(new_state, Ordering::Relaxed);
				self.device.set_macro_recording(new_state);
			},

			DeviceEvent::KeyDown(KeyType::Mode, mode) => 
			{
				self.active_mode = *mode;
				self.blink_timer = Self::BLINK_DELAY;
				self.stop_all_hold_to_repeat_macros();
			},

			DeviceEvent::MediaKeyDown(key) => self.state.window_system.send_key_combo_press(match key
			{
				MediaKey::Mute => "XF86AudioMute",
				MediaKey::PlayPause => "XF86AudioPlay",
				MediaKey::Next => "XF86AudioNext",
				MediaKey::Previous => "XF86AudioPrev",
				MediaKey::VolumeUp => "XF86AudioRaiseVolume",
				MediaKey::VolumeDown => "XF86AudioLowerVolume"
			}),

			_ => ()
		}
	}

	fn update_macro_indicators(&mut self)
	{
		self.blink_timer += Self::POLL_INTERVAL;

		if self.blink_timer < Self::BLINK_DELAY
		{
			return
		}

		self.blink_timer = 0;
		self.blink_state = !self.blink_state;

		let blink_color = Color::new(if self.blink_state { 255 } else { 0 }, 0, 0);
		let mut gkey_data: Vec<(Scancode, Color)> = Vec::new();

		// TODO proabably re-implement this section when drain_filter is added to HashMap

		let stopped_macro_numbers: HashMap<u8, HashSet<u8>> = self.macro_states
			.iter()
			.map(|(mode, mode_states)| 
			{
				 let stopped_mode_macros = mode_states
					.iter()
					.filter_map(|(gkey_number, (_tx, stopped, _activation_type))|
					{
						let stopped = stopped.load(Ordering::Relaxed).then_some(*gkey_number);

						// if this is the current mode, and the macro is running or stopped,
						// override the color of the key as appropriate

						if *mode == self.active_mode
						{
							let scancode = Scancode::from_gkey(*gkey_number).unwrap();
							let set_color = stopped
								.map(|_gkey_number| self.last_color_for_scancode(scancode))
								.unwrap_or(blink_color);
							gkey_data.push((scancode, set_color));
						}

						stopped
					})
					.collect();

				(*mode, stopped_mode_macros)
			})
			.collect();

		for (mode, mode_states) in &mut self.macro_states
		{
			if let Some(mode_stopped_macros) = stopped_macro_numbers.get(mode)
			{
				mode_states.retain(|gkey_number, _state| 
					!mode_stopped_macros.contains(gkey_number));
			}
		}

		if !gkey_data.is_empty()
		{
			self.device.set_4(&gkey_data);
			self.device.commit();
		}

		(1..=self.mode_count).for_each(|mode| 
		{
			let led_on = if mode == self.active_mode
			{
				true
			}
			else
			{
				let mode_has_active_macros = self.macro_states
					.get(&mode)
					.map(|mode_macros| !mode_macros.is_empty())
					.unwrap_or(false);

				mode_has_active_macros && self.blink_state
			};

			self.device.set_mode_led(mode, led_on);
		});
	}

	fn macro_keydown(&mut self, gkey_number: u8)
	{
		println!("gkey down {}", gkey_number);

		if let Entry::Occupied(ref entry) = self.current_mode_macro_states().entry(gkey_number)
		{
			let (tx, stopped, activation_type) = entry.get();

			if !stopped.load(Ordering::Relaxed)
			{
				println!("has hashmap entry, activationtype: {:#?}", &activation_type);

				match activation_type
				{
					ActivationType::Toggle => 
					{
						println!("stopping toggle macro");
						tx.send(MacroSignal::Stop);
						return
					},
					ActivationType::Repeat(_count) => 
					{
						println!("resetting count on repeat macro");
						tx.send(MacroSignal::ResetCount);
						return
					},
					_ => ()
				}
			}
		}

		if let Some(macro_) = self.macro_for_gkey(gkey_number)
		{
			println!("starting macro: {:#?}", &macro_);

			let (macro_tx, macro_rx) = channel();
			let state = Arc::clone(&self.state);
			let stopped = Arc::new(AtomicBool::new(false));
			let macro_thread_stopped = Arc::clone(&stopped);

			self.current_mode_macro_states().insert(gkey_number, 
				(macro_tx, stopped, macro_.activation_type));

			self.main_thread_tx.send(MainThreadSignal::RunMacroInPool(Box::new(move || 
			{
				Macro::execution_thread(
					macro_, 
					state, 
					macro_rx, 
					macro_thread_stopped)
			})));
		}
	}

	fn macro_keyup(&mut self, gkey_number: u8)
	{
		if let Some((tx, _stopped, ActivationType::HoldToRepeat)) = self
			.current_mode_macro_states().get(&gkey_number)
		{
			println!("stopping hold to repeat macro");
			tx.send(MacroSignal::Stop);
		}
	}

	fn stop_all_hold_to_repeat_macros(&self)
	{
		for (_mode, mode_macros) in &self.macro_states
		{
			for (_gkey_number, (tx, _stopped, activation_type)) in mode_macros
			{
				if *activation_type == ActivationType::HoldToRepeat
				{
					tx.send(MacroSignal::Stop);
				}
			}
		}
	}

	fn stop_and_remove_all_macros(&mut self)
	{
		self.macro_states
			.drain()
			.for_each(|(_mode, mut mode_macros)|
			{
				mode_macros
					.drain()
					.for_each(|(_gkey_number, (tx, _stopped, _activation_type))| 
					{
						tx.send(MacroSignal::Stop);
					});
			});
	}
}
