//! What Reclutched is allowed to collect and store. Every switch can be turned off, and the
//! future settings page will edit this same file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Every on/off switch: its name (as typed in `reclutched config`) and what it does.
pub const TOGGLES: &[(&str, &str)] = &[
    (
        "matches",
        "result, score and abandoned matches (off = nothing is stored at all)",
    ),
    (
        "roster",
        "who played, final stats per player, who joined/left and when",
    ),
    (
        "goals",
        "every goal: scorer, assist, speed, position, time, boost at the moment",
    ),
    (
        "stat_events",
        "shots, saves, demolitions, MVP...: the events plus your boost/speed when they happened",
    ),
    (
        "ball_hits",
        "ball touches as averages per player: counts, field thirds, hit speed",
    ),
    (
        "boost_pickups",
        "boost pad pickup counts (the game only sends these while spectating)",
    ),
    ("crossbar_hits", "crossbar hit counts and the hardest hit"),
    (
        "movement",
        "boost, speed, powerslide and demolished habits (averages per match, no raw samples)",
    ),
    ("possession", "ball possession % and ball speed"),
    (
        "offline_matches",
        "also store offline matches (bots, exhibition, training)",
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("cannot read or write the settings file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the settings file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// How the app shows live information while you play. Chosen in the setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// Only the in-game overlay, no other window.
    Overlay,
    /// A live stats window instead of an overlay (for people with two screens).
    TwoScreens,
    /// Both the overlay and the window.
    Both,
}

/// `#[serde(default)]` means a key missing from the file falls back to [`Settings::default`],
/// so old settings files keep working when we add new switches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub matches: bool,
    pub roster: bool,
    pub goals: bool,
    pub stat_events: bool,
    pub ball_hits: bool,
    pub boost_pickups: bool,
    pub crossbar_hits: bool,
    pub movement: bool,
    pub possession: bool,
    pub offline_matches: bool,
    /// Your own player id, remembered after the first tracked match (not a switch).
    pub my_primary_id: Option<String>,
    /// The app starts tracking by itself when it opens (not a "what to collect" switch).
    pub auto_track: bool,
    /// False until the first-launch setup in the app has been finished.
    pub setup_complete: bool,
    /// Rocket League's `TAGame/Config` folder, where the Stats API is switched on.
    pub game_config_dir: Option<std::path::PathBuf>,
    /// The Rocket League username typed in the setup.
    pub username: Option<String>,
    /// Overlay, two screens or both; chosen in the setup.
    pub display_mode: Option<DisplayMode>,
    /// The key you hold to show the overlay: a name like "Tab" or `code:<number>` for any other
    /// key; empty for none.
    pub overlay_key: String,
    /// The controller button you hold to show the overlay (same format; empty for none). The key
    /// and the button both work when both are set.
    pub overlay_button: String,
    /// Extra words that identify the Rocket League window (its class or title) when the usual
    /// ones do not match your launcher. Not a "what to collect" switch.
    pub game_window_hints: Vec<String>,
    /// Capturing (screenshots, clips) is on. Needs permission from the desktop each session.
    pub capture_enabled: bool,
    /// The key that takes a screenshot (same format as `overlay_key`; empty for none).
    pub screenshot_key: String,
    /// The controller button that takes a screenshot (same format; empty for none).
    pub screenshot_button: String,
    /// Where screenshots (and later clips) go; `None` means `~/Videos/Reclutched`.
    pub capture_dir: Option<std::path::PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            matches: true,
            roster: true,
            goals: true,
            stat_events: true,
            ball_hits: true,
            boost_pickups: true,
            crossbar_hits: true,
            movement: true,
            possession: true,
            offline_matches: false,
            my_primary_id: None,
            auto_track: true,
            setup_complete: false,
            game_config_dir: None,
            username: None,
            display_mode: None,
            overlay_key: "Tab".to_string(),
            overlay_button: String::new(),
            game_window_hints: Vec::new(),
            capture_enabled: false,
            screenshot_key: "F9".to_string(),
            screenshot_button: String::new(),
            capture_dir: None,
        }
    }
}

