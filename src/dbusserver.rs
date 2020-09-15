use std::sync::Arc;
use std::sync::mpsc::{Sender, Receiver};
use std::time::Duration;
use std::thread;

use crate::{SharedState, MainThreadSignal};

pub struct Server
{
	state: Arc<SharedState>,
	tx: Sender<MainThreadSignal>,
	rx: Receiver<()>
}

impl Server
{
	const BUS_NAME: &'static str = "rs.lave.g815_driver";

	pub fn new(state: Arc<SharedState>, tx: Sender<MainThreadSignal>, rx: Receiver<()>) -> Self
	{
		Self
		{
			state, tx, rx
		}
	}

	fn setup(&self)
	{
		use dbus::channel::MatchingReceiver;

		let connection = self.state.dbus.lock().unwrap();
		let mut crossroads = dbus_crossroads::Crossroads::new();
		connection.request_name(Self::BUS_NAME, false, true, true).unwrap();

		struct Test;

		let interface = crossroads.register("rs.lave.g815_driver.keyboard", |builder|
		{
			builder.method("Test", ("test_arg",), ("test_arg_2",), |context: &mut dbus_crossroads::Context, data: &mut Test, (name,): (String,)| 
			{
				Ok(("test",))
			});
		});

		crossroads.insert("/testpath", &[interface], Test);

		connection.start_receive(
			dbus::message::MatchRule::new_method_call(), 
			Box::new(move |message, connection| 
			{
				crossroads.handle_message(message, connection);
				true
			}));
	}

	pub fn event_loop(&self)
	{
		self.setup();

		while self.rx.try_recv().is_err()
		{
			thread::sleep(Duration::from_millis(100));
			self.state.dbus.lock().unwrap().process(Duration::from_millis(0));
		}

		self.state.dbus.lock().unwrap().release_name(Self::BUS_NAME);
	}
}
