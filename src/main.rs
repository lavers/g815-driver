use hidapi::HidApi;

mod x11i;
mod g815;

fn main()
{
	let hidapi = HidApi::new().unwrap();
	let mut kb = g815::G815Keyboard::new(&hidapi).unwrap();

	println!("Firmware Version: {}\nBootloader Version: {}", 
			 &kb.firmware_version().unwrap(),
			 &kb.bootloader_version().unwrap());

	let gkey_capability = kb.capability_data(g815::Capability::GKeys);
	println!("GKey capability data: {:x?}", gkey_capability.unwrap());

	let mkey_capability = kb.capability_data(g815::Capability::ModeKeys);
	println!("MKey capability data: {:x?}", mkey_capability.unwrap());

	let gkey_capability = kb.capability_data(g815::Capability::GameModeKey);
	println!("game mode key capability data: {:x?}", gkey_capability.unwrap());

	println!("now attempting to enter software mode");
	kb.take_control();
	println!("should now be in software mode. waiting, then entering macro record mode");

	std::thread::sleep(std::time::Duration::from_millis(2000));
	println!("now in record mode");
	kb.set_macro_record_mode(g815::MacroRecordMode::Recording);
	std::thread::sleep(std::time::Duration::from_millis(2000));
	kb.set_macro_record_mode(g815::MacroRecordMode::Default);
	println!("out of record mode");

	println!("now watching for interrupts, try some keys!");
	kb.interrupt_watch(std::time::Duration::new(15, 0));
	println!("done watching for interrupts");
	kb.release_control();
	println!("should now be back in hardware mode.");

	let x11i = x11i::X11Interface::new();
	let active_window = x11i.get_active_window_info().unwrap();

	println!(
		"Active Window:\n\tPID: {}\n\tTitle: {}\n\tExecutable: {}", 
		active_window.pid.unwrap_or(0),
		active_window.title.unwrap_or("unknown".to_string()),
		active_window.executable.unwrap_or("unknown".to_string()));
}
