//! The first-launch setup as plain logic: which step comes next, what is allowed, and how the
//! answers turn into settings. No GTK in here, so it is easy to test.

use reclutched_core::rocketleague::settings::{DisplayMode, Settings};

pub const MAX_USERNAME_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Username,
    Mode,
    Kind,
    /// The three yes/no questions of the Regular setup.
    Regular,
    /// The Advanced setup: one page per group of switches.
    AdvMatches,
    AdvEvents,
    AdvHabits,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupKind {
    Regular,
    Advanced,
}

/// The answers of the Regular setup. Defaults are what we recommend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegularAnswers {
    /// Also track offline matches (bots, exhibition)?
    pub offline_matches: bool,
    /// Collect detailed stats (boost habits, touches, possession, crossbar hits)?
    pub detailed_stats: bool,
    /// Remember other players' names in the match history?
    pub remember_names: bool,
}

impl Default for RegularAnswers {
    fn default() -> Self {
        Self {
            offline_matches: false,
            detailed_stats: true,
            remember_names: true,
        }
    }
}

/// The switches the "detailed stats" question controls.
pub const DETAILED_SWITCHES: [&str; 5] = [
    "movement",
    "ball_hits",
    "possession",
    "crossbar_hits",
    "boost_pickups",
];

/// Switches shown on each Advanced page. Together they are every switch in `TOGGLES`.
pub const ADV_MATCHES: [&str; 3] = ["matches", "roster", "offline_matches"];
pub const ADV_EVENTS: [&str; 4] = ["goals", "stat_events", "crossbar_hits", "boost_pickups"];
pub const ADV_HABITS: [&str; 3] = ["movement", "ball_hits", "possession"];

/// Everything typed or chosen so far.
#[derive(Debug, Clone)]
pub struct SetupState {
    pub username: String,
    pub mode: Option<DisplayMode>,
    pub kind: Option<SetupKind>,
    pub regular: RegularAnswers,
    /// The settings as they are being edited. The Advanced pages change switches in here
    /// directly; it starts as whatever is saved (so people who used the CLI keep their choices).
    pub draft: Settings,
}

impl SetupState {
    pub fn new(current: Settings) -> Self {
        Self {
            username: current.username.clone().unwrap_or_default(),
            mode: current.display_mode,
            kind: None,
            regular: RegularAnswers::default(),
            draft: current,
        }
    }

    /// The steps for the chosen setup. Until a setup is chosen we assume Regular.
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::Username, Step::Mode, Step::Kind];
        match self.kind {
            Some(SetupKind::Advanced) => {
                steps.extend([Step::AdvMatches, Step::AdvEvents, Step::AdvHabits]);
            }
            _ => steps.push(Step::Regular),
        }
        steps.push(Step::Finish);
        steps
    }

    pub fn next(&self, from: Step) -> Option<Step> {
        let steps = self.steps();
        let i = steps.iter().position(|s| *s == from)?;
        steps.get(i + 1).copied()
    }

    pub fn back(&self, from: Step) -> Option<Step> {
        let steps = self.steps();
        let i = steps.iter().position(|s| *s == from)?;
        i.checked_sub(1).map(|i| steps[i])
    }

    /// (position starting at 0, total) for the step dots.
    pub fn progress(&self, at: Step) -> (usize, usize) {
        let steps = self.steps();
        (
            steps.iter().position(|s| *s == at).unwrap_or(0),
            steps.len(),
        )
    }

    /// Whether the Next button may be used on this step.
    pub fn can_continue(&self, at: Step) -> bool {
        match at {
            Step::Username => username_error(&self.username).is_none(),
            Step::Mode => self.mode.is_some(),
            Step::Kind => self.kind.is_some(),
            _ => true,
        }
    }

    /// The settings to save when the setup is finished.
    pub fn finish(&self) -> Settings {
        let mut s = self.draft.clone();
        s.setup_complete = true;
        s.username = Some(self.username.trim().to_string());
        s.display_mode = self.mode;
        if self.kind == Some(SetupKind::Regular) {
            let r = self.regular;
            s.offline_matches = r.offline_matches;
            for name in DETAILED_SWITCHES {
                s.set(name, r.detailed_stats);
            }
            s.roster = r.remember_names;
        }
        s
    }

    /// The lines of the summary page: (what, value).
    pub fn summary(&self) -> Vec<(&'static str, String)> {
        let done = self.finish();
        let on = count_on(&done);
        vec![
            ("Username", done.username.clone().unwrap_or_default()),
            ("Mode", mode_label(self.mode).to_string()),
            ("Setup", kind_label(self.kind).to_string()),
            (
                "Tracking",
                format!(
                    "{on} of {} switches on",
                    reclutched_core::rocketleague::settings::TOGGLES.len()
                ),
            ),
        ]
    }
}

