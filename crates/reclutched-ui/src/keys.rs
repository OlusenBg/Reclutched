//! Watches the overlay key: hold it to show the overlay, let go to hide it.
//!
//! A Wayland app cannot see keys while the game has the focus, so we read the keyboards
//! directly from `/dev/input` (this needs your user to be in the `input` group). Every key
//! except the one you chose is dropped right where it is read: nothing else is looked at,
//! stored, logged or sent anywhere. The key is not blocked either, the game still gets it.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
}

/// Whether we can read any keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAccess {
    Ok,
    /// Keyboards exist but we are not allowed to read them (not in the `input` group).
    NoPermission,
    NoKeyboards,
}

const ACCESS_OK: u8 = 0;
const ACCESS_NO_PERMISSION: u8 = 1;
const ACCESS_NO_KEYBOARDS: u8 = 2;

/// The keys you can pick, by name, with their Linux key codes.
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

/// The key code for a name from [`KEYS`] (not case sensitive).
pub fn key_code(name: &str) -> Option<u16> {
    KEYS.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        .map(|&(_, code)| code)
}

pub struct KeyWatcher {
    events: mpsc::Receiver<KeyEvent>,
    wanted: Arc<AtomicU16>,
    access: Arc<AtomicU8>,
}

impl KeyWatcher {
    /// Starts watching for `key` (a name from [`KEYS`]). `None` for an unknown name.
    pub fn start(key: &str) -> Option<Self> {
        let code = key_code(key)?;
        let wanted = Arc::new(AtomicU16::new(code));
        let access = Arc::new(AtomicU8::new(ACCESS_NO_KEYBOARDS));
        let (tx, events) = mpsc::channel();
        {
            let (wanted, access) = (wanted.clone(), access.clone());
            std::thread::spawn(move || manage_readers(&tx, &wanted, &access));
        }
        Some(Self {
            events,
            wanted,
            access,
        })
    }

