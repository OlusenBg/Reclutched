//! The settings page: a profile, the display mode and every tracking switch.
//! Changes are saved as soon as you make them.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};
use reclutched_core::rocketleague::statsapi;

use crate::flow::{username_error, ADV_EVENTS, ADV_HABITS, ADV_MATCHES};
use crate::keys;
use crate::setup::{switch_subtitle, switch_title};

/// The Hyprland (0.55+, Lua config) rule that draws our window fully opaque. See the README.
const HYPRLAND_OPAQUE_RULE: &str =
    r#"hl.window_rule({ match = { class = "^dev\\.reclutched\\.Reclutched$" }, opaque = true })"#;

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

/// Called with the code of the controller button that was pressed (`None`: nothing in time).
pub type PadCallback = Box<dyn Fn(Option<u16>)>;

/// One section of the settings: a heading and groups, next to the sidebar and left-aligned.
struct SectionPage {
    scroller: gtk::ScrolledWindow,
    column: gtk::Box,
}

impl SectionPage {
    fn new(title: &str) -> Self {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Start)
            .margin_top(30)
            .margin_bottom(36)
            .margin_start(40)
            .margin_end(40)
            .build();
        // An invisible line of spaces gives the section its preferred width (so the groups stay
        // at a comfortable width and sit against the sidebar), but, being ellipsized, it can
        // shrink when the window is narrow. The visible heading is drawn on top of it.
        let ruler = gtk::Label::builder()
            .label("\u{00A0}".repeat(90))
            .css_classes(["settings-title", "settings-ruler"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let heading = gtk::Overlay::builder()
            .child(&ruler)
            .margin_bottom(22)
            .build();
        heading.add_overlay(
            &gtk::Label::builder()
                .label(title)
                .css_classes(["settings-title"])
                .halign(gtk::Align::Start)
                .build(),
        );
        column.append(&heading);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&column)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        Self { scroller, column }
    }

    fn add(&self, group: &impl IsA<gtk::Widget>) {
        group.set_margin_bottom(26);
        self.column.append(group);
    }
}

/// What the page needs from the app around it.
pub struct Hooks {
    /// The back arrow.
    pub on_back: Rc<dyn Fn()>,
    /// "Run the setup again".
    pub on_setup: Rc<dyn Fn()>,
    /// Shows a short message (saved, could not save, ...).
    pub toast: Rc<dyn Fn(&str)>,
    /// Listens for the next controller button and calls back with its code (`None` when nothing
    /// was pressed in time).
    pub capture_button: Rc<dyn Fn(PadCallback)>,
    pub cancel_capture: Rc<dyn Fn()>,
    /// The keyboards and controllers we read right now, when the watcher runs.
    pub devices: Rc<dyn Fn() -> Option<(keys::Devices, keys::KeyAccess)>>,
}

