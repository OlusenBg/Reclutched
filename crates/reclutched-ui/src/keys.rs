//! Watches the overlay buttons: hold one to show the overlay, let go to hide it. You can set a
//! keyboard key and a controller button; either one works.
//!
//! A Wayland app cannot see keys while the game has the focus, so we read the keyboards and
//! controllers directly from `/dev/input` (this needs your user to be in the `input` group).
//! Everything except the key and the button you chose is dropped right where it is read: nothing
//! else is looked at, stored, logged or sent anywhere. Nothing is blocked either, the game still
//! gets every key and button.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Where an input came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Keyboard,
    Controller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed(Source),
    Released(Source),
    /// The next controller button, after [`KeyWatcher::capture_button`].
    Captured(u16),
}

/// Whether we can read any keyboard or controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAccess {
    Ok,
    /// Devices exist but we are not allowed to read them (not in the `input` group).
    NoPermission,
    NoKeyboards,
}

const ACCESS_OK: u8 = 0;
const ACCESS_NO_PERMISSION: u8 = 1;
const ACCESS_NO_KEYBOARDS: u8 = 2;

/// Common keyboard keys, by name, with their Linux key codes. (Any other key works too; it is
/// stored as `code:<number>`.)
pub const KEYS: &[(&str, u16)] = &[
    ("Tab", 15),
    ("Esc", 1),
    ("Caps Lock", 58),
    ("`", 41),
    ("Space", 57),
    ("Left Shift", 42),
    ("Right Shift", 54),
    ("Left Ctrl", 29),
    ("Right Ctrl", 97),
    ("Left Alt", 56),
    ("F1", 59),
    ("F2", 60),
    ("F3", 61),
    ("F4", 62),
    ("F5", 63),
    ("F6", 64),
    ("F7", 65),
    ("F8", 66),
    ("F9", 67),
    ("F10", 68),
    ("F11", 87),
    ("F12", 88),
];

/// Controller buttons by their place on the pad, with a hint for the usual brands. The D-pad
/// is here too: pads report it as buttons or as an axis, both end up as these codes.
pub const PAD_BUTTONS: &[(&str, u16)] = &[
    ("A / Cross (bottom)", 0x130),
    ("B / Circle (right)", 0x131),
    ("Y / Triangle (top)", 0x133),
    ("X / Square (left)", 0x134),
    ("LB / L1", 0x136),
    ("RB / R1", 0x137),
    ("Select / Back / Share", 0x13a),
    ("Start / Menu / Options", 0x13b),
    ("Guide / PS button", 0x13c),
    ("Left stick click (L3)", 0x13d),
    ("Right stick click (R3)", 0x13e),
    ("D-pad up", DPAD_UP),
    ("D-pad down", DPAD_DOWN),
    ("D-pad left", DPAD_LEFT),
    ("D-pad right", DPAD_RIGHT),
];

const DPAD_UP: u16 = 0x220;
const DPAD_DOWN: u16 = 0x221;
const DPAD_LEFT: u16 = 0x222;
const DPAD_RIGHT: u16 = 0x223;

/// The code of a saved binding: a name from [`KEYS`] or [`PAD_BUTTONS`] (not case sensitive) or
/// `code:<number>`. Empty or unknown means "not set".
pub fn binding_code(text: &str) -> Option<u16> {
    let text = text.trim();
    if let Some(number) = text.strip_prefix("code:") {
        return number.trim().parse().ok().filter(|&c| c != 0);
    }
    KEYS.iter()
        .chain(PAD_BUTTONS)
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|&(_, code)| code)
}

/// How a binding is saved: the plain name for the common keys, `code:<number>` for the rest.
pub fn binding_text(code: u16) -> String {
    match KEYS.iter().find(|&&(_, c)| c == code) {
        Some(&(name, _)) => name.to_string(),
        None => format!("code:{code}"),
    }
}