impl Settings {
    /// The folder for screenshots and clips: your choice, or `~/Videos/Reclutched`.
    pub fn capture_folder(&self) -> Option<PathBuf> {
        self.capture_dir.clone().or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Videos").join("Reclutched"))
        })
    }

    /// `<config dir>/settings.json`.
    pub fn default_path() -> Option<PathBuf> {
        Some(crate::config_dir()?.join("settings.json"))
    }

    /// A missing file is fine: everything at its default.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Current value of a switch, or `None` if there is no such switch.
    pub fn get(&self, name: &str) -> Option<bool> {
        Some(match name {
            "matches" => self.matches,
            "roster" => self.roster,
            "goals" => self.goals,
            "stat_events" => self.stat_events,
            "ball_hits" => self.ball_hits,
            "boost_pickups" => self.boost_pickups,
            "crossbar_hits" => self.crossbar_hits,
            "movement" => self.movement,
            "possession" => self.possession,
            "offline_matches" => self.offline_matches,
            _ => return None,
        })
    }

    /// Sets a switch; returns false if there is no such switch.
    pub fn set(&mut self, name: &str, value: bool) -> bool {
        let slot = match name {
            "matches" => &mut self.matches,
            "roster" => &mut self.roster,
            "goals" => &mut self.goals,
            "stat_events" => &mut self.stat_events,
            "ball_hits" => &mut self.ball_hits,
            "boost_pickups" => &mut self.boost_pickups,
            "crossbar_hits" => &mut self.crossbar_hits,
            "movement" => &mut self.movement,
            "possession" => &mut self.possession,
            "offline_matches" => &mut self.offline_matches,
            _ => return false,
        };
        *slot = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_collect_everything_except_offline_matches() {
        let s = Settings::default();
        for (name, _) in TOGGLES {
            let expected = *name != "offline_matches";
            assert_eq!(s.get(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn set_and_get_work_for_every_toggle_and_reject_unknown_names() {
        let mut s = Settings::default();
        for (name, _) in TOGGLES {
            let flipped = !s.get(name).unwrap();
            assert!(s.set(name, flipped), "{name}");
            assert_eq!(s.get(name), Some(flipped), "{name}");
        }
        assert!(!s.set("nope", true));
        assert_eq!(s.get("nope"), None);
    }

    #[test]
    fn partial_file_falls_back_to_defaults_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("reclutched-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{ "ball_hits": false }"#).unwrap();

        let mut s = Settings::load(&path).unwrap();
        assert!(!s.ball_hits && s.goals);

        s.my_primary_id = Some("Epic|1|0".into());
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn setup_fields_default_to_first_launch_and_old_files_still_load() {
        let s = Settings::default();
        assert!(!s.setup_complete && s.username.is_none() && s.display_mode.is_none());
        assert!(s.auto_track, "tracking from the app is on by default");
        // A file written before these fields existed.
        let old: Settings =
            serde_json::from_str(r#"{ "goals": false, "my_primary_id": "Epic|1|0" }"#).unwrap();
        assert!(!old.goals && !old.setup_complete && old.username.is_none());
        assert!(
            old.auto_track,
            "files from before this setting existed get the default"
        );
    }

    #[test]
    fn display_mode_uses_readable_names_in_the_file() {
        for (mode, text) in [
            (DisplayMode::Overlay, "overlay"),
            (DisplayMode::TwoScreens, "two_screens"),
            (DisplayMode::Both, "both"),
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<DisplayMode>(&json).unwrap(), mode);
        }
        let s: Settings = serde_json::from_str(
            r#"{ "setup_complete": true, "username": "Zed", "display_mode": "two_screens" }"#,
        )
        .unwrap();
        assert!(s.setup_complete);
        assert_eq!(s.username.as_deref(), Some("Zed"));
        assert_eq!(s.display_mode, Some(DisplayMode::TwoScreens));
    }

    #[test]
    fn missing_file_gives_defaults_and_bad_json_is_an_error() {
        assert_eq!(
            Settings::load(Path::new("/nonexistent/settings.json")).unwrap(),
            Settings::default()
        );
        let path = std::env::temp_dir().join(format!("reclutched-bad-{}.json", std::process::id()));
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(Settings::load(&path), Err(SettingsError::Json(_))));
        std::fs::remove_file(path).unwrap();
    }
}
