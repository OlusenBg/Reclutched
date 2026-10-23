//! Knows whether the Rocket League window has the focus, so the overlay only shows on the game.
//!
//! Hyprland, Sway / i3 and niri are supported: each reports focus changes on a socket we can read
//! without any extra permission. On other desktops (GNOME, KDE) the answer is "unknown" and the
//! overlay falls back to "show while the game is connected".

use std::io::{BufRead, BufReader, Read, Write};
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
        } else if let Some(socket) = env_path("NIRI_SOCKET") {
            let state = state.clone();
            std::thread::spawn(move || watch_niri(&socket, &hints, &state));
        } else if let Some(socket) = env_path("SWAYSOCK").or_else(|| env_path("I3SOCK")) {
            let state = state.clone();
            std::thread::spawn(move || watch_sway(&socket, &hints, &state));
        }
        Self { state }
    }

    /// `Some(true)` when the game has the focus, `Some(false)` when something else has, `None`
    /// when we cannot tell (an unsupported desktop, or the compositor is not answering).
    pub fn game_focused(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            FOCUSED => Some(true),
            NOT_FOCUSED => Some(false),
            _ => None,
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
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

// ---------- Sway / i3 ----------

const IPC_MAGIC: &[u8] = b"i3-ipc";
const IPC_SUBSCRIBE: u32 = 2;
const IPC_GET_TREE: u32 = 4;
const IPC_EVENT_WINDOW: u32 = 0x8000_0003;

/// One i3/Sway IPC message: magic, payload length, type (both little-endian on x86/ARM), payload.
fn ipc_message(kind: u32, payload: &str) -> Vec<u8> {
    let mut out = IPC_MAGIC.to_vec();
    out.extend((payload.len() as u32).to_ne_bytes());
    out.extend(kind.to_ne_bytes());
    out.extend(payload.as_bytes());
    out
}

fn read_ipc(stream: &mut impl Read) -> std::io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 14];
    stream.read_exact(&mut head)?;
    if &head[..6] != IPC_MAGIC {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let len = u32::from_ne_bytes([head[6], head[7], head[8], head[9]]) as usize;
    let kind = u32::from_ne_bytes([head[10], head[11], head[12], head[13]]);
    // A window tree can be big, but never this big: refuse nonsense instead of allocating it.
    if len > 64 << 20 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok((kind, payload))
}

/// Class (or Wayland app id) and title of a Sway / i3 container. XWayland windows, which is what
/// Proton games are, carry the class in `window_properties`.
pub fn parse_sway_container(node: &serde_json::Value) -> Option<(String, String)> {
    let text = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).map(str::to_string);
    let class = text(node.get("app_id"))
        .filter(|c| !c.is_empty())
        .or_else(|| text(node.get("window_properties").and_then(|p| p.get("class"))))
        .unwrap_or_default();
    Some((class, text(node.get("name")).unwrap_or_default()))
}

/// The focused window in a `GET_TREE` reply.
pub fn focused_in_tree(node: &serde_json::Value) -> Option<(String, String)> {
    if node.get("focused").and_then(|f| f.as_bool()) == Some(true) {
        return parse_sway_container(node);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| node.get(key)?.as_array())
        .flatten()
        .find_map(focused_in_tree)
}

/// The window an event says just got the focus.
pub fn focus_event_window(payload: &[u8]) -> Option<(String, String)> {
    let event: serde_json::Value = serde_json::from_slice(payload).ok()?;
    if event.get("change")?.as_str()? != "focus" {
        return None;
    }
    parse_sway_container(event.get("container")?)
}