/// The keyboard code for a key press seen by the app. Desktops number keys as the keyboard's own
/// (evdev) code plus 8; `None` for the numbers below that, which are not real keys.
pub fn code_from_desktop_keycode(keycode: u32) -> Option<u16> {
    u16::try_from(keycode.checked_sub(8)?)
        .ok()
        .filter(|&c| c != 0)
}

/// What to show for a saved keyboard binding.
pub fn key_label(saved: &str) -> String {
    match binding_code(saved) {
        None => "Not set".to_string(),
        Some(code) => KEYS
            .iter()
            .find(|&&(_, c)| c == code)
            .map_or_else(|| format!("Key {code}"), |&(name, _)| name.to_string()),
    }
}

/// What to show for a saved controller binding.
pub fn button_label(saved: &str) -> String {
    match binding_code(saved) {
        None => "Not set".to_string(),
        Some(code) => PAD_BUTTONS
            .iter()
            .find(|&&(_, c)| c == code)
            .map_or_else(|| format!("Button {code}"), |&(name, _)| name.to_string()),
    }
}

/// How many keyboards and controllers we are reading right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Devices {
    pub keyboards: usize,
    pub controllers: usize,
}

struct Shared {
    /// 0 = not set.
    key: AtomicU16,
    button: AtomicU16,
    /// The next controller button press is reported as [`KeyEvent::Captured`].
    capture: AtomicBool,
    access: AtomicU8,
    keyboards: AtomicU16,
    controllers: AtomicU16,
}

pub struct KeyWatcher {
    events: mpsc::Receiver<KeyEvent>,
    shared: Arc<Shared>,
}

impl KeyWatcher {
    /// Starts watching for the saved key and button (see [`binding_code`]; either may be empty).
    pub fn start(key: &str, button: &str) -> Self {
        let shared = Arc::new(Shared {
            key: AtomicU16::new(binding_code(key).unwrap_or(0)),
            button: AtomicU16::new(binding_code(button).unwrap_or(0)),
            capture: AtomicBool::new(false),
            access: AtomicU8::new(ACCESS_NO_KEYBOARDS),
            keyboards: AtomicU16::new(0),
            controllers: AtomicU16::new(0),
        });
        let (tx, events) = mpsc::channel();
        {
            let shared = shared.clone();
            std::thread::spawn(move || manage_readers(&tx, &shared));
        }
        Self { events, shared }
    }

    /// Switches to other bindings (saved text, see [`binding_code`]).
    pub fn set_bindings(&self, key: &str, button: &str) {
        self.shared
            .key
            .store(binding_code(key).unwrap_or(0), Ordering::Relaxed);
        self.shared
            .button
            .store(binding_code(button).unwrap_or(0), Ordering::Relaxed);
    }

    /// The next controller button you press is sent as [`KeyEvent::Captured`] (once).
    pub fn capture_button(&self) {
        self.shared.capture.store(true, Ordering::Relaxed);
    }

    pub fn cancel_capture(&self) {
        self.shared.capture.store(false, Ordering::Relaxed);
    }

    pub fn drain(&self) -> Vec<KeyEvent> {
        self.events.try_iter().collect()
    }

    pub fn access(&self) -> KeyAccess {
        match self.shared.access.load(Ordering::Relaxed) {
            ACCESS_OK => KeyAccess::Ok,
            ACCESS_NO_PERMISSION => KeyAccess::NoPermission,
            _ => KeyAccess::NoKeyboards,
        }
    }

    pub fn devices(&self) -> Devices {
        Devices {
            keyboards: usize::from(self.shared.keyboards.load(Ordering::Relaxed)),
            controllers: usize::from(self.shared.controllers.load(Ordering::Relaxed)),
        }
    }
}