/// Builds the settings page. It is shown inside the main window with a back arrow, and changes
/// are saved as soon as you make them.
pub fn build(
    parent: &adw::ApplicationWindow,
    settings_path: PathBuf,
    db_path: PathBuf,
    hooks: Hooks,
    section: Option<&str>,
) -> gtk::Widget {
    let current = Settings::load(&settings_path).unwrap_or_default();
    let path = Rc::new(settings_path);
    let on_error = hooks.toast.clone();

    // One page per section; the sidebar on the left switches between them.
    let general_page = SectionPage::new("General");
    let overlay_page = SectionPage::new("Overlay");
    let game_page = SectionPage::new("Rocket League");
    let track_page = SectionPage::new("What to track");
    let data_page = SectionPage::new("Your data");

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
        .subtitle("Overlay: in-game panel. 2 Screens: live window.")
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

    general_page.add(&profile);
    let (overlay_group, key_controller) = overlay_group(&path, &current, &on_error, &hooks);
    overlay_page.add(&overlay_group);

    // --- Rocket League: where it is, and whether its Stats API is on ---
    let game = adw::PreferencesGroup::builder()
        .title("Game folder and Stats API")
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
    game_page.add(&game);

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
    track_page.add(&tracking);

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
        track_page.add(&group);
    }

    // --- Hyprland draws every window at active_opacity (0.9 in HyDE); a rule makes this one solid ---
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let window = adw::PreferencesGroup::builder()
            .title("Window")
            .description(
                "Looks see-through? Hyprland dims every window. Add this line to your hyprland.lua",
            )
            .build();
        let rule_row = adw::ActionRow::builder()
            .title("Solid window rule")
            .subtitle(HYPRLAND_OPAQUE_RULE)
            .subtitle_selectable(true)
            .build();
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        copy.connect_clicked(|button| button.clipboard().set_text(HYPRLAND_OPAQUE_RULE));
        rule_row.add_suffix(&copy);
        window.add(&rule_row);
        general_page.add(&window);
    }

    // --- where things live, and the way back to the setup ---
    let data = adw::PreferencesGroup::builder().title("Files").build();
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
    data_page.add(&data);
    data_page.add(&crate::wipe::group(
        parent,
        db_path.clone(),
        hooks.toast.clone(),
    ));

    {
        let on_setup = hooks.on_setup.clone();
        rerun.connect_activated(move |_| on_setup());
    }

    let header = adw::HeaderBar::new();
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Back to your matches")
        .build();
    {
        let on_back = hooks.on_back.clone();
        back.connect_clicked(move |_| on_back());
    }
    header.pack_start(&back);
    header.set_title_widget(Some(&adw::WindowTitle::new("Settings", "")));
    let sections: [(&str, &str, &SectionPage); 5] = [
        ("general", "General", &general_page),
        ("overlay", "Overlay", &overlay_page),
        ("game", "Rocket League", &game_page),
        ("track", "What to track", &track_page),
        ("data", "Your data", &data_page),
    ];
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hexpand(true)
        .build();
    let nav = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["settings-nav"])
        .build();
    for (id, title, section) in &sections {
        stack.add_named(&section.scroller, Some(id));
        nav.append(
            &gtk::Label::builder()
                .label(*title)
                .halign(gtk::Align::Start)
                .build(),
        );
    }
    {
        let stack = stack.clone();
        let ids: Vec<&str> = sections.iter().map(|(id, _, _)| *id).collect();
        nav.connect_row_selected(move |_, row| {
            if let Some(id) = row.and_then(|r| ids.get(r.index() as usize)) {
                stack.set_visible_child_name(id);
            }
        });
    }
    let first = section
        .and_then(|wanted| sections.iter().position(|(id, _, _)| *id == wanted))
        .unwrap_or(0);
    nav.select_row(nav.row_at_index(first as i32).as_ref());
    let sidebar = gtk::ScrolledWindow::builder()
        .child(&nav)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .width_request(230)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&sidebar);
    body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    body.append(&stack);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    view.add_controller(key_controller);
    view.upcast()
}

