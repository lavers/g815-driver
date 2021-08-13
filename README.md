#  g815-driver 

An early work-in-progress linux driver for the Logitech G815 keyboard. Enables macro keys and application-aware lighting control with a yaml config file. 

Absolutely no warranty of any kind is provided. I take no responsibility if your keyboard breaks (or becomes sentient and endeavours to end all life on earth).

## Setup

Copy config.default.yml to ~/.config/g815d/config.yml, then run `cargo run` in the project directory.  For debugging, run with `RUST_LOG=debug cargo run` or `RUST_LOG=trace` (trace will be very verbose)

## Usage

g815-driver is currently controlled only by the config.yml file. This file is watched whilst the program is running, and will live-reload your configuration if you make any changes to it. When changing the config file, keep an eye on the console as it will print errors if your changes cannot be parsed/read for any reason. 

The driver applies named "profiles" to your keyboard based on the currently matched window. A profile can contain a theme, game mode key lists, mode options and macro key bindings. There must be a profile named 'default'. 

### Profiles

Profiles can contain (all of these keys are optional):
* `conditions` - the conditions required to enter this mode
* `theme` - the theme applied when this profile becomes active
* `gkey_sets` - the named gkey sets to apply
* `gkeys` - gkey bindings specific to this mode
* `game_mode_keys` - list of keys to be disabled when game mode is active in this profile
* `modes` - map of mode number to mode profile

Mode profiles are mostly the same as normal profiles, except they have no `game_mode_keys`, `modes` or `conditions`.

Conditions are all based on the current active window as reported by X11. All keys are optional, but at least one must be specified. All will be interpreted as regexes. All specified conditions must match for the profile to be activated. Conditions are specified:


```
conditions:
	title: <window title regex>
	executable: <the full path to the binary of the currently active window>
	class: <the active window class>
	class_name: <the active window class name>
```

Profiles are specified like so:

```
profiles:
	my_theme:
		conditions:
		theme: <theme name>
		game_mode_keys:
			- left_meta
			- right_meta
			- ctrl
			- alt
		gkeys:
			<gkey number>: <macro name or action>
		modes:
			<mode key number>:
				theme: <theme name>
				gkeys:
					<gkey number>: <macro name or action>
```

### Macros

The `macros` key stores your named macros. Macros have an activation type and a list of steps to take when activated. A step is an action and an optional duration (or delay depending on the action).

Activation types are:
* `singular` - steps are run once per press
* `repeat` - steps are run X times per press
* `hold_to_repeat` - steps are run in a loop whilst the key is held
* `toggle` - pressing the key activates an infinite loop repeating your steps, pressing again will stop it

Step actions are:
* `mouse_click` - simulate a mouse button press
	* argument is the mouse button, either: `left`, `middle` or `right`
	* duration is the time to hold the button for
* `key_press` - simulate a key sequence press
	* argument is the key sequence, as X11 keysym names, separated by `+`.
	`alt` is aliased to `Alt_L`, `ctrl` to `Control_L` etc for convenience.
		* examples: `ctrl+c`, `ctrl+shift+s`, `win+l`, etc
	* duration is the time to hold the keys for
* `run_command` - run a command
	* argument is the shell command, passed to `/bin/sh -c`
	* duration is ignored
* `delay` - wait X milliseconds before continuing with the next step
	* no argument
	* duration is the delay
* `debug_print` - print to stdout (for development)
	* argument is the string
* `dbus_method_call` - send a dbus message
	* duration ignored
	* dbus example (takes a screenshot with Flameshot) 
		```
		action:
			dbus_method_call:
				destination:
					destination: org.flameshot.Flameshot
					path: /
					interface: org.flameshot.Flameshot
					method: captureScreen
		```

A macro is defined like so:
```
macros:
	macro_name: 
		activation_type: <an activation type from above>
		steps:
			- action:
				<a step action from above>: <action argument(s)>
			  duration: <duration of the action>
			- action:
				<another action>: <another argument>
			  duration: 123
			...etc
```

### Themes

The `themes` key stores your named themes. A theme can be either a list of `ColorAssignment`s or an `EffectConfiguration`. Effect configurations are detailed in src/device/rgb.rs. Color assignments are simpler, you specify a color and a list of keys to apply it to (`KeySelection`). Themes can be specified like so:
```
themes:
	my_theme:
		- color: ff0000
		  keys:
		    - single: a
			- keygroup: my-keygroup-name
			- multiple: [b, c, d]
		- color: 00ff00
		  keys:
			- single: e
```
Colors should always be in full-length (6 characters) hex format.

### Keygroups

`keygroups` are for easily selecting multiple keys with a single name. Several standard keygroups are already defined in the default config, and you can add more.

### Gkey Sets

`gkey_sets` are for re-using common collections of macro key assignments across multiple modes and themes, without having to redefine them every time.They are named sets of key bindings to either a single action, or a macro name.

They are defined like so:
```
gkey_sets:
	set_name:
		<gkey number>:
			key_press: "ctrl+c"
		<gkey number>:
			macro: <macro name>
```

## Next steps

* allow profile switching with cli commands
* make a proper startup script / background daemon

## Known issues

* app will sometimes crash when reading the active window if the window no longer exists
* if the app crashes, the media buttons/volume wheel will no longer work unless the keyboard is unplugged and plugged back in again