//! The settings page: a profile, the display mode and every tracking switch.
//! Changes are saved as soon as you make them.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};

use crate::flow::{username_error, ADV_EVENTS, ADV_HABITS, ADV_MATCHES};
use crate::setup::{switch_subtitle, switch_title};

const MODES: [(DisplayMode, &str); 3] = [
    (DisplayMode::Overlay, "Overlay"),
    (DisplayMode::TwoScreens, "2 Screens"),
    (DisplayMode::Both, "Both"),
];

/// Loads the file, lets `change` edit it and saves it again. Reading first means we never
/// overwrite something that changed elsewhere (for example the player id `track` learned).
fn update(path: &std::path::Path, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut settings = Settings::load(path).map_err(|e| e.to_string())?;
    change(&mut settings);
    settings.save(path).map_err(|e| e.to_string())
}

/// Opens the dialog over `parent`. `on_setup` runs when "Run setup again" is pressed,
/// `on_error` with a message when saving fails.
pub fn open(
    parent: &adw::ApplicationWindow,
    settings_path: PathBuf,
    db_path: PathBuf,
    on_setup: impl Fn() + 'static,
    on_error: impl Fn(&str) + 'static,
) {
    let current = Settings::load(&settings_path).unwrap_or_default();
    let on_error = Rc::new(on_error);
    let path = Rc::new(settings_path);

    let page = adw::PreferencesPage::new();

    // --- profile and mode ---
    let profile = adw::PreferencesGroup::builder().title("Profile").build();
    let name_row = adw::EntryRow::builder()
        .title("Rocket League username")
        .show_apply_button(true)
        .text(current.username.as_deref().unwrap_or(""))
        .build();
    {
        let path = path.clone();
        let on_error = on_error.clone();
        name_row.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            if let Some(problem) = username_error(&text) {
                on_error(problem);
                return;
            }
            if let Err(e) = update(&path, |s| s.username = Some(text)) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    profile.add(&name_row);

    let mode_row = adw::ComboRow::builder()
        .title("Display mode")
        .subtitle("Overlay and live window are coming; your choice is saved")
        .model(&gtk::StringList::new(&MODES.map(|(_, name)| name)))
        .build();
    mode_row.set_selected(
        MODES
            .iter()
            .position(|(m, _)| Some(*m) == current.display_mode)
            .unwrap_or(2) as u32,
    );
    {
        let path = path.clone();
        let on_error = on_error.clone();
        mode_row.connect_selected_notify(move |row| {
            let Some((mode, _)) = MODES.get(row.selected() as usize) else {
                return;
            };
            if let Err(e) = update(&path, |s| s.display_mode = Some(*mode)) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    profile.add(&mode_row);
    page.add(&profile);

    // --- the tracking switches, in the same three groups as the Advanced setup ---
    let groups: [(&str, &[&str]); 3] = [
        ("Matches and players", &ADV_MATCHES),
        ("Events", &ADV_EVENTS),
        ("Habits", &ADV_HABITS),
    ];
    for (i, (title, switches)) in groups.into_iter().enumerate() {
        let group = adw::PreferencesGroup::builder().title(title).build();
        if i == 0 {
            group.set_description(Some("What to collect. Changes apply from the next match."));
        }
        for name in switches {
            let row = adw::SwitchRow::builder()
                .title(switch_title(name))
                .subtitle(switch_subtitle(name))
                .active(current.get(name).unwrap_or(true))
                .build();
            let path = path.clone();
            let on_error = on_error.clone();
            let key = (*name).to_string();
            row.connect_active_notify(move |row| {
                let on = row.is_active();
                if let Err(e) = update(&path, |s| {
                    s.set(&key, on);
                }) {
                    on_error(&format!("Could not save: {e}"));
                }
            });
            group.add(&row);
        }
        page.add(&group);
    }

    // --- where things live, and the way back to the setup ---
    let data = adw::PreferencesGroup::builder().title("Your data").build();
    for (title, value) in [
        ("Match database", db_path.display().to_string()),
        ("Settings file", path.display().to_string()),
    ] {
        data.add(
            &adw::ActionRow::builder()
                .title(title)
                .subtitle(value)
                .subtitle_selectable(true)
                .build(),
        );
    }
    let rerun = adw::ActionRow::builder()
        .title("Run the setup again")
        .subtitle("Walk through the first-launch questions once more")
        .activatable(true)
        .build();
    rerun.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    data.add(&rerun);
    page.add(&data);

    let dialog = adw::PreferencesDialog::builder()
        .title("Settings")
        .content_width(560)
        .content_height(640)
        .build();
    dialog.add(&page);
    {
        let dialog = dialog.clone();
        rerun.connect_activated(move |_| {
            dialog.close();
            on_setup();
        });
    }
    dialog.present(Some(parent));
}
