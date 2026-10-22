//! Knows whether the Rocket League window has the focus, so the overlay only shows on the game.
//!
//! Only Hyprland is supported for now: it reports every focus change on a socket we can read
//! without any extra permission. On other desktops the answer is "unknown" and the overlay falls
//! back to "show while the game is connected".

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

const UNKNOWN: u8 = 0;
const NOT_FOCUSED: u8 = 1;
const FOCUSED: u8 = 2;

/// Whether the focused window is the game. Cheap to clone and to ask.
#[derive(Clone)]
pub struct GameFocus {
    state: Arc<AtomicU8>,
}

impl GameFocus {
    /// Starts watching in the background. `hints` are extra words that identify the game window.
    pub fn start(hints: Vec<String>) -> Self {
        let state = Arc::new(AtomicU8::new(UNKNOWN));
        if let Some(socket) = hyprland_event_socket() {
            let state = state.clone();
            std::thread::spawn(move || watch_hyprland(&socket, &hints, &state));
        }
        Self { state }
    }

    /// `Some(true)` when the game has the focus, `Some(false)` when something else has, `None`
    /// when we cannot tell (not Hyprland, or Hyprland is not answering).
    pub fn game_focused(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            FOCUSED => Some(true),
            NOT_FOCUSED => Some(false),
            _ => None,
        }
    }
}

/// `$XDG_RUNTIME_DIR/hypr/<instance>/.socket2.sock` (older versions used `/tmp/hypr/...`).
fn hyprland_event_socket() -> Option<PathBuf> {
    let instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    if instance.is_empty() {
        return None;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let new = runtime.map(|r| r.join("hypr").join(&instance).join(".socket2.sock"));
    match new {
        Some(path) if path.exists() => Some(path),
        _ => Some(PathBuf::from(format!("/tmp/hypr/{instance}/.socket2.sock"))),
    }
}

fn watch_hyprland(socket: &PathBuf, hints: &[String], state: &AtomicU8) {
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => {
                // The events only tell us about changes, so ask once for the current window.
                if let Some((class, title)) = current_window() {
                    set(state, &class, &title, hints);
                }
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if let Some((class, title)) = parse_active_window(&line) {
                        set(state, &class, &title, hints);
                    }
                }
                // Hyprland went away.
                state.store(UNKNOWN, Ordering::Relaxed);
            }
            Err(_) => state.store(UNKNOWN, Ordering::Relaxed),
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn set(state: &AtomicU8, class: &str, title: &str, hints: &[String]) {
    let value = if is_game_window(class, title, hints) {
        FOCUSED
    } else {
        NOT_FOCUSED
    };
    state.store(value, Ordering::Relaxed);
}

/// The focused window right now, from `hyprctl activewindow -j`.
fn current_window() -> Option<(String, String)> {
    let out = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;
    parse_window_json(&String::from_utf8_lossy(&out.stdout))
}

fn parse_window_json(text: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let get = |key: &str| value.get(key)?.as_str().map(str::to_string);
    Some((get("class").unwrap_or_default(), get("title")?))
}

/// `activewindow>>CLASS,TITLE` (the title may contain commas, the class cannot).
pub fn parse_active_window(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("activewindow>>")?;
    let (class, title) = rest.split_once(',').unwrap_or((rest, ""));
    Some((class.to_string(), title.to_string()))
}

/// Whether a window with this class and title is Rocket League: the usual Steam/Proton class, a
/// class containing "rocketleague", the game's own title, or any of the user's extra hints.
pub fn is_game_window(class: &str, title: &str, hints: &[String]) -> bool {
    let (class, title) = (class.to_lowercase(), title.to_lowercase());
    // Not just "contains": a browser tab called "Rocket League - Reddit" must not count.
    if class == "steam_app_252950" || class.contains("rocketleague") {
        return true;
    }
    if title.starts_with("rocket league (") || title == "rocket league" {
        return true;
    }
    hints
        .iter()
        .map(|h| h.to_lowercase())
        .filter(|h| !h.is_empty())
        .any(|h| class.contains(&h) || title.contains(&h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usual_game_windows_are_recognised() {
        assert!(is_game_window(
            "steam_app_252950",
            "Rocket League (64-bit, DX11, Cooked)",
            &[]
        ));
        assert!(is_game_window("rocketleague.exe", "", &[]));
        assert!(is_game_window(
            "",
            "Rocket League (64-bit, DX11, Cooked)",
            &[]
        ));
        assert!(is_game_window("Steam_App_252950", "whatever", &[]));
    }

    #[test]
    fn other_windows_are_not() {
        assert!(!is_game_window(
            "firefox",
            "Rocket League - Reddit - Mozilla Firefox",
            &[]
        ));
        assert!(!is_game_window("kitty", "reclutched-ui ~/Reclutched", &[]));
        assert!(!is_game_window("", "", &[]));
    }

    #[test]
    fn hints_catch_other_launchers() {
        let hints = vec!["rl-heroic".to_string(), String::new()];
        assert!(is_game_window("RL-Heroic", "x", &hints));
        assert!(is_game_window(
            "steam_proton",
            "my rl-heroic window",
            &hints
        ));
        assert!(
            !is_game_window("kitty", "x", &hints),
            "an empty hint matches nothing"
        );
    }

    #[test]
    fn events_are_parsed_with_commas_in_the_title() {
        assert_eq!(
            parse_active_window("activewindow>>kitty,a, b, c"),
            Some(("kitty".into(), "a, b, c".into()))
        );
        assert_eq!(
            parse_active_window("activewindow>>,"),
            Some((String::new(), String::new()))
        );
        assert_eq!(parse_active_window("workspace>>2"), None);
        assert_eq!(parse_active_window("activewindowv2>>55d2074f29d0"), None);
    }

    #[test]
    fn the_current_window_comes_from_hyprctl_json() {
        let json = r#"{"address":"0x1","class":"steam_app_252950","title":"Rocket League (64-bit, DX11, Cooked)","pid":3}"#;
        assert_eq!(
            parse_window_json(json),
            Some((
                "steam_app_252950".into(),
                "Rocket League (64-bit, DX11, Cooked)".into()
            ))
        );
        // No window focused: hyprctl prints an empty object or "Invalid".
        assert_eq!(parse_window_json("{}"), None);
        assert_eq!(parse_window_json("Invalid"), None);
    }
}