/// Every few seconds: look for keyboards and controllers and start a reader thread for each
/// new one.
fn manage_readers(tx: &mpsc::Sender<KeyEvent>, shared: &Arc<Shared>) {
    let watched: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    // Which watched devices are keyboards, to count them apart from the controllers.
    let kinds: Arc<Mutex<Vec<(PathBuf, Source)>>> = Arc::default();
    loop {
        let found = std::fs::read_to_string("/proc/bus/input/devices")
            .map(|text| parse_devices(&text))
            .unwrap_or_default();
        let mut denied = 0;
        for (event, source) in &found {
            let path = PathBuf::from(format!("/dev/input/{event}"));
            if watched.lock().is_ok_and(|w| w.contains(&path)) {
                continue;
            }
            match File::open(&path) {
                Ok(file) => {
                    if let Ok(mut w) = watched.lock() {
                        w.insert(path.clone());
                    }
                    if let Ok(mut k) = kinds.lock() {
                        k.push((path.clone(), *source));
                    }
                    let (tx, shared, watched, kinds, source) = (
                        tx.clone(),
                        shared.clone(),
                        watched.clone(),
                        kinds.clone(),
                        *source,
                    );
                    std::thread::spawn(move || {
                        pump(file, source, &shared, &tx);
                        // The device went away (or the reader stopped): look for it again later.
                        if let Ok(mut w) = watched.lock() {
                            w.remove(&path);
                        }
                        if let Ok(mut k) = kinds.lock() {
                            k.retain(|(p, _)| *p != path);
                        }
                    });
                }
                Err(_) => denied += 1,
            }
        }
        let (keyboards, controllers) = kinds.lock().map_or((0, 0), |k| {
            (
                k.iter().filter(|(_, s)| *s == Source::Keyboard).count(),
                k.iter().filter(|(_, s)| *s == Source::Controller).count(),
            )
        });
        shared.keyboards.store(keyboards as u16, Ordering::Relaxed);
        shared
            .controllers
            .store(controllers as u16, Ordering::Relaxed);
        let open = keyboards + controllers;
        let state = if found.is_empty() {
            ACCESS_NO_KEYBOARDS
        } else if open == 0 && denied > 0 {
            ACCESS_NO_PERMISSION
        } else {
            ACCESS_OK
        };
        shared.access.store(state, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(4));
    }
}

/// Size of one `struct input_event`: a timestamp (two longs), type, code and value.
const EVENT_SIZE: usize = 2 * std::mem::size_of::<usize>() + 8;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
/// The D-pad as an axis (many pads report it this way instead of as buttons).
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
/// Controller buttons start here (`BTN_MISC`), keyboard keys are below.
const BTN_MISC: u16 = 0x100;

/// What a controller event means for us: buttons (and D-pad directions) going down or up. One
/// event can be two changes: sliding the D-pad from left straight to right releases left and
/// presses right.
fn pad_changes(kind: u16, code: u16, value: i32, hat: &mut [i32; 2]) -> Vec<(u16, bool)> {
    match (kind, code) {
        (EV_KEY, c) if c >= BTN_MISC && matches!(value, 0 | 1) => vec![(c, value == 1)],
        (EV_ABS, ABS_HAT0X | ABS_HAT0Y) => {
            let axis = usize::from(code == ABS_HAT0Y);
            let (low, high) = if axis == 0 {
                (DPAD_LEFT, DPAD_RIGHT)
            } else {
                (DPAD_UP, DPAD_DOWN)
            };
            let direction = |v: i32| if v < 0 { low } else { high };
            let (before, now) = (hat[axis], value.signum());
            hat[axis] = now;
            let mut changes = Vec::new();
            if before != 0 && before != now {
                changes.push((direction(before), false));
            }
            if now != 0 && now != before {
                changes.push((direction(now), true));
            }
            changes
        }
        _ => Vec::new(),
    }
}

