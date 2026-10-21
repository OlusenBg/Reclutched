//! The settings page: a profile, the display mode and every tracking switch.
//! Changes are saved as soon as you make them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};
use reclutched_core::rocketleague::statsapi;

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

/// Accepts the game's install folder, its `TAGame` folder or its `Config` folder and returns
/// the `Config` folder, or `None` if this is not Rocket League.
fn normalize_game_folder(chosen: &Path) -> Option<PathBuf> {
    [
        chosen.to_path_buf(),
        chosen.join("Config"),
        chosen.join("TAGame/Config"),
    ]
    .into_iter()
    .find(|dir| statsapi::config_file_in(dir).is_some())
}

/// What to tell the user about the Stats API in this folder.
fn statsapi_summary(config_dir: Option<&Path>) -> (String, bool) {
    let Some(dir) = config_dir else {
        return ("Game folder not found yet".to_string(), false);
    };
    let Some(file) = statsapi::config_file_in(dir) else {
        return (
            "No Stats API settings file in that folder".to_string(),
            false,
        );
    };
    match std::fs::read_to_string(&file).map(|t| statsapi::read(&t)) {
        Ok(s) if s.is_on() => (
            format!("On, {} updates per second", s.packet_send_rate),
            false,
        ),
        Ok(_) => (
            "Off. Matches cannot be tracked until it is on".to_string(),
            true,
        ),
        Err(e) => (format!("Cannot read the settings file: {e}"), false),
    }
}

/// Opens the dialog over `parent`. `on_setup` runs when "Run setup again" is pressed.
/// Messages (saved, could not save, ...) appear inside the dialog itself, because the main
/// window's own toasts would be hidden behind it.
pub fn open(
    parent: &adw::ApplicationWindow,
    settings_path: PathBuf,
    db_path: PathBuf,
    on_setup: impl Fn() + 'static,
) {
    let current = Settings::load(&settings_path).unwrap_or_default();
    let path = Rc::new(settings_path);
    let dialog = adw::PreferencesDialog::builder()
        .title("Settings")
        .content_width(560)
        .content_height(640)
        .build();
    let on_error: Rc<dyn Fn(&str)> = {
        let dialog = dialog.clone();
        Rc::new(move |message| dialog.add_toast(adw::Toast::new(message)))
    };

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

    // --- Rocket League: where it is, and whether its Stats API is on ---
    let game = adw::PreferencesGroup::builder()
        .title("Rocket League")
        .description("The game only shares your stats when its Stats API is on. Restart the game after changing it.")
        .build();
    let folder_row = adw::ActionRow::builder()
        .title("Game folder")
        .subtitle_selectable(true)
        .build();
    let api_row = adw::ActionRow::builder().title("Stats API").build();
    let find_button = gtk::Button::builder()
        .label("Find")
        .valign(gtk::Align::Center)
        .tooltip_text("Look for Rocket League in the usual places (Heroic, Steam, Lutris)")
        .build();
    let choose_button = gtk::Button::builder()
        .label("Choose\u{2026}")
        .valign(gtk::Align::Center)
        .tooltip_text("Pick the game folder yourself")
        .build();
    let on_button = gtk::Button::builder()
        .label("Turn on")
        .css_classes(["suggested-action"])
        .valign(gtk::Align::Center)
        .build();
    folder_row.add_suffix(&find_button);
    folder_row.add_suffix(&choose_button);
    api_row.add_suffix(&on_button);
    game.add(&folder_row);
    game.add(&api_row);
    page.add(&game);

    // Shows the saved folder and the Stats API state.
    let refresh = {
        let path = path.clone();
        let folder_row = folder_row.clone();
        let api_row = api_row.clone();
        let on_button = on_button.clone();
        Rc::new(move || {
            let dir = Settings::load(&path).ok().and_then(|s| s.game_config_dir);
            folder_row.set_subtitle(
                &dir.as_deref()
                    .map_or("Not found yet. Press Find or Choose.".to_string(), |d| {
                        d.display().to_string()
                    }),
            );
            let (text, can_turn_on) = statsapi_summary(dir.as_deref());
            api_row.set_subtitle(&text);
            on_button.set_visible(can_turn_on);
        })
    };
    // The first time: look for the game by ourselves, so most people never need to press Find.
    if current.game_config_dir.is_none() {
        if let Some(found) = statsapi::find_game_config_dirs().into_iter().next() {
            let _ = update(&path, |s| s.game_config_dir = Some(found));
        }
    }
    refresh();
    {
        let path = path.clone();
        let refresh = refresh.clone();
        let on_error = on_error.clone();
        find_button.connect_clicked(move |_| {
            match statsapi::find_game_config_dirs().into_iter().next() {
                Some(found) => {
                    if let Err(e) = update(&path, |s| s.game_config_dir = Some(found)) {
                        on_error(&format!("Could not save: {e}"));
                    }
                }
                None => on_error("Rocket League was not found. Use Choose to pick its folder."),
            }
            refresh();
        });
    }
    {
        let path = path.clone();
        let refresh = refresh.clone();
        let on_error = on_error.clone();
        let parent = parent.clone();
        choose_button.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose Rocket League's game folder")
                .build();
            let (path, refresh, on_error) = (path.clone(), refresh.clone(), on_error.clone());
            dialog.select_folder(Some(&parent), gtk::gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return }; // cancelled
                let Some(chosen) = file.path() else { return };
                match normalize_game_folder(&chosen) {
                    Some(dir) => {
                        if let Err(e) = update(&path, |s| s.game_config_dir = Some(dir)) {
                            on_error(&format!("Could not save: {e}"));
                        }
                        refresh();
                    }
                    None => on_error(
                        "That does not look like Rocket League's folder (no TAGame/Config inside).",
                    ),
                }
            });
        });
    }
    {
        let path = path.clone();
        let refresh = refresh.clone();
        let on_error = on_error.clone();
        on_button.connect_clicked(move |_| {
            let file = Settings::load(&path)
                .ok()
                .and_then(|s| s.game_config_dir)
                .and_then(|d| statsapi::config_file_in(&d));
            match file {
                Some(file) => match statsapi::turn_on_in_file(&file, statsapi::RECOMMENDED_RATE) {
                    Ok(()) => on_error(
                        "Stats API turned on. Restart Rocket League for it to take effect.",
                    ),
                    Err(e) => on_error(&format!("Could not change {}: {e}", file.display())),
                },
                None => on_error("Choose the game folder first."),
            }
            refresh();
        });
    }

    // --- should the app track by itself? ---
    let tracking = adw::PreferencesGroup::builder().title("Tracking").build();
    let auto_row = adw::SwitchRow::builder()
        .title("Track matches when the app opens")
        .subtitle("The app watches Rocket League and saves every match. You can also switch it on and off from the main window.")
        .active(current.auto_track)
        .build();
    {
        let path = path.clone();
        let on_error = on_error.clone();
        auto_row.connect_active_notify(move |row| {
            let on = row.is_active();
            if let Err(e) = update(&path, |s| s.auto_track = on) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    tracking.add(&auto_row);
    page.add(&tracking);

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
