mod x11i;
mod g815;

fn main()
{
	loop
	{
		let x11i = x11i::X11Interface::new();
		let active_window = x11i.get_active_window().unwrap();

		let active_pid = x11i.get_window_pid(active_window).unwrap();
		let process_commandline = match active_pid 
		{
			Some(pid) => std::fs::read_to_string(format!("/proc/{}/cmdline", pid)).unwrap(),
			None => "unknown, no pid defined".to_string()
		};

		println!("current active window id: {}", active_window);
		println!("active window pid: {}", active_pid.unwrap_or(0));
		println!("active window command line: {}", process_commandline);
		println!("active window name: {}", x11i
			.get_window_name(active_window)
			.unwrap()
			.unwrap_or("[unknown]".to_string()));

		std::thread::sleep(std::time::Duration::new(5, 0));
	}
}