/// Reads events until the device closes, and passes on presses and releases of the wanted key
/// (keyboards) or button (controllers). Auto-repeat, value 2, is ignored.
fn pump(mut device: impl Read, source: Source, shared: &Shared, tx: &mpsc::Sender<KeyEvent>) {
    let mut buf = [0u8; EVENT_SIZE];
    let mut hat = [0i32; 2];
    while device.read_exact(&mut buf).is_ok() {
        let (kind, code, value) = parse_event(&buf);
        let ok = match source {
            Source::Keyboard => {
                if kind != EV_KEY || code != shared.key.load(Ordering::Relaxed) {
                    continue;
                }
                match value {
                    1 => tx.send(KeyEvent::Pressed(Source::Keyboard)).is_ok(),
                    0 => tx.send(KeyEvent::Released(Source::Keyboard)).is_ok(),
                    _ => true,
                }
            }
            Source::Controller => pad_changes(kind, code, value, &mut hat)
                .into_iter()
                .all(|(code, pressed)| send_pad(shared, tx, code, pressed)),
        };
        if !ok {
            return;
        }
    }
}

/// Passes on one controller change: the capture (a press while capturing) or the wanted button.
/// False when nobody is listening any more.
fn send_pad(shared: &Shared, tx: &mpsc::Sender<KeyEvent>, code: u16, pressed: bool) -> bool {
    if pressed && shared.capture.swap(false, Ordering::Relaxed) {
        return tx.send(KeyEvent::Captured(code)).is_ok();
    }
    if code != shared.button.load(Ordering::Relaxed) {
        return true;
    }
    tx.send(if pressed {
        KeyEvent::Pressed(Source::Controller)
    } else {
        KeyEvent::Released(Source::Controller)
    })
    .is_ok()
}

fn parse_event(buf: &[u8; EVENT_SIZE]) -> (u16, u16, i32) {
    let at = 2 * std::mem::size_of::<usize>();
    let kind = u16::from_ne_bytes([buf[at], buf[at + 1]]);
    let code = u16::from_ne_bytes([buf[at + 2], buf[at + 3]]);
    let value = i32::from_ne_bytes([buf[at + 4], buf[at + 5], buf[at + 6], buf[at + 7]]);
    (kind, code, value)
}

