use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::process::{Command, Stdio};
use std::env;

use serde::{Serialize, Deserialize};

use crate::SharedState;
use crate::windowsystem::MouseButton;

#[derive(Serialize, Deserialize, Debug)]
pub struct Macro
{
	activation_type: ActivationType,
	theme: Option<String>,
	steps: Vec<Step>
}

#[derive(Serialize, Deserialize, Debug)]
enum ActivationType
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

#[derive(Serialize, Deserialize, Debug)]
struct Step
{
	action: Action,
	duration: u64
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Action
{
	#[serde(rename = "mouse_click")]
	MouseClick(MouseButton),
	#[serde(rename = "key_press")]
	KeyPress(String),
	#[serde(rename = "run_command")]
	RunCommand(String),
	#[serde(rename = "delay")]
	Delay
}

impl Step
{
	fn execute(&self, state: &Arc<RwLock<SharedState>>)
	{
		match &self.action
		{
			Action::Delay => std::thread::sleep(Duration::from_millis(self.duration)),

			Action::MouseClick(button) => state.read().unwrap().window_system
				.send_mouse_click(*button),

			Action::KeyPress(keysequence) => state.read().unwrap().window_system
				.send_key_combo_press(keysequence),

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
	Stop
}

fn macro_thread(
	state: Arc<RwLock<SharedState>>, 
	macro_: Macro, 
	signal_receiver: Receiver<Signal>, 
	count: Option<u32>)
{
	let mut i = 0;

	while count.is_none() || i < count.unwrap()
	{
		i += 1;

		macro_.steps
			.iter()
			.for_each(|step| step.execute(&state));

		match signal_receiver.try_recv()
		{
			Ok(signal) => match signal
			{
				Signal::Stop => break
			},
			Err(TryRecvError::Empty) => continue,
			Err(TryRecvError::Disconnected) => break
		}
	}
}
