//! The capture keys: one watcher for the screenshot key and one for the clip key, started and
//! stopped by what the settings say.

use reclutched_core::rocketleague::settings::Settings;

use crate::keys::{KeyEvent, KeyWatcher};
use crate::triggers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Screenshot,
    Clip,
}

/// The (key, button) saved for an action, when the action should react at all.
pub fn binding(settings: &Settings, action: Action) -> Option<(String, String)> {
    if !settings.capture_enabled {
        return None;
    }
    let (key, button) = match action {
        Action::Screenshot => (&settings.screenshot_key, &settings.screenshot_button),
        Action::Clip => {
            if !triggers::key_on(settings) {
                return None;
            }
            (&settings.clip_key, &settings.clip_button)
        }
    };
    (!key.is_empty() || !button.is_empty()).then(|| (key.clone(), button.clone()))
}

#[derive(Default)]
pub struct Hotkeys {
    screenshot: Option<KeyWatcher>,
    clip: Option<KeyWatcher>,
}

fn follow(slot: &mut Option<KeyWatcher>, wanted: Option<(String, String)>) {
    match (slot.as_ref(), wanted) {
        (None, Some((key, button))) => *slot = Some(KeyWatcher::start(&key, &button)),
        (Some(watcher), Some((key, button))) => watcher.set_bindings(&key, &button),
        // Not wanted any more: dropping the watcher stops its readers.
        (Some(_), None) => *slot = None,
        (None, None) => {}
    }
}

impl Hotkeys {
    /// Starts, retunes or stops the watchers from the saved settings.
    pub fn sync(&mut self, settings: &Settings) {
        follow(&mut self.screenshot, binding(settings, Action::Screenshot));
        follow(&mut self.clip, binding(settings, Action::Clip));
    }

    /// The keys pressed since the last call.
    pub fn pressed(&self) -> Vec<Action> {
        let mut found = Vec::new();
        for (watcher, action) in [
            (&self.screenshot, Action::Screenshot),
            (&self.clip, Action::Clip),
        ] {
            if let Some(w) = watcher {
                found.extend(
                    w.drain()
                        .iter()
                        .filter(|e| matches!(e, KeyEvent::Pressed(_)))
                        .map(|_| action),
                );
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Settings {
        Settings {
            capture_enabled: true,
            clip_mode: "both".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_listens_while_capturing_is_off() {
        let off = Settings {
            capture_enabled: false,
            ..on()
        };
        assert_eq!(binding(&off, Action::Screenshot), None);
        assert_eq!(binding(&off, Action::Clip), None);
    }

    #[test]
    fn the_keys_follow_the_settings() {
        let s = on();
        assert_eq!(
            binding(&s, Action::Screenshot),
            Some(("F9".to_string(), String::new()))
        );
        assert_eq!(
            binding(&s, Action::Clip),
            Some(("F10".to_string(), String::new()))
        );
        // Automatic clips only: the clip key does nothing.
        let auto = Settings {
            clip_mode: "auto".into(),
            ..on()
        };
        assert_eq!(binding(&auto, Action::Clip), None);
        // A key and a button, or just a button.
        let pad = Settings {
            screenshot_key: String::new(),
            screenshot_button: "code:304".into(),
            ..on()
        };
        assert_eq!(
            binding(&pad, Action::Screenshot),
            Some((String::new(), "code:304".to_string()))
        );
        let none = Settings {
            screenshot_key: String::new(),
            ..on()
        };
        assert_eq!(binding(&none, Action::Screenshot), None);
    }
}
