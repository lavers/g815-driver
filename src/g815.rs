use hidapi::HidApi;

static VID: u16 = 0x046d;
static PID: u16 = 0xc33f;

static PREAMBLE: u16 = 0x11ff;
static COMMAND_ACTIVATE_MODE: u16 = 0x0b1a; // followed by bitmask of mode key
static COMMAND_SET_13: u16 = 0x106a; // followed by r, g, b, [13 keycodes]
static COMMAND_SET_4: u16 = 0x101a; // followed by keycode, r, g, b, [ff terminator if < 4]
static COMMAND_SET_EFFECT: u16 = 0x0f1a; // followed by group, effect, r, g, b, [period h..l], [00..00..01]
static COMMAND_COMMIT: u16 = 0x107a;
static COMMAND_MARK_START: u16 = 0x083a; // usually before sending group of effects
static COMMAND_MARK_END: u16 = 0x081a; // usually after sending effects
static COMMAND_SET_MACRO_RECORD_MODE: u16 = 0x0c0a; // followed by 00 or 01 for in/out of record mode
static COMMAND_SET_CONTROL_MODE: u16 = 0x111a; // 01 for hardware, 02 for software

static INTERRUPT_G_KEY_BITMASK: u16 = 0x0a00; // 00 [bitmask]
static INTERRUPT_MODE_KEY_BITMASK: u16 = 0x0b00; // 00 [bitmask]
static INTERRUPT_MACRO_KEY_BITMASK: u16 = 0x0c00; // 00 [bitmask]
static INTERRUPT_BRIGHTNESS_LEVEL: u16 = 0x0d00; // followed by 00 [brightness level as percentage]

fn test()
{
	let hidapi = HidApi::new().unwrap();
	let keyboard = hidapi.open(VID, PID);
}
