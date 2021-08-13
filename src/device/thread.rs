use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::mpsc::{channel, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;

use log::{info, debug};
use crossbeam::{Receiver, TryRecvError};

use crate::{SharedState, MainThreadSignal};
use crate::macros::{Macro, MacroSignal, ActivationType};
use crate::dbus::DBusSignal;
use crate::windowsystem::WindowSystemSignal;
use super::rgb::{ScancodeAssignments, EffectGroup, EffectConfiguration, Theme, Color};
use super::scancode::Scancode;
use super::{Device, DeviceEvent, KeyType, MediaKey};

type MacroState = (Sender<MacroSignal>, Arc<AtomicBool>, ActivationType);

pub enum DeviceSignal
{
	Shutdown,
	ProfileChanged,
	ConfigurationReloaded,
	MediaStateChanged
}

enum CurrentLightingState
{
	Custom(ScancodeAssignments),
	Effect(EffectConfiguration)
}

pub struct DeviceThread
{
	device: Box<dyn Device>,
	state: Arc<SharedState>,
	main_thread_tx: Sender<MainThreadSignal>,
	dbus_tx: Sender<DBusSignal>,
	window_system_tx: Sender<WindowSystemSignal>,
	// map of mode number -> gkey number = Current macro state
	macro_states: HashMap<u8, HashMap<u8, MacroState>>,
	lighting_state: CurrentLightingState,
	blink_timer: u64,
	blink_state: bool,
	active_mode: u8,
	mode_count: u8,
	overrides: HashMap<Scancode, Color>
}

impl DeviceThread
{
	// both these in milliseconds
	const POLL_INTERVAL: u64 = 5;
	const BLINK_DELAY: u64 = 400;

	pub fn new(
		device: Box<dyn Device>,
		state: Arc<SharedState>,
		dbus_tx: Sender<DBusSignal>,
		window_system_tx: Sender<WindowSystemSignal>,
		main_thread_tx: Sender<MainThreadSignal>) -> Self
	{
		let mode_count = device.mode_count().unwrap_or(0);

		Self
		{
			device,
			state,
			main_thread_tx,
			window_system_tx,
			dbus_tx,
			mode_count,
			macro_states: HashMap::new(),
			lighting_state: CurrentLightingState::Effect(EffectConfiguration::None),
			blink_timer: 0,
			blink_state: false,
			active_mode: 1,
			overrides: HashMap::new()
		}
	}

	fn current_mode_macro_states(&mut self) -> &mut HashMap<u8, MacroState>
	{
		self.macro_states.entry(self.active_mode).or_default()
	}

	fn macro_for_gkey(&self, gkey_number: u8) -> Option<Macro>
	{
		let config = self.state.config.read().unwrap();
		let current_profile = self.state.active_profile.read().unwrap();

		current_profile
			.macro_for_gkey(&config, self.active_mode, gkey_number)
			.map(|macro_| macro_.into_owned())
	}

	fn last_color_for_scancode(&self, scancode: Scancode) -> Color
	{
		let last_color = match &self.lighting_state
		{
			CurrentLightingState::Custom(color_data) => color_data
				.iter()
				.find(|(_color, scancodes)| scancodes.contains(&scancode))
				.map(|(color, _scancodes)| *color),
			CurrentLightingState::Effect(_data) => None
		};

		last_color.unwrap_or_else(Color::black)
	}

	/// Main event loop for a connected device. General flow is:
	///    - Poll for events from the device, then handle them
	///    - Handle any signals from other threads
	///    - Update indicators on the keyboard as a result of any state changes
	pub fn event_loop(&mut self, rx: Receiver<DeviceSignal>)
	{
		self.device.take_control();

		loop
		{
			self.device
				.get_events()
				.iter()
				.for_each(|event| self.handle_event(event));

			match rx.try_recv()
			{
				Err(TryRecvError::Empty) => (),

				Err(TryRecvError::Disconnected)
					| Ok(DeviceSignal::Shutdown) => break,

				Ok(DeviceSignal::ConfigurationReloaded)
					| Ok(DeviceSignal::ProfileChanged) =>
				{
					self.blink_timer = Self::BLINK_DELAY;
					self.stop_and_remove_all_macros();
					self.apply_profile();
					self.apply_overrides();
					self.device.commit();
				},

				Ok(DeviceSignal::MediaStateChanged) =>
				{
					use crate::media::PlayerStatus;

					let media_state = { *self.state.media_state.read().unwrap() };
					let no_media = media_state.player_status == PlayerStatus::NoMedia;
					let red = Color::new(255, 0, 0);

					self.set_override(Scancode::Mute, media_state.muted.then(|| red));
					self.set_override(Scancode::MediaPrevious, no_media.then(Color::black));
					self.set_override(Scancode::MediaNext, no_media.then(Color::black));
					self.set_override(Scancode::MediaPlayPause, match media_state.player_status
					{
						PlayerStatus::Playing => None,
						PlayerStatus::Paused => Some(red),
						PlayerStatus::NoMedia => Some(Color::black())
					});

					self.apply_profile();
					self.apply_overrides();
					self.device.commit();
				}
			}

			// don't try and override keys if an effect is running
			if let CurrentLightingState::Custom(_data) = &self.lighting_state
			{
				self.update_macro_indicators();
			}

			thread::sleep(Duration::from_millis(Self::POLL_INTERVAL));
		}

		self.device.release_control();
	}

	fn apply_profile(&mut self)
	{
		let config = self.state.config.read().unwrap();
		let profile = self.state.active_profile.read().unwrap();
		let theme = profile.theme(&config, self.active_mode);

		self.device.reset_game_mode_keys();

		if let Some(game_mode_scancodes) = &profile.game_mode_keys
		{
			self.device.add_game_mode_keys(game_mode_scancodes);
		}

		match theme
		{
			Theme::Static(_assignments) =>
			{
				// fine to unwrap this, None is only returned for Theme::Effect variants
				let scancodes = theme.scancode_assignments(&config.keygroups).unwrap();
				//self.device.clear(); this is causing flickering
				self.device.set_all(Color::black());
				self.device.apply_scancode_assignments(&scancodes);
				self.device.commit();
				self.lighting_state = CurrentLightingState::Custom(scancodes);
			},
			Theme::Effect(effect) =>
			{
				// TODO work out wtf is going on with the logo
				let group = EffectGroup::Keys;
				self.device.set_effect(group, effect);
				self.lighting_state = CurrentLightingState::Effect(effect.clone());
			}
		}
	}

	fn set_override<C>(&mut self, scancode: Scancode, color: C)
	where
		C: Into<Option<Color>> + std::fmt::Debug
	{
		debug!("set override for {:?} to {:?}", &scancode, &color);
		if let Some(color) = color.into()
		{
			self.overrides.insert(scancode, color);
		}
		else
		{
			self.overrides.remove(&scancode);
		}
	}

	fn apply_overrides(&mut self)
	{
		if let CurrentLightingState::Custom(_) = &self.lighting_state
		{
			let mut assignments = HashMap::new();

			for (scancode, color) in &self.overrides
			{
				assignments
					.entry(*color)
					.or_insert_with(Vec::new)
					.push(*scancode);
			}

			let assignments: ScancodeAssignments = assignments.drain().collect();
			self.device.apply_scancode_assignments(assignments.as_ref());
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
				info!("brightness level was changed to {}%", brightness)
			},

			DeviceEvent::KeyUp(KeyType::MacroRecord, _) =>
			{
				let new_state = !self.state.macro_recording.load(Ordering::Relaxed);
				self.state.macro_recording.store(new_state, Ordering::Relaxed);
				self.device.set_macro_recording(new_state);
			},

			DeviceEvent::KeyDown(KeyType::Mode, mode) =>
			{
				debug!("mode changed to: {}", mode);
				self.active_mode = *mode;
				self.blink_timer = Self::BLINK_DELAY;
				self.stop_all_hold_to_repeat_macros();
			},

			DeviceEvent::MediaKeyDown(key) => self.window_system_tx
				.send(WindowSystemSignal::SendKeyCombo(match key
				{
					MediaKey::Mute => "XF86AudioMute",
					MediaKey::PlayPause => "XF86AudioPlay",
					MediaKey::Next => "XF86AudioNext",
					MediaKey::Previous => "XF86AudioPrev",
					MediaKey::VolumeUp => "XF86AudioRaiseVolume",
					MediaKey::VolumeDown => "XF86AudioLowerVolume"
				}.to_string()))
				.unwrap_or(()),

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
						let stopped = stopped.load(Ordering::Relaxed).then(|| *gkey_number);

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
				debug!("clearing stopped macros in mode {}: {:#?}", mode, mode_stopped_macros);

				mode_states.retain(|gkey_number, _state|
					!mode_stopped_macros.contains(gkey_number));
			}
		}

		if !gkey_data.is_empty()
		{
			self.device.set_4(&gkey_data);
			self.device.commit();
		}

		let mut mode_leds = 0;

		for mode in 1..=self.mode_count
		{
			if mode != self.active_mode
			{
				let mode_has_active_macros = self.macro_states
					.get(&mode)
					.map(|mode_macros| !mode_macros.is_empty())
					.unwrap_or(false);

				if !mode_has_active_macros || !self.blink_state
				{
					continue
				}
			}

			mode_leds |= 1 << (mode - 1);
		}

		self.device.set_mode_leds(mode_leds);
	}

	fn macro_keydown(&mut self, gkey_number: u8)
	{
		debug!("gkey down {}", gkey_number);

		if let Entry::Occupied(ref entry) = self.current_mode_macro_states().entry(gkey_number)
		{
			let (tx, stopped, activation_type) = entry.get();

			if !stopped.load(Ordering::Relaxed)
			{
				debug!("macro slot is already active, activationtype: {:#?}", &activation_type);

				match activation_type
				{
					ActivationType::Toggle =>
					{
						debug!("stopping toggle macro");
						tx.send(MacroSignal::Stop);
						return
					},
					ActivationType::Repeat(_count) =>
					{
						debug!("resetting count on repeat macro");
						tx.send(MacroSignal::ResetCount);
						return
					},
					_ => ()
				}
			}
		}

		if let Some(macro_) = self.macro_for_gkey(gkey_number)
		{
			debug!("starting macro: {:#?}", &macro_);

			let (macro_tx, macro_rx) = channel();
			let stopped = Arc::new(AtomicBool::new(false));
			let macro_thread_stopped = Arc::clone(&stopped);

			self.current_mode_macro_states().insert(gkey_number,
				(macro_tx, stopped, macro_.activation_type));

			self.main_thread_tx.send(MainThreadSignal::RunMacroInPool(Box::new(
			{
				let window_system_tx = self.window_system_tx.clone();
				let dbus_tx = self.dbus_tx.clone();
				move || macro_.execute(macro_rx, window_system_tx, dbus_tx, macro_thread_stopped)
			})));
		}
	}

	fn macro_keyup(&mut self, gkey_number: u8)
	{
		debug!("gkey up {}", gkey_number);

		if let Some((tx, _stopped, ActivationType::HoldToRepeat)) = self
			.current_mode_macro_states().get(&gkey_number)
		{
			debug!("stopping hold to repeat macro");
			tx.send(MacroSignal::Stop);
		}
	}

	fn stop_all_hold_to_repeat_macros(&self)
	{
		debug!("stopping all hold to repeat macros");

		for mode_macros in self.macro_states.values()
		{
			for (tx, _stopped, activation_type) in mode_macros.values()
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
		debug!("stopping all macros");

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
