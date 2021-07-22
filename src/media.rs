use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::time::Duration;
use std::convert::TryFrom;

use zbus::dbus_proxy;
use log::{trace, debug};
use pulse::operation::State as OpState;
use pulse::callbacks::ListResult;

use crate::MainThreadSignal;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlayerStatus
{
	Playing,
	Paused,
	NoMedia
}

impl TryFrom<String> for PlayerStatus
{
	type Error = String;

	fn try_from(status: String) -> Result<Self, Self::Error>
	{
		match status.as_str()
		{
			"Playing" => Ok(Self::Playing),
			"Paused" => Ok(Self::Paused),
			"Stopped" => Ok(Self::NoMedia),
			_ => Err(format!("Unknown PlayerStatus value '{}'", status))
		}
	}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MediaState
{
	pub muted: bool,
	pub player_status: PlayerStatus
}

impl Default for MediaState
{
	fn default() -> Self
	{
		Self
		{
			muted: false,
			player_status: PlayerStatus::NoMedia
		}
	}
}

#[dbus_proxy(interface = "org.freedesktop.DBus")]
trait FreeDesktopDBus
{
	fn list_names(&self) -> zbus::Result<Vec<String>>;
}

#[dbus_proxy(interface = "org.mpris.MediaPlayer2.Player")]
trait MediaPlayer2Player
{
	#[dbus_proxy(property)]
	fn playback_status(&self) -> zbus::Result<String>;
}

pub enum MediaWatcherSignal
{
	Shutdown
}

pub struct MediaWatcher
{
	mpris_players_regex: regex::Regex,
	pulse_context: pulse::context::Context,
	pulse_loop: pulse::mainloop::standard::Mainloop,
	pulse_introspecter: pulse::context::introspect::Introspector,
	dbus: zbus::Connection,
	fd_proxy: FreeDesktopDBusProxy<'static>
}

impl MediaWatcher
{
	pub fn new() -> Result<Self, String>
	{
		let pulse_loop = pulse::mainloop::standard::Mainloop::new()
			.ok_or("failed to allocate pulse mainloop struct")?;
		let pulse_context = pulse::context::Context::new(&pulse_loop, env!("CARGO_PKG_NAME"))
			.ok_or("failed to allocate pulse context struct")?;
		let pulse_introspecter = pulse_context.introspect();
		let dbus = zbus::Connection::new_session().map_err(|e| e.to_string())?;
		let fd_proxy = FreeDesktopDBusProxy::new(&dbus).map_err(|e| e.to_string())?;

		trace!("media watcher starting up, context and dbus ok");

		let mut watcher = Self
		{
			pulse_context,
			pulse_loop,
			pulse_introspecter,
			dbus,
			fd_proxy,
			mpris_players_regex: regex::Regex::new(r"^org\.mpris\.MediaPlayer2\..*$").unwrap()
		};

		watcher.pulse_connect()?;
		Ok(watcher)
	}

	/// Attempts to connect (or re-connect) to the pulse daemon indefinitely until
	/// a ready or error condition is returned
	fn pulse_connect(&mut self) -> Result<(), String>
	{
		use pulse::context::{State, FlagSet};

		trace!("connecting to pulse");

		self.pulse_context.connect(None, FlagSet::NOFLAGS, None)
			.map_err(|e| e.to_string().unwrap_or_else(|| "unknown error".to_string()))?;

		loop
		{
			self.pulse_loop.iterate(true);
			let state = self.pulse_context.get_state();

			trace!("waiting for pulse to connect, state = {:?}", &state);

			match state
			{
				State::Ready => return Ok(()),
				State::Failed => return Err("pulse connection failed".to_string()),
				State::Terminated => return Err("pulse connection terminated".to_string()),
				_ => ()
			}
		}
	}

