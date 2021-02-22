use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::process::{Command, Stdio};
use std::env;

use serde::{Serialize, Deserialize};

use crate::windowsystem::{MouseButton, WindowSystemSignal};
use crate::dbus::DBusSignal;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationType
{
	Singular,
	Repeat(u32),
	HoldToRepeat,
	Toggle
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action
{
	MouseClick(MouseButton),
	KeyPress(String),
	RunCommand(String),
	Delay,
	DebugPrint(String),
	DbusMethodCall
	{
		destination: String,
		path: String,
		interface: String,
		method: String,
		arguments: Option<Vec<String>>
	}
}

pub enum MacroSignal
{
	Stop,
	ResetCount
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Macro
{
	pub activation_type: ActivationType,
	pub theme: Option<String>,
	pub steps: Vec<Step>
}

impl Macro
{
	/// Convienience function for creating a new single-step macro from a single action
	pub fn from_action(action: Action) -> Self
	{
		Self
		{
			activation_type: ActivationType::Singular,
			theme: None,
			steps: vec![Step
			{
				action,
				duration: 5 // TODO actually think about what is sensible here
			}]
		}
	}

	/// Gets the number of times this macro should run (None for unlimited)
	pub fn execution_count(&self) -> Option<u32>
	{
		match self.activation_type
		{
			ActivationType::Singular => Some(1),
			ActivationType::Repeat(count) => Some(count),
			ActivationType::HoldToRepeat
				| ActivationType::Toggle => None
		}
	}

	/// Executes the macro by running all of it's steps in turn.
	///
	/// The macro will run until it's configured `execution_count()` is reached
	/// at which point is_finished will be set to true.
	pub fn execute(
		&self,
		rx: Receiver<MacroSignal>,
		window_system: Sender<WindowSystemSignal>,
		dbus: Sender<DBusSignal>,
		is_finished: Arc<AtomicBool>)
	{
		let mut count = self.execution_count();
		let mut i = 0;

		while count.is_none() || i < count.unwrap()
		{
			i += 1;

			self.steps
				.iter()
				.for_each(|step| step.execute(&window_system, &dbus));

			match rx.try_recv()
			{
				Ok(MacroSignal::ResetCount) => count = self.execution_count(),
				Ok(MacroSignal::Stop)
					| Err(TryRecvError::Disconnected) => break,
				Err(TryRecvError::Empty) => ()
			}
		}

		is_finished.store(true, Ordering::Relaxed);
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step
{
	action: Action,
	duration: u64
}

impl Step
{
	fn execute(&self, window_system: &Sender<WindowSystemSignal>, dbus: &Sender<DBusSignal>)
	{
		match &self.action
		{
			Action::Delay => std::thread::sleep(Duration::from_millis(self.duration)),

			Action::MouseClick(button) => window_system
				.send(WindowSystemSignal::SendClick(*button))
				.unwrap_or(()),

			Action::KeyPress(keysequence) => window_system
				.send(WindowSystemSignal::SendKeyCombo(keysequence.clone()))
				.unwrap_or(()),

			Action::DebugPrint(message) => println!("{}", message),

			Action::RunCommand(command) =>
			{
				Command::new(env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()))
					.arg("-c")
					.arg(command)
					.stdin(Stdio::null())
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.spawn();
			},

			Action::DbusMethodCall { destination, path, interface, method, arguments } =>
			{
				if let Ok(message) = zbus::Message::method(
					None,
					Some(destination),
					path,
					Some(interface),
					method,
					arguments)
				{
					dbus.send(DBusSignal::SendMessage(message));
				}
			}
		};
	}
}