/// The `eventN` names of the keyboards and controllers in `/proc/bus/input/devices`.
/// Keyboards have a `kbd` handler and can auto-repeat (so mice and power buttons are left out);
/// controllers have a joystick handler (`js0`, ...).
pub fn parse_devices(text: &str) -> Vec<(String, Source)> {
    let mut found = Vec::new();
    for block in text.split("\n\n") {
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers="));
        let ev_bits = block
            .lines()
            .find_map(|l| l.strip_prefix("B: EV="))
            .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok());
        let (Some(handlers), Some(ev_bits)) = (handlers, ev_bits) else {
            continue;
        };
        let words: Vec<&str> = handlers.split_whitespace().collect();
        let source = if words.contains(&"kbd") && ev_bits & (1 << 20) != 0 {
            Source::Keyboard // bit 20 is EV_REP: only keyboards repeat keys
        } else if words
            .iter()
            .any(|w| w.starts_with("js") && w[2..].chars().all(|c| c.is_ascii_digit()))
        {
            Source::Controller
        } else {
            continue;
        };
        found.extend(
            words
                .iter()
                .filter(|w| w.starts_with("event"))
                .map(|w| (w.to_string(), source)),
        );
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_bytes(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut b = vec![0u8; 2 * std::mem::size_of::<usize>()]; // the timestamp
        b.extend(kind.to_ne_bytes());
        b.extend(code.to_ne_bytes());
        b.extend(value.to_ne_bytes());
        b
    }

    fn shared(key: u16, button: u16) -> Shared {
        Shared {
            key: AtomicU16::new(key),
            button: AtomicU16::new(button),
            capture: AtomicBool::new(false),
            access: AtomicU8::new(ACCESS_OK),
            keyboards: AtomicU16::new(0),
            controllers: AtomicU16::new(0),
        }
    }

    fn pumped(source: Source, shared: &Shared, events: &[(u16, u16, i32)]) -> Vec<KeyEvent> {
        let bytes: Vec<u8> = events
            .iter()
            .flat_map(|&(k, c, v)| event_bytes(k, c, v))
            .collect();
        let (tx, rx) = mpsc::channel();
        pump(std::io::Cursor::new(bytes), source, shared, &tx);
        drop(tx);
        rx.iter().collect()
    }

    #[test]
    fn keyboard_press_and_release_come_through_repeats_and_other_keys_do_not() {
        let tab = 15;
        let events = [
            (EV_KEY, 30, 1),  // another key
            (EV_KEY, tab, 1), // press
            (EV_KEY, tab, 2), // auto-repeat
            (EV_KEY, tab, 2),
            (0, 0, 0),        // a sync event
            (3, tab, 1),      // not a key event at all
            (EV_KEY, tab, 0), // release
            (EV_KEY, 30, 0),
        ];
        assert_eq!(
            pumped(Source::Keyboard, &shared(tab, 0), &events),
            vec![
                KeyEvent::Pressed(Source::Keyboard),
                KeyEvent::Released(Source::Keyboard)
            ]
        );
    }

    #[test]
    fn a_half_event_at_the_end_is_ignored() {
        let mut bytes = event_bytes(EV_KEY, 15, 1);
        bytes.extend([1, 2, 3]);
        let (tx, rx) = mpsc::channel();
        pump(
            std::io::Cursor::new(bytes),
            Source::Keyboard,
            &shared(15, 0),
            &tx,
        );
        drop(tx);
        assert_eq!(
            rx.iter().collect::<Vec<_>>(),
            vec![KeyEvent::Pressed(Source::Keyboard)]
        );
    }

    #[test]
    fn controller_buttons_work_and_keyboard_keys_are_ignored_on_a_controller() {
        let select = 0x13a;
        let events = [
            (EV_KEY, 0x130, 1), // another button
            (EV_KEY, select, 1),
            (EV_KEY, select, 0),
            (EV_KEY, 15, 1), // a keyboard code: below BTN_MISC, not a button
        ];
        assert_eq!(
            pumped(Source::Controller, &shared(0, select), &events),
            vec![
                KeyEvent::Pressed(Source::Controller),
                KeyEvent::Released(Source::Controller)
            ]
        );
    }

    #[test]
    fn the_dpad_as_an_axis_acts_like_buttons() {
        // Hold left, let go; hold up, then slide to down, let go.
        let events = [
            (EV_ABS, ABS_HAT0X, -1),
            (EV_ABS, ABS_HAT0X, 0),
            (EV_ABS, ABS_HAT0Y, -1),
            (EV_ABS, ABS_HAT0Y, 1),
            (EV_ABS, ABS_HAT0Y, 0),
        ];
        // Listening for left: pressed and released once.
        assert_eq!(
            pumped(Source::Controller, &shared(0, DPAD_LEFT), &events),
            vec![
                KeyEvent::Pressed(Source::Controller),
                KeyEvent::Released(Source::Controller)
            ]
        );
        // Listening for down: only the slide to down and its release.
        assert_eq!(
            pumped(Source::Controller, &shared(0, DPAD_DOWN), &events),
            vec![
                KeyEvent::Pressed(Source::Controller),
                KeyEvent::Released(Source::Controller)
            ]
        );
        // Listening for up: pressed, then released when it slid away.
        assert_eq!(
            pumped(Source::Controller, &shared(0, DPAD_UP), &events),
            vec![
                KeyEvent::Pressed(Source::Controller),
                KeyEvent::Released(Source::Controller)
            ]
        );
    }

    #[test]
    fn capture_reports_the_next_button_once_whatever_it_is() {
        let s = shared(0, 0);
        s.capture.store(true, Ordering::Relaxed);
        let events = [
            (EV_KEY, 0x131, 1),
            (EV_KEY, 0x131, 0),
            (EV_KEY, 0x130, 1), // a second press is not captured
        ];
        assert_eq!(
            pumped(Source::Controller, &s, &events),
            vec![KeyEvent::Captured(0x131)]
        );
        assert!(!s.capture.load(Ordering::Relaxed));
        // The D-pad axis can be captured too.
        s.capture.store(true, Ordering::Relaxed);
        assert_eq!(
            pumped(Source::Controller, &s, &[(EV_ABS, ABS_HAT0X, 1)]),
            vec![KeyEvent::Captured(DPAD_RIGHT)]
        );
    }

    #[test]
    fn saved_bindings_round_trip_and_unknown_ones_are_not_set() {
        assert_eq!(binding_code("Tab"), Some(15));
        assert_eq!(binding_code(" esc "), Some(1));
        assert_eq!(binding_code("f12"), Some(88));
        assert_eq!(binding_code("code:200"), Some(200));
        assert_eq!(binding_code("code:0"), None);
        assert_eq!(binding_code("code:x"), None);
        assert_eq!(binding_code(""), None);
        assert_eq!(binding_code("nope"), None);
        assert_eq!(binding_text(15), "Tab");
        assert_eq!(binding_text(200), "code:200");
        assert_eq!(binding_code(&binding_text(200)), Some(200));
        // The button table's names also work, and show nicely.
        assert_eq!(binding_code("Select / Back / Share"), Some(0x13a));
        assert_eq!(button_label("code:314"), "Select / Back / Share");
        assert_eq!(button_label("code:999"), "Button 999");
        assert_eq!(key_label("code:200"), "Key 200");
        assert_eq!(key_label(""), "Not set");
    }

    #[test]
    fn desktop_key_numbers_are_the_keyboard_codes_plus_eight() {
        // Tab is 15, F7 is 65, the key left of 1 is 41.
        assert_eq!(code_from_desktop_keycode(23), Some(15));
        assert_eq!(binding_text(code_from_desktop_keycode(23).unwrap()), "Tab");
        assert_eq!(binding_text(code_from_desktop_keycode(73).unwrap()), "F7");
        assert_eq!(binding_text(code_from_desktop_keycode(49).unwrap()), "`");
        assert_eq!(code_from_desktop_keycode(8), None);
        assert_eq!(code_from_desktop_keycode(3), None);
        assert_eq!(code_from_desktop_keycode(0), None);
        assert_eq!(code_from_desktop_keycode(70_000), None);
    }

    #[test]
    fn the_name_tables_have_no_duplicates() {
        for table in [KEYS, PAD_BUTTONS] {
            let codes: HashSet<_> = table.iter().map(|&(_, c)| c).collect();
            let names: HashSet<_> = table.iter().map(|&(n, _)| n).collect();
            assert_eq!(codes.len(), table.len());
            assert_eq!(names.len(), table.len());
        }
        assert!(KEYS.iter().all(|&(_, c)| c < BTN_MISC));
        assert!(PAD_BUTTONS.iter().all(|&(_, c)| c >= BTN_MISC));
    }

    #[test]
    fn keyboards_and_controllers_are_picked_from_the_device_list() {
        let text = "\
I: Bus=0011 Vendor=0001 Product=0001 Version=ab41
N: Name=\"AT Translated Set 2 keyboard\"
H: Handlers=sysrq kbd leds event3
B: PROP=0
B: EV=120013

I: Bus=0003 Vendor=046d Product=c077 Version=0111
N: Name=\"USB Optical Mouse\"
H: Handlers=mouse0 event5
B: PROP=0
B: EV=17

I: Bus=0019 Vendor=0000 Product=0001 Version=0000
N: Name=\"Power Button\"
H: Handlers=kbd event1
B: PROP=0
B: EV=3

I: Bus=0003 Vendor=045e Product=028e Version=0110
N: Name=\"Microsoft X-Box 360 pad\"
H: Handlers=event12 js0
B: PROP=0
B: EV=20000b

I: Bus=0003 Vendor=054c Product=0ce6 Version=8111
N: Name=\"DualSense Wireless Controller Touchpad\"
H: Handlers=mouse1 event14
B: PROP=5
B: EV=b
";
        assert_eq!(
            parse_devices(text),
            vec![
                ("event3".to_string(), Source::Keyboard),
                ("event12".to_string(), Source::Controller),
            ]
        );
        assert!(parse_devices("").is_empty());
    }
}