	/// Searches for all dbus services matching org.mpris.MediaPlayer2.*, selects the
	/// first one it finds, extracts the value of the `PlaybackStatus` property,
	/// and attempts to converts it to a PlayerStatus enum.
	fn player_status(&self) -> Result<PlayerStatus, String>
	{
		self.fd_proxy
			.list_names()
			.map_err(|e| e.to_string())
			.and_then(|service_names| service_names
				.iter()
				.find(|service_name| self.mpris_players_regex.is_match(service_name))
				.cloned()
				.ok_or_else(|| "no loaded media players found on dbus".to_string()))
			.and_then(|player_service|
			{
				let proxy = MediaPlayer2PlayerProxy::new_for(
					&self.dbus,
					player_service.as_ref(),
					"/org/mpris/MediaPlayer2");

				proxy
					.and_then(|proxy| proxy.playback_status())
					.map_err(|e| e.to_string())
					.and_then(PlayerStatus::try_from)
			})
	}

	/// Runs the main loop for the media watcher, watching for changes to mpris
	/// PlayerStatus values and checking the mute state of the current default
	/// pulse sink.
	pub fn run(&mut self, rx: Receiver<MediaWatcherSignal>, tx: Sender<MainThreadSignal>)
	{
		enum PulseReply
		{
			DefaultSinkName(Option<String>),
			Muted(bool)
		}

		let (callback_tx, callback_rx) = channel();
		let mut media_state = MediaState::default();
		let mut default_sink = None;
		let mut server_info_op: Option<pulse::operation::Operation<_>> = None;
		let mut sink_info_op: Option<pulse::operation::Operation<_>> = None;

		loop
		{
			match rx.try_recv()
			{
				Ok(MediaWatcherSignal::Shutdown)
					| Err(TryRecvError::Disconnected) => break,
				Err(TryRecvError::Empty) => ()
			}

			std::thread::sleep(Duration::from_millis(250));

			let mut current_state = MediaState
			{
				player_status: self.player_status().unwrap_or(PlayerStatus::NoMedia),
				// default to the last mute state if pulse hasn't replied in time
				muted: media_state.muted
			};

			loop
			{
				// iterate each time because we most likely have both replies
				// waiting from pulse in the time the thread has been sleeping for
				self.pulse_loop.iterate(false);

				match callback_rx.try_recv()
				{
					Ok(PulseReply::DefaultSinkName(name)) if name != default_sink =>
					{
						debug!("pulse default sink has changed: {:?} => {:?}", &default_sink, &name);
						default_sink = name;
					},
					Ok(PulseReply::Muted(muted)) => current_state.muted = muted,
					Ok(_) => (),
					Err(_) => break
				}
			}

			if media_state != current_state
			{
				debug!("media state has changed: {:?} => {:?}", &media_state, &current_state);
				media_state = current_state;
				tx.send(MainThreadSignal::MediaStateChanged(current_state));
			}

			// make sure we only send another server_info request if we've already
			// had the result of the last one back so we don't get out-of-order replies
			// (same for get_sink_info)

			if server_info_op.as_ref().map(|op| op.get_state() != OpState::Running).unwrap_or(true)
			{
				server_info_op = Some(self.pulse_introspecter.get_server_info(
				{
					let callback_tx = callback_tx.clone();
					move |server_info| callback_tx
						.send(PulseReply::DefaultSinkName(server_info
							.default_sink_name
							.as_deref()
							.map(|name| name.to_owned())))
						.unwrap_or(())
				}));
			}

			if let Some(ref sink_name) = default_sink
			{
				if sink_info_op.as_ref().map(|op| op.get_state() != OpState::Running).unwrap_or(true)
				{
					sink_info_op = Some(self.pulse_introspecter.get_sink_info_by_name(sink_name,
					{
						let callback_tx = callback_tx.clone();
						move |response| if let ListResult::Item(sink_info) = response
						{
							callback_tx.send(PulseReply::Muted(sink_info.mute));
						}
					}));
				}
			}
		}

		self.pulse_context.disconnect();
	}
}
