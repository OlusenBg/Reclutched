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
        }
    }
}

impl Settings {
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
