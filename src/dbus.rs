use std::sync::mpsc::{Sender, Receiver, TryRecvError};
use std::time::Duration;
use std::thread;
use std::convert::TryInto;

use zbus::{Connection, ObjectServer, dbus_interface};
use zbus::fdo::{DBusProxy, RequestNameFlags};

use crate::MainThreadSignal;

struct ServerInterface;

#[dbus_interface(name = "rs.lave.g815_driver")]
impl ServerInterface
{
	pub fn test(&mut self) -> String
	{
		log::debug!("test was called");
		"test".into()
	}
}

pub enum DBusSignal
{
	Shutdown,
	SendMessage(zbus::Message)
}

pub struct Server
{
	tx: Sender<MainThreadSignal>,
	rx: Receiver<DBusSignal>,
	proxy: DBusProxy<'static>,
	connection: Connection,
	server: ObjectServer<'static>
}

impl Server
{
	const BUS_NAME: &'static str = "rs.lave.g815_driver";
	const BUS_PATH: &'static str = "/rs/lave/g815_driver";

	pub fn new(tx: Sender<MainThreadSignal>, rx: Receiver<DBusSignal>) -> Self
	{
		let handshake = zbus::handshake::ClientHandshake::new_session_nonblock().unwrap();
		let authenticated_socket = handshake.blocking_finish().unwrap();
		let connection = zbus::Connection::new_authenticated_unix(authenticated_socket);

		let proxy = DBusProxy::new(&connection).unwrap();
		let name = proxy.hello().unwrap();

		connection.set_unique_name(name).unwrap();
		proxy.request_name(Self::BUS_NAME, RequestNameFlags::ReplaceExisting.into()).unwrap();

		let mut server = ObjectServer::new(&connection);
		let interface = ServerInterface {};

		server.at(&Self::BUS_PATH.try_into().unwrap(), interface).unwrap();

		Self
		{
			tx,
			rx,
			proxy,
			server,
			connection
		}
	}

	pub fn run(&mut self)
	{
		loop
		{
			match self.rx.try_recv()
			{
				Ok(DBusSignal::Shutdown)
					| Err(TryRecvError::Disconnected) => break,

				Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),

				Ok(DBusSignal::SendMessage(message)) =>
				{
					if let Err(error) = self.connection.send_message(message)
					{
						log::warn!("failed to send dbus message ({:#?})", error);
					}
				}
			}

			match self.server.try_handle_next()
			{
				Err(zbus::Error::Io(io_error)) =>
				{
					if io_error.kind() != std::io::ErrorKind::WouldBlock
					{
						log::warn!("dbus io error = {:?}", io_error);
					}
				},
				Err(error) =>
				{
					log::warn!("incoming dbus message not handled = {:?}", error);
				},
				_ => ()
			}
		}

		self.server.remove::<ServerInterface>(&Self::BUS_PATH.try_into().unwrap());
		self.proxy.release_name(Self::BUS_NAME);
	}
}
