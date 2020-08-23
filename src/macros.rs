use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use serde::{Serialize, Deserialize};

use crate::SharedState;
use crate::windowsystem::WindowSystem;

#[derive(Serialize, Deserialize)]
enum MouseButton
{
	Left,
	Middle,
	Right
}

#[derive(Serialize, Deserialize)]
enum ActivationType
{
	Singular,
	Repeat(u32),
	HoldToRepeat,
	Toggle
}

#[derive(Serialize, Deserialize)]
enum Action
{
	MouseClick(MouseButton),
	KeyPress(String),
	RunCommand(String)
}

#[derive(Serialize, Deserialize)]
struct Step
{
	action: Action,
	duration: u64,
	delay: u64
}

impl Step
{
	fn execute(&self, window_system: &dyn WindowSystem)
	{
		std::thread::sleep(Duration::from_millis(self.delay));

		match &self.action
		{
			Action::MouseClick(button) => 
			{

			},
			Action::KeyPress(keysequence) => 
			{

			},
			Action::RunCommand(command) => 
			{

			}
		};
	}
}

#[derive(Serialize, Deserialize)]
pub struct Macro
{
	activation_type: ActivationType,
	theme: Option<String>,
	steps: Vec<Step>
}

pub enum Signal
{
	Stop
}

fn macro_thread(
	state: Arc<SharedState>, 
	macro_: Macro, 
	signal_receiver: Receiver<Signal>, 
	count: Option<u32>)
{
	let mut i = 0;

	while count.is_none() || i < count.unwrap()
	{
		i += 1;

		macro_
			.steps
			.iter()
			.for_each(|step| step.execute(state.window_system.as_ref()));

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