fn count_on(s: &Settings) -> usize {
    reclutched_core::rocketleague::settings::TOGGLES
        .iter()
        .filter(|(name, _)| s.get(name) == Some(true))
        .count()
}

pub fn mode_label(mode: Option<DisplayMode>) -> &'static str {
    match mode {
        Some(DisplayMode::Overlay) => "Overlay",
        Some(DisplayMode::TwoScreens) => "2 Screens",
        Some(DisplayMode::Both) => "Both",
        None => "Not chosen",
    }
}

pub fn kind_label(kind: Option<SetupKind>) -> &'static str {
    match kind {
        Some(SetupKind::Regular) => "Regular",
        Some(SetupKind::Advanced) => "Advanced",
        None => "Not chosen",
    }
}

/// Why this username can't be used, or `None` when it is fine.
pub fn username_error(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Some("Please enter your username.");
    }
    if name.chars().count() > MAX_USERNAME_CHARS {
        return Some("That is too long (32 characters at most).");
    }
    if name.chars().any(char::is_control) {
        return Some("That contains characters we can't use.");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reclutched_core::rocketleague::settings::TOGGLES;

    fn state() -> SetupState {
        SetupState::new(Settings::default())
    }

    #[test]
    fn usernames_are_trimmed_and_checked() {
        assert!(username_error("").is_some());
        assert!(username_error("   ").is_some());
        assert!(username_error("Olusen_Bg").is_none());
        assert!(username_error("  Zed  ").is_none());
        assert!(username_error(&"x".repeat(32)).is_none());
        assert!(username_error(&"x".repeat(33)).is_some());
        // Spaces around the name do not count towards the length.
        assert!(username_error(&format!("  {}  ", "x".repeat(32))).is_none());
        assert!(username_error("bad\nname").is_some());
        assert!(
            username_error("Igrisア").is_none(),
            "non-ASCII names are fine"
        );
    }

    #[test]
    fn regular_path_goes_username_mode_kind_regular_finish() {
        let mut s = state();
        s.kind = Some(SetupKind::Regular);
        let mut seen = vec![Step::Username];
        while let Some(next) = s.next(*seen.last().unwrap()) {
            seen.push(next);
        }
        assert_eq!(
            seen,
            [
                Step::Username,
                Step::Mode,
                Step::Kind,
                Step::Regular,
                Step::Finish
            ]
        );
        assert_eq!(s.back(Step::Username), None);
        assert_eq!(s.next(Step::Finish), None);
        assert_eq!(s.back(Step::Regular), Some(Step::Kind));
    }

    #[test]
    fn advanced_path_has_three_switch_pages() {
        let mut s = state();
        s.kind = Some(SetupKind::Advanced);
        assert_eq!(s.next(Step::Kind), Some(Step::AdvMatches));
        assert_eq!(s.next(Step::AdvMatches), Some(Step::AdvEvents));
        assert_eq!(s.next(Step::AdvEvents), Some(Step::AdvHabits));
        assert_eq!(s.next(Step::AdvHabits), Some(Step::Finish));
        assert_eq!(s.back(Step::Finish), Some(Step::AdvHabits));
        assert_eq!(s.progress(Step::AdvEvents), (4, 7));
        s.kind = Some(SetupKind::Regular);
        assert_eq!(s.progress(Step::Regular), (3, 5));
    }

    #[test]
    fn advanced_pages_cover_every_switch_exactly_once() {
        let mut all: Vec<&str> = ADV_MATCHES
            .iter()
            .chain(&ADV_EVENTS)
            .chain(&ADV_HABITS)
            .copied()
            .collect();
        // crossbar_hits and boost_pickups appear on the events page, so no duplicates are expected.
        all.sort_unstable();
        let mut expected: Vec<&str> = TOGGLES.iter().map(|(n, _)| *n).collect();
        expected.sort_unstable();
        assert_eq!(all, expected);
    }

    #[test]
    fn next_button_rules() {
        let mut s = state();
        assert!(!s.can_continue(Step::Username));
        s.username = "  Zed ".into();
        assert!(s.can_continue(Step::Username));
        assert!(!s.can_continue(Step::Mode));
        s.mode = Some(DisplayMode::Both);
        assert!(s.can_continue(Step::Mode));
        assert!(!s.can_continue(Step::Kind));
        s.kind = Some(SetupKind::Regular);
        assert!(s.can_continue(Step::Kind));
        assert!(s.can_continue(Step::Regular) && s.can_continue(Step::Finish));
    }

    #[test]
    fn regular_defaults_turn_on_detail_and_names_but_not_offline() {
        let mut s = state();
        s.username = " Zed ".into();
        s.mode = Some(DisplayMode::TwoScreens);
        s.kind = Some(SetupKind::Regular);
        let out = s.finish();
        assert!(out.setup_complete);
        assert_eq!(out.username.as_deref(), Some("Zed"));
        assert_eq!(out.display_mode, Some(DisplayMode::TwoScreens));
        assert!(!out.offline_matches);
        assert!(out.roster && out.matches && out.goals && out.stat_events);
        for name in DETAILED_SWITCHES {
            assert_eq!(out.get(name), Some(true), "{name}");
        }
    }

    #[test]
    fn regular_answers_change_exactly_their_switches() {
        let mut s = state();
        s.username = "Zed".into();
        s.mode = Some(DisplayMode::Overlay);
        s.kind = Some(SetupKind::Regular);
        s.regular = RegularAnswers {
            offline_matches: true,
            detailed_stats: false,
            remember_names: false,
        };
        let out = s.finish();
        assert!(out.offline_matches && !out.roster);
        for name in DETAILED_SWITCHES {
            assert_eq!(out.get(name), Some(false), "{name}");
        }
        // The rest is untouched: the result, goals and stat events stay on.
        assert!(out.matches && out.goals && out.stat_events);
    }

    #[test]
    fn advanced_keeps_the_switches_exactly_as_edited_and_ignores_regular_answers() {
        // e.g. switched off earlier with the CLI
        let current = Settings {
            ball_hits: false,
            ..Settings::default()
        };
        let mut s = SetupState::new(current);
        assert!(!s.draft.ball_hits, "advanced starts from the saved values");
        s.username = "Zed".into();
        s.mode = Some(DisplayMode::Both);
        s.kind = Some(SetupKind::Advanced);
        s.draft.set("goals", false);
        s.regular.detailed_stats = true; // must not be applied in Advanced
        let out = s.finish();
        assert!(!out.goals && !out.ball_hits && out.movement);
        assert_eq!(out.display_mode, Some(DisplayMode::Both));
    }

    #[test]
    fn finish_keeps_unrelated_settings() {
        let current = Settings {
            my_primary_id: Some("Epic|1|0".into()),
            ..Settings::default()
        };
        let mut s = SetupState::new(current);
        s.username = "Zed".into();
        s.kind = Some(SetupKind::Regular);
        assert_eq!(s.finish().my_primary_id.as_deref(), Some("Epic|1|0"));
    }

    #[test]
    fn summary_lists_the_choices() {
        let mut s = state();
        s.username = "Zed".into();
        s.mode = Some(DisplayMode::TwoScreens);
        s.kind = Some(SetupKind::Regular);
        let text = s.summary();
        assert_eq!(text[0], ("Username", "Zed".to_string()));
        assert_eq!(text[1].1, "2 Screens");
        assert_eq!(text[2].1, "Regular");
        assert_eq!(text[3].1, "9 of 10 switches on");
    }

    #[test]
    fn existing_username_and_mode_prefill_a_rerun() {
        let current = Settings {
            username: Some("Old".into()),
            display_mode: Some(DisplayMode::Overlay),
            ..Settings::default()
        };
        let s = SetupState::new(current);
        assert_eq!(s.username, "Old");
        assert_eq!(s.mode, Some(DisplayMode::Overlay));
        assert_eq!(s.kind, None, "the kind is always asked again");
    }
}