/// The two ways to hold the overlay: a keyboard key and a controller button. Both can be set;
/// either one works. Returns the group and the key controller that must be added to the page
/// (it catches the next key press while you are setting the key).
fn overlay_group(
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
    hooks: &Hooks,
) -> (adw::PreferencesGroup, gtk::EventControllerKey) {
    let group = adw::PreferencesGroup::builder()
        .title("Hold to show")
        .description("Hold the key or the button to show the overlay in the game (Overlay and Both modes). Set one or both; either works. Reading them needs permission: sudo usermod -aG input $USER, then log out and in.")
        .build();

    // --- the keyboard key ---
    let key_row = adw::ActionRow::builder()
        .title("Keyboard key")
        .subtitle(keys::key_label(&current.overlay_key))
        .build();
    let key_change = gtk::Button::builder()
        .label("Change")
        .valign(gtk::Align::Center)
        .build();
    let key_clear = gtk::Button::builder()
        .label("Clear")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    key_row.add_suffix(&key_change);
    key_row.add_suffix(&key_clear);
    group.add(&key_row);

    let key_listening = Rc::new(Cell::new(false));
    {
        let (key_listening, key_row, key_change) =
            (key_listening.clone(), key_row.clone(), key_change.clone());
        let path = path.clone();
        key_change.clone().connect_clicked(move |_| {
            if key_listening.get() {
                // Pressed again: stop listening.
                key_listening.set(false);
                key_change.set_label("Change");
                key_row.set_subtitle(&keys::key_label(&saved_key(&path)));
            } else {
                key_listening.set(true);
                key_change.set_label("Cancel");
                key_row.set_subtitle("Press the key you want\u{2026}");
            }
        });
    }
    {
        let (path, key_row, on_error) = (path.clone(), key_row.clone(), on_error.clone());
        key_clear.connect_clicked(move |_| {
            match update(&path, |s| s.overlay_key = String::new()) {
                Ok(()) => key_row.set_subtitle(&keys::key_label("")),
                Err(e) => on_error(&format!("Could not save: {e}")),
            }
        });
    }
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (key_listening, key_row, key_change) =
            (key_listening.clone(), key_row.clone(), key_change.clone());
        let (path, on_error) = (path.clone(), on_error.clone());
        controller.connect_key_pressed(move |_, _keyval, keycode, _state| {
            if !key_listening.get() {
                return gtk::glib::Propagation::Proceed;
            }
            key_listening.set(false);
            key_change.set_label("Change");
            let Some(code) = keys::code_from_desktop_keycode(keycode) else {
                key_row.set_subtitle(&keys::key_label(&saved_key(&path)));
                return gtk::glib::Propagation::Stop;
            };
            let text = keys::binding_text(code);
            match update(&path, |s| s.overlay_key.clone_from(&text)) {
                Ok(()) => key_row.set_subtitle(&keys::key_label(&text)),
                Err(e) => {
                    on_error(&format!("Could not save: {e}"));
                    key_row.set_subtitle(&keys::key_label(&saved_key(&path)));
                }
            }
            gtk::glib::Propagation::Stop
        });
    }

    // --- the controller button ---
    let pad_subtitle = {
        let devices = hooks.devices.clone();
        Rc::new(move |saved: &str| {
            let mut text = keys::button_label(saved);
            match devices() {
                Some((_, keys::KeyAccess::NoPermission)) => {
                    text.push_str("  \u{00B7}  no permission to read controllers yet");
                }
                Some((d, _)) if d.controllers == 0 => {
                    text.push_str("  \u{00B7}  no controller detected");
                }
                _ => {}
            }
            text
        })
    };
    let pad_row = adw::ActionRow::builder()
        .title("Controller button")
        .subtitle(pad_subtitle(&current.overlay_button))
        .build();
    let pad_change = gtk::Button::builder()
        .label("Change")
        .valign(gtk::Align::Center)
        .build();
    let pad_clear = gtk::Button::builder()
        .label("Clear")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    pad_row.add_suffix(&pad_change);
    pad_row.add_suffix(&pad_clear);
    group.add(&pad_row);

    let pad_listening = Rc::new(Cell::new(false));
    {
        let (pad_listening, pad_row, pad_change) =
            (pad_listening.clone(), pad_row.clone(), pad_change.clone());
        let (path, on_error, hooks_capture, hooks_cancel, subtitle) = (
            path.clone(),
            on_error.clone(),
            hooks.capture_button.clone(),
            hooks.cancel_capture.clone(),
            pad_subtitle.clone(),
        );
        pad_change.clone().connect_clicked(move |_| {
            if pad_listening.get() {
                pad_listening.set(false);
                hooks_cancel();
                pad_change.set_label("Change");
                pad_row.set_subtitle(&subtitle(&saved_button(&path)));
                return;
            }
            pad_listening.set(true);
            pad_change.set_label("Cancel");
            pad_row.set_subtitle("Press a button on your controller\u{2026}");
            let (pad_listening, pad_row, pad_change) =
                (pad_listening.clone(), pad_row.clone(), pad_change.clone());
            let (path, on_error, subtitle) = (path.clone(), on_error.clone(), subtitle.clone());
            hooks_capture(Box::new(move |code| {
                pad_listening.set(false);
                pad_change.set_label("Change");
                if let Some(code) = code {
                    let text = format!("code:{code}");
                    if let Err(e) = update(&path, |s| s.overlay_button.clone_from(&text)) {
                        on_error(&format!("Could not save: {e}"));
                    }
                } else {
                    on_error("No button was pressed. Is the controller connected and the permission set?");
                }
                pad_row.set_subtitle(&subtitle(&saved_button(&path)));
            }));
        });
    }
    {
        let (path, pad_row, on_error, subtitle) = (
            path.clone(),
            pad_row.clone(),
            on_error.clone(),
            pad_subtitle.clone(),
        );
        let cancel = hooks.cancel_capture.clone();
        pad_clear.connect_clicked(move |_| {
            cancel();
            match update(&path, |s| s.overlay_button = String::new()) {
                Ok(()) => pad_row.set_subtitle(&subtitle("")),
                Err(e) => on_error(&format!("Could not save: {e}")),
            }
        });
    }
    (group, controller)
}

fn saved_key(path: &Path) -> String {
    Settings::load(path)
        .map(|s| s.overlay_key)
        .unwrap_or_default()
}

fn saved_button(path: &Path) -> String {
    Settings::load(path)
        .map(|s| s.overlay_button)
        .unwrap_or_default()
}