    /// Switches to another key (a name from [`KEYS`]); false for an unknown name.
    pub fn set_key(&self, key: &str) -> bool {
        match key_code(key) {
            Some(code) => {
                self.wanted.store(code, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn drain(&self) -> Vec<KeyEvent> {
        self.events.try_iter().collect()
    }

    pub fn access(&self) -> KeyAccess {
        match self.access.load(Ordering::Relaxed) {
            ACCESS_OK => KeyAccess::Ok,
            ACCESS_NO_PERMISSION => KeyAccess::NoPermission,
            _ => KeyAccess::NoKeyboards,
        }
    }
}

/// Every few seconds: look for keyboards and start a reader thread for each new one.
fn manage_readers(tx: &mpsc::Sender<KeyEvent>, wanted: &Arc<AtomicU16>, access: &Arc<AtomicU8>) {
    let watched: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    loop {
        let found = std::fs::read_to_string("/proc/bus/input/devices")
            .map(|text| parse_keyboards(&text))
            .unwrap_or_default();
        let mut denied = 0;
        for event in &found {
            let path = PathBuf::from(format!("/dev/input/{event}"));
            if watched.lock().is_ok_and(|w| w.contains(&path)) {
                continue;
            }
            match File::open(&path) {
                Ok(file) => {
                    if let Ok(mut w) = watched.lock() {
                        w.insert(path.clone());
                    }
                    let (tx, wanted, watched) = (tx.clone(), wanted.clone(), watched.clone());
                    std::thread::spawn(move || {
                        pump(file, &wanted, &tx);
                        // The keyboard went away (or the reader stopped): look for it again later.
                        if let Ok(mut w) = watched.lock() {
                            w.remove(&path);
                        }
                    });
                }
                Err(_) => denied += 1,
            }
        }
        let open = watched.lock().map_or(0, |w| w.len());
        let state = if found.is_empty() {
            ACCESS_NO_KEYBOARDS
        } else if open == 0 && denied > 0 {
            ACCESS_NO_PERMISSION
        } else {
            ACCESS_OK
        };
        access.store(state, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(4));
    }
}

/// Size of one `struct input_event`: a timestamp (two longs), type, code and value.
const EVENT_SIZE: usize = 2 * std::mem::size_of::<usize>() + 8;
const EV_KEY: u16 = 1;

/// Reads events until the device closes, and passes on presses and releases of the wanted key.
/// (Auto-repeat, value 2, is ignored, so holding the key does not flood us.)
fn pump(mut device: impl Read, wanted: &AtomicU16, tx: &mpsc::Sender<KeyEvent>) {
    let mut buf = [0u8; EVENT_SIZE];
    while device.read_exact(&mut buf).is_ok() {
        let (kind, code, value) = parse_event(&buf);
        if kind != EV_KEY || code != wanted.load(Ordering::Relaxed) {
            continue;
        }
        let event = match value {
            1 => KeyEvent::Pressed,
            0 => KeyEvent::Released,
            _ => continue,
        };
        if tx.send(event).is_err() {
            return;
        }
    }
}

fn parse_event(buf: &[u8; EVENT_SIZE]) -> (u16, u16, i32) {
    let at = 2 * std::mem::size_of::<usize>();
    let kind = u16::from_ne_bytes([buf[at], buf[at + 1]]);
    let code = u16::from_ne_bytes([buf[at + 2], buf[at + 3]]);
    let value = i32::from_ne_bytes([buf[at + 4], buf[at + 5], buf[at + 6], buf[at + 7]]);
    (kind, code, value)
}

/// The `eventN` names of the keyboards in `/proc/bus/input/devices`: devices with a `kbd`
/// handler that can auto-repeat (so mice and power buttons are left out).
pub fn parse_keyboards(text: &str) -> Vec<String> {
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
        // Bit 20 is EV_REP: only keyboards repeat keys.
        if words.contains(&"kbd") && ev_bits & (1 << 20) != 0 {
            found.extend(
                words
                    .iter()
                    .filter(|w| w.starts_with("event"))
                    .map(|w| w.to_string()),
            );
        }
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

    fn pumped(wanted: u16, events: &[(u16, u16, i32)]) -> Vec<KeyEvent> {
        let bytes: Vec<u8> = events
            .iter()
            .flat_map(|&(k, c, v)| event_bytes(k, c, v))
            .collect();
        let (tx, rx) = mpsc::channel();
        pump(std::io::Cursor::new(bytes), &AtomicU16::new(wanted), &tx);
        drop(tx);
        rx.iter().collect()
    }

    #[test]
    fn press_and_release_of_the_wanted_key_come_through_repeats_and_others_do_not() {
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
            pumped(tab, &events),
            vec![KeyEvent::Pressed, KeyEvent::Released]
        );
    }

    #[test]
    fn a_half_event_at_the_end_is_ignored() {
        let mut bytes = event_bytes(EV_KEY, 15, 1);
        bytes.extend([1, 2, 3]);
        let (tx, rx) = mpsc::channel();
        pump(std::io::Cursor::new(bytes), &AtomicU16::new(15), &tx);
        drop(tx);
        assert_eq!(rx.iter().collect::<Vec<_>>(), vec![KeyEvent::Pressed]);
    }

    #[test]
    fn key_names_map_to_codes_and_are_unique() {
        assert_eq!(key_code("Tab"), Some(15));
        assert_eq!(key_code(" esc "), Some(1));
        assert_eq!(key_code("f12"), Some(88));
        assert_eq!(key_code("nope"), None);
        let codes: HashSet<_> = KEYS.iter().map(|&(_, c)| c).collect();
        let names: HashSet<_> = KEYS.iter().map(|&(n, _)| n).collect();
        assert_eq!(codes.len(), KEYS.len());
        assert_eq!(names.len(), KEYS.len());
    }

    #[test]
    fn only_keyboards_are_picked_from_the_device_list() {
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

I: Bus=0003 Vendor=1b1c Product=1b2e Version=0111
N: Name=\"Corsair Gaming Keyboard\"
H: Handlers=sysrq kbd event7 leds event8
B: PROP=0
B: EV=120013
";
        assert_eq!(parse_keyboards(text), vec!["event3", "event7", "event8"]);
        assert!(parse_keyboards("").is_empty());
    }
}
