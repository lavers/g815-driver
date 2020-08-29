use macro_attr::*;
use enum_derive::*;

macro_attr!
{
	#[derive(Copy, Debug, PartialEq, Eq, Clone, EnumFromStr!, IterVariants!(Scancodes))]
	pub enum Scancode 
	{
		// standard usb hid scancodes

		A = 4, 
		B, C, D, E, F, G, H, I, J, K, L, N, M, O, P, Q, R, S, T, U, V, W, X, Y, Z, 
		N1, N2, N3, N4, N5, N6, N7, N8, N9, N0,
		Enter,
		Escape,
		Backspace,
		Tab,
		Space,
		Minus,
		Equals,
		LeftBracket,
		RightBracket,
		USBackslash,
		HashTilde,
		Semicolon,
		Apostrophe,
		Grave,
		Comma,
		Dot,
		Slash,
		CapsLock,
		F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
		PrintScreen,
		ScrollLock,
		Pause,
		Insert,
		Home,
		PageUp,
		Delete,
		End,
		PageDown,
		Right,
		Left,
		Down,
		Up,
		NumLock,
		NumpadDivide, NumpadMultiply, NumpadMinus, NumpadPlus, NumpadEnter,
		Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9, Numpad0,
		NumpadDot,
		Backslash,
		NumpadEquals = 0x67,
		ContextMenu = 0x76, // 62
		Mute = 0x7f,
		LeftControl = 0xe0, // 0x68
		LeftShift,
		LeftAlt,
		LeftMeta,
		RightControl,
		RightShift,
		RightAlt,
		RightMeta,

		// logitech-specific key codes (not real scan codes)

		Light = 0x99,
		G1 = 0xb4,
		G2,
		G3,
		G4,
		G5,
		Logo = 0xd2,
		MediaPrevious = 0x9e,
		MediaNext = 0x9d,
		MediaPlayPause = 0x9b
	}
}

impl Scancode
{
	pub fn to_rgb_id(&self) -> u8
	{
		let id = *self as u8;

		match &self
		{
			// these don't have real scancodes so they're already
			// rgb-only key ids

			Scancode::G1 
				| Scancode::G2 
				| Scancode::G3 
				| Scancode::G4 
				| Scancode::G5 
				| Scancode::MediaPrevious 
				| Scancode::MediaNext 
				| Scancode::MediaPlayPause 
				| Scancode::Logo 
				| Scancode::Light => id,

			Scancode::Mute => 0x9c,
			Scancode::ContextMenu => 0x62,

			Scancode::LeftControl
				| Scancode::LeftShift
				| Scancode::LeftAlt
				| Scancode::LeftMeta
				| Scancode::RightControl
				| Scancode::RightShift
				| Scancode::RightAlt
				| Scancode::RightMeta => id - 0x78,

			_ => id - 0x03
		}
	}
}
