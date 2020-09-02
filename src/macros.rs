use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;
use std::process::{Command, Stdio};
use std::env;

use serde::{Serialize, Deserialize};

use crate::SharedState;
use crate::windowsystem::MouseButton;
use crate::device::DeviceThreadSignal;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Macro
{
	pub activation_type: ActivationType,
	pub theme: Option<String>,
	pub steps: Vec<Step>
}


#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActivationType
{
	#[serde(rename = "singular")]
	Singular,
	#[serde(rename = "repeat")]
	Repeat(u32),
	#[serde(rename = "hold_to_repeat")]
	HoldToRepeat,
	#[serde(rename = "toggle")]
	Toggle
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step
{
	action: Action,
	duration: u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action
{
	#[serde(rename = "mouse_click")]
	MouseClick(MouseButton),
	#[serde(rename = "key_press")]
	KeyPress(String),
	#[serde(rename = "run_command")]
	RunCommand(String),
	#[serde(rename = "delay")]
	Delay,
	#[serde(rename = "debug_print")]
	DebugPrint(String)
}

impl Step
{
	fn execute(&self, state: &Arc<SharedState>)
	{
		match &self.action
		{
			Action::Delay => std::thread::sleep(Duration::from_millis(self.duration)),
			Action::MouseClick(button) => state.window_system.send_mouse_click(*button),
			Action::KeyPress(keysequence) => state.window_system.send_key_combo_press(keysequence),
			Action::DebugPrint(message) => println!("{}", message),
			Action::RunCommand(command) => 
			{
				Command::new(env::var_os("SHELL").unwrap_or("/bin/sh".into()))
					.arg("-c")
					.arg(command)
					.stdin(Stdio::null())
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.spawn();
			}
		};
	}
}

pub enum Signal
{
	Stop,
	ResetCount
}

impl Macro
{
	pub fn from_action(action: Action) -> Self
	{
		Macro
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

	pub fn execution_thread(
		self,
		state: Arc<SharedState>, 
		signal_receiver: Receiver<Signal>,
		device_thread_tx: Sender<DeviceThreadSignal>,
		macro_id: (u8, u8))
	{
		let mut count = self.execution_count();
		let mut i = 0;

		while count.is_none() || i < count.unwrap()
		{
			i += 1;

			self.steps
				.iter()
				.for_each(|step| step.execute(&state));

			match signal_receiver.try_recv()
			{
				Ok(Signal::ResetCount) => count = self.execution_count(),
				Ok(Signal::Stop) 
					| Err(TryRecvError::Disconnected) => break,
				Err(TryRecvError::Empty) => continue
			}
		}

		device_thread_tx.send(DeviceThreadSignal::MacroFinished(macro_id));
	}
}