fn watch_sway(socket: &PathBuf, hints: &[String], state: &AtomicU8) {
    loop {
        if let Ok(mut stream) = UnixStream::connect(socket) {
            let _ = sway_session(&mut stream, hints, state);
            state.store(UNKNOWN, Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn sway_session(
    stream: &mut UnixStream,
    hints: &[String],
    state: &AtomicU8,
) -> std::io::Result<()> {
    // Where the focus is right now, then every change.
    stream.write_all(&ipc_message(IPC_GET_TREE, ""))?;
    loop {
        let (kind, payload) = read_ipc(stream)?;
        if kind == IPC_GET_TREE {
            if let Some((class, title)) = serde_json::from_slice(&payload)
                .ok()
                .and_then(|tree: serde_json::Value| focused_in_tree(&tree))
            {
                set(state, &class, &title, hints);
            }
            break;
        }
    }
    stream.write_all(&ipc_message(IPC_SUBSCRIBE, r#"["window"]"#))?;
    loop {
        let (kind, payload) = read_ipc(stream)?;
        if kind == IPC_EVENT_WINDOW {
            if let Some((class, title)) = focus_event_window(&payload) {
                set(state, &class, &title, hints);
            }
        }
    }
}

// ---------- niri ----------

/// The windows niri told us about and which one has the focus.
#[derive(Default)]
pub struct NiriWindows {
    windows: std::collections::HashMap<u64, (String, String)>,
    focused: Option<u64>,
}

impl NiriWindows {
    /// Applies one line of niri's event stream.
    pub fn apply(&mut self, event: &serde_json::Value) {
        let window = |w: &serde_json::Value| -> Option<(u64, (String, String))> {
            let text = |key: &str| {
                w.get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            Some((w.get("id")?.as_u64()?, (text("app_id"), text("title"))))
        };
        if let Some(all) = event.get("WindowsChanged") {
            self.windows = all
                .get("windows")
                .and_then(|w| w.as_array())
                .into_iter()
                .flatten()
                .filter_map(|w| {
                    let found = window(w)?;
                    if w.get("is_focused").and_then(|f| f.as_bool()) == Some(true) {
                        self.focused = Some(found.0);
                    }
                    Some(found)
                })
                .collect();
        } else if let Some(changed) = event.get("WindowOpenedOrChanged") {
            if let Some(w) = changed.get("window") {
                if let Some((id, info)) = window(w) {
                    self.windows.insert(id, info);
                    if w.get("is_focused").and_then(|f| f.as_bool()) == Some(true) {
                        self.focused = Some(id);
                    }
                }
            }
        } else if let Some(closed) = event.get("WindowClosed") {
            if let Some(id) = closed.get("id").and_then(|i| i.as_u64()) {
                self.windows.remove(&id);
                if self.focused == Some(id) {
                    self.focused = None;
                }
            }
        } else if let Some(change) = event.get("WindowFocusChanged") {
            // `id` is null when no window has the focus.
            self.focused = change.get("id").and_then(|i| i.as_u64());
        }
    }

    /// Class and title of the focused window; empty when nothing is focused.
    pub fn focused(&self) -> (String, String) {
        self.focused
            .and_then(|id| self.windows.get(&id).cloned())
            .unwrap_or_default()
    }
}

fn watch_niri(socket: &PathBuf, hints: &[String], state: &AtomicU8) {
    loop {
        if let Ok(mut stream) = UnixStream::connect(socket) {
            if stream.write_all(b"\"EventStream\"\n").is_ok() {
                let mut windows = NiriWindows::default();
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    windows.apply(&event);
                    let (class, title) = windows.focused();
                    set(state, &class, &title, hints);
                }
            }
            state.store(UNKNOWN, Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_secs(3));
    }
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

    #[test]
    fn the_focused_window_is_found_in_a_sway_tree() {
        let tree = serde_json::json!({
            "focused": false, "name": "root",
            "nodes": [{
                "focused": false, "name": "HEADLESS-1",
                "nodes": [{"focused": false, "name": "1", "nodes": [], "floating_nodes": [
                    {"focused": false, "name": "x", "app_id": "kitty"},
                    {"focused": true, "name": "Rocket League (64-bit, DX11, Cooked)",
                     "app_id": null, "window_properties": {"class": "steam_app_252950"}}
                ]}]
            }]
        });
        assert_eq!(
            focused_in_tree(&tree),
            Some((
                "steam_app_252950".into(),
                "Rocket League (64-bit, DX11, Cooked)".into()
            ))
        );
        let wayland = serde_json::json!({"focused": true, "name": "term", "app_id": "kitty"});
        assert_eq!(
            focused_in_tree(&wayland),
            Some(("kitty".into(), "term".into()))
        );
    }

    #[test]
    fn sway_focus_events_are_read_and_others_ignored() {
        let focus = br#"{"change":"focus","container":{"name":"Rocket League (64-bit, DX11, Cooked)","app_id":null,"window_properties":{"class":"steam_app_252950"}}}"#;
        assert_eq!(
            focus_event_window(focus),
            Some((
                "steam_app_252950".into(),
                "Rocket League (64-bit, DX11, Cooked)".into()
            ))
        );
        assert_eq!(
            focus_event_window(br#"{"change":"title","container":{"name":"x"}}"#),
            None
        );
        assert_eq!(focus_event_window(b"not json"), None);
    }

    #[test]
    fn ipc_messages_round_trip() {
        let bytes = ipc_message(IPC_SUBSCRIBE, r#"["window"]"#);
        let (kind, payload) = read_ipc(&mut &bytes[..]).unwrap();
        assert_eq!(kind, IPC_SUBSCRIBE);
        assert_eq!(payload, br#"["window"]"#);
        assert!(read_ipc(&mut &b"nope"[..]).is_err());
        assert!(read_ipc(&mut &b"i3-ipcxxxxxxxx"[..]).is_err());
    }

    #[test]
    fn niri_events_track_the_focused_window() {
        let mut w = NiriWindows::default();
        let line = |t: &str| serde_json::from_str::<serde_json::Value>(t).unwrap();
        w.apply(&line(
            r#"{"WindowsChanged":{"windows":[
                {"id":1,"title":"term","app_id":"foot","is_focused":true},
                {"id":2,"title":"Rocket League (64-bit, DX11, Cooked)","app_id":"steam_app_252950","is_focused":false}]}}"#,
        ));
        assert_eq!(w.focused(), ("foot".into(), "term".into()));
        w.apply(&line(r#"{"WindowFocusChanged":{"id":2}}"#));
        assert!(is_game_window(&w.focused().0, &w.focused().1, &[]));
        // A title change on the focused window.
        w.apply(&line(
            r#"{"WindowOpenedOrChanged":{"window":{"id":2,"title":"Rocket League","app_id":"steam_app_252950","is_focused":true}}}"#,
        ));
        assert_eq!(w.focused().1, "Rocket League");
        w.apply(&line(r#"{"WindowFocusChanged":{"id":null}}"#));
        assert_eq!(w.focused(), (String::new(), String::new()));
        w.apply(&line(r#"{"WindowFocusChanged":{"id":1}}"#));
        w.apply(&line(r#"{"WindowClosed":{"id":1}}"#));
        assert_eq!(w.focused(), (String::new(), String::new()));
    }

    /// Needs a running Sway (`SWAYSOCK`) whose focused window has "reclutched" in its app id:
    /// `cargo test -p reclutched-ui -- --ignored sway_live`.
    #[test]
    #[ignore]
    fn sway_live() {
        let focus = GameFocus::start(vec!["reclutched".to_string()]);
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(focus.game_focused(), Some(true));
        let other = GameFocus::start(vec!["not-a-window".to_string()]);
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(other.game_focused(), Some(false));
    }
}
