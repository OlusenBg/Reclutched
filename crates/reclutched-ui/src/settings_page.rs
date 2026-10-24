//! The settings page: a profile, the display mode and every tracking switch.
//! Changes are saved as soon as you make them.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};
use reclutched_core::rocketleague::statsapi;

use crate::capture;
use crate::flow::{username_error, ADV_EVENTS, ADV_HABITS, ADV_MATCHES};
use crate::keys;
use crate::setup::{switch_subtitle, switch_title};

/// The Hyprland (0.55+, Lua config) rule that draws our window fully opaque. See the README.
const HYPRLAND_OPAQUE_RULE: &str =
    r#"hl.window_rule({ match = { class = "^dev\\.reclutched\\.Reclutched$" }, opaque = true })"#;

const POPUP_POSITIONS: [(&str, &str); 3] = [
    ("top", "Top, centred"),
    ("top-left", "Top left"),
    ("top-right", "Top right"),
];

const CORNERS: [(&str, &str); 4] = [
    ("bottom-left", "Bottom left"),
    ("bottom-right", "Bottom right"),
    ("top-left", "Top left"),
    ("top-right", "Top right"),
];

const MODES: [(DisplayMode, &str); 3] = [
    (DisplayMode::Overlay, "Overlay"),
    (DisplayMode::TwoScreens, "2 Screens"),
    (DisplayMode::Both, "Both"),
];

/// Loads the file, lets `change` edit it and saves it again. Reading first means we never
/// overwrite something that changed elsewhere (for example the player id `track` learned).
pub(crate) fn update(
    path: &std::path::Path,
    change: impl FnOnce(&mut Settings),
) -> Result<(), String> {
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

/// What the Capture section needs from the app.
#[derive(Clone)]
pub struct CaptureHooks {
    pub status: Rc<dyn Fn() -> capture::Status>,
    /// Turns capturing on or off (on: the desktop asks what to share).
    pub turn: Rc<dyn Fn(bool)>,
    /// Forgets what you chose to share, then asks again.
    pub choose_again: Rc<dyn Fn()>,
    pub screenshot: Rc<dyn Fn()>,
    /// Shows an example of the save pop-up.
    pub preview: Rc<dyn Fn()>,
    /// What the recording is doing right now.
    pub recording: Rc<dyn Fn() -> String>,
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
    pub capture: CaptureHooks,
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
    let keybinds_page = SectionPage::new("Keybinds");
    let overlay_page = SectionPage::new("Overlay");
    let capture_page = SectionPage::new("Capture");
    let game_page = SectionPage::new("Rocket League");
    let track_page = SectionPage::new("Tracking");
    let data_page = SectionPage::new("Data");
    let about_page = SectionPage::new("About");

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
    // --- keybinds: every key and controller button in one place ---
    let note = adw::PreferencesGroup::builder()
        .description("Set a keyboard key, a controller button, or both; either one works. Reading them needs permission once: run  sudo usermod -aG input $USER  and log out and in again. Other keys and buttons are never read, stored or sent anywhere.")
        .build();
    keybinds_page.add(&note);
    let (overlay_keys, key_controller) = binding_group(
        "Show the overlay (hold)",
        "Hold it in the game to show the panel; let go to hide it.",
        OVERLAY_BINDING,
        &path,
        &current,
        &on_error,
        &hooks,
    );
    keybinds_page.add(&overlay_keys);
    let (shot_keys, shot_controller) = binding_group(
        "Take a screenshot",
        "Press it to save a picture of the game (turn on Capture first).",
        SCREENSHOT_BINDING,
        &path,
        &current,
        &on_error,
        &hooks,
    );
    keybinds_page.add(&shot_keys);
    let (clip_keys, clip_controller) = binding_group(
        "Save a clip",
        "Press it to save the last seconds as a clip (turn on Capture first; clips need Manual or Both).",
        CLIP_BINDING,
        &path,
        &current,
        &on_error,
        &hooks,
    );
    keybinds_page.add(&clip_keys);

    // --- startup (General) ---
    general_page.add(&startup_group(&path, &current, &on_error));

    // --- overlay look ---
    for group in overlay_groups(&path, &current, &on_error) {
        overlay_page.add(&group);
    }

    capture_section(&capture_page, parent, &path, &current, &on_error, &hooks);
    for group in recording_groups(&path, &current, &on_error, &hooks) {
        capture_page.add(&group);
    }

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
    let tracking = adw::PreferencesGroup::builder()
        .title("While you play")
        .build();
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
    general_page.add(&tracking);

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
    about_page.add(&about_group());
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
    let sections: [(&str, &str, &SectionPage); 8] = [
        ("general", "General", &general_page),
        ("keybinds", "Keybinds", &keybinds_page),
        ("overlay", "Overlay", &overlay_page),
        ("capture", "Capture", &capture_page),
        ("game", "Rocket League", &game_page),
        ("track", "Tracking", &track_page),
        ("data", "Data", &data_page),
        ("about", "About", &about_page),
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
    view.add_controller(shot_controller);
    view.add_controller(clip_controller);
    view.upcast()
}

/// The Capture section: the switch, what is captured, the screenshot key and where files go.
fn capture_section(
    page: &SectionPage,
    parent: &adw::ApplicationWindow,
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
    hooks: &Hooks,
) {
    let group = adw::PreferencesGroup::builder()
        .title("Screenshots and clips")
        .description("Your desktop asks once what to share (your screen or the game window). Reclutched only records while this is on.")
        .build();
    let switch = adw::SwitchRow::builder()
        .title("Capturing")
        .subtitle("Turn it on to take screenshots")
        .active(current.capture_enabled)
        .build();
    let status = adw::ActionRow::builder().title("Status").build();
    let choose_again = gtk::Button::builder()
        .label("Choose again")
        .valign(gtk::Align::Center)
        .build();
    status.add_suffix(&choose_again);
    let shot_now = adw::ActionRow::builder()
        .title("Take a screenshot now")
        .subtitle("A test: the same as pressing the key")
        .activatable(true)
        .build();
    shot_now.add_suffix(&gtk::Image::from_icon_name("camera-photo-symbolic"));
    group.add(&switch);
    group.add(&status);
    group.add(&shot_now);
    page.add(&group);

    let show = {
        let (status, status_fn) = (status.clone(), hooks.capture.status.clone());
        Rc::new(move || {
            let (text, ready) = match status_fn() {
                capture::Status::Off => ("Off".to_string(), false),
                capture::Status::Asking => (
                    "Waiting for your desktop to ask what to share\u{2026}".to_string(),
                    false,
                ),
                capture::Status::Ready(what) => (format!("Ready, capturing {what}"), true),
                capture::Status::Failed(e) => (format!("Could not start: {e}"), false),
            };
            status.set_subtitle(&text);
            ready
        })
    };
    show();
    {
        let (path, on_error, turn, show) = (
            path.clone(),
            on_error.clone(),
            hooks.capture.turn.clone(),
            show.clone(),
        );
        switch.connect_active_notify(move |row| {
            let on = row.is_active();
            if let Err(e) = update(&path, |s| s.capture_enabled = on) {
                on_error(&format!("Could not save: {e}"));
            }
            turn(on);
            show();
        });
    }
    {
        let again = hooks.capture.choose_again.clone();
        choose_again.connect_clicked(move |_| again());
    }
    {
        let shot = hooks.capture.screenshot.clone();
        shot_now.connect_activated(move |_| shot());
    }
    // Keep the status line up to date while the page is open.
    {
        let (status, show) = (status.downgrade(), show);
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(600), move || {
            if status.upgrade().is_none() {
                return gtk::glib::ControlFlow::Break;
            }
            show();
            gtk::glib::ControlFlow::Continue
        });
    }

    // --- the pop-up when something is saved ---
    let popup = adw::PreferencesGroup::builder()
        .title("Save pop-up")
        .description("A small card slides down from the top of the screen, over the game, when a screenshot, a clip or a match is saved.")
        .build();
    let show_row = adw::SwitchRow::builder()
        .title("Show the pop-up")
        .active(current.notify_enabled)
        .build();
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        show_row.connect_active_notify(move |row| {
            let on = row.is_active();
            if let Err(e) = update(&path, |s| s.notify_enabled = on) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    popup.add(&show_row);
    let seconds_row = adw::SpinRow::with_range(2.0, 20.0, 1.0);
    seconds_row.set_title("How long it stays");
    seconds_row.set_subtitle("Seconds");
    seconds_row.set_value(f64::from(current.notify_seconds.clamp(2, 20)));
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        seconds_row.connect_value_notify(move |row| {
            let seconds = row.value() as u32;
            if let Err(e) = update(&path, |s| s.notify_seconds = seconds) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    popup.add(&seconds_row);
    let position_row = adw::ComboRow::builder()
        .title("Where")
        .model(&gtk::StringList::new(
            &POPUP_POSITIONS.map(|(_, name)| name),
        ))
        .build();
    position_row.set_selected(
        POPUP_POSITIONS
            .iter()
            .position(|(key, _)| *key == current.notify_position)
            .unwrap_or(0) as u32,
    );
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        position_row.connect_selected_notify(move |row| {
            let Some((key, _)) = POPUP_POSITIONS.get(row.selected() as usize) else {
                return;
            };
            if let Err(e) = update(&path, |s| s.notify_position = (*key).to_string()) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    popup.add(&position_row);
    let sound_row = adw::SwitchRow::builder()
        .title("Play a sound")
        .subtitle("A short system sound when something is saved")
        .active(current.notify_sound)
        .build();
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        sound_row.connect_active_notify(move |row| {
            let on = row.is_active();
            if let Err(e) = update(&path, |s| s.notify_sound = on) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    popup.add(&sound_row);
    let preview_row = adw::ActionRow::builder()
        .title("Show an example")
        .subtitle("See what it looks like with these settings")
        .activatable(true)
        .build();
    preview_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let preview = hooks.capture.preview.clone();
        preview_row.connect_activated(move |_| preview());
    }
    popup.add(&preview_row);
    page.add(&popup);

    // --- where files go ---
    let files = adw::PreferencesGroup::builder().title("Saving").build();
    let folder_text = |s: &Settings| {
        s.capture_folder()
            .map_or("(unknown)".to_string(), |f| f.display().to_string())
    };
    let folder_row = adw::ActionRow::builder()
        .title("Folder")
        .subtitle(folder_text(current))
        .subtitle_selectable(true)
        .build();
    let choose = gtk::Button::builder()
        .label("Choose\u{2026}")
        .valign(gtk::Align::Center)
        .build();
    folder_row.add_suffix(&choose);
    files.add(&folder_row);
    {
        let (path, on_error, parent, folder_row) = (
            path.clone(),
            on_error.clone(),
            parent.clone(),
            folder_row.clone(),
        );
        choose.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Where should screenshots and clips go?")
                .build();
            let (path, on_error, folder_row) = (path.clone(), on_error.clone(), folder_row.clone());
            dialog.select_folder(Some(&parent), gtk::gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let Some(chosen) = file.path() else { return };
                match update(&path, |s| s.capture_dir = Some(chosen.clone())) {
                    Ok(()) => folder_row.set_subtitle(&chosen.display().to_string()),
                    Err(e) => on_error(&format!("Could not save: {e}")),
                }
            });
        });
    }
    page.add(&files);

    // --- what has to be installed ---
    let needs = adw::PreferencesGroup::builder()
        .title("Requirements")
        .build();
    let (text, fine) = match capture::gst::check_elements(&capture::SCREENSHOT_ELEMENTS) {
        Ok(()) => (
            "GStreamer is installed with everything needed".to_string(),
            true,
        ),
        Err(e) => (e, false),
    };
    let row = adw::ActionRow::builder()
        .title(if fine {
            "GStreamer"
        } else {
            "GStreamer needs attention"
        })
        .subtitle(text)
        .subtitle_selectable(true)
        .build();
    needs.add(&row);
    page.add(&needs);
}

/// Starting with the computer.
fn startup_group(
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Startup")
        .description("Open Reclutched when you log in, so tracking and the overlay are ready before you start the game.")
        .build();
    let login = adw::SwitchRow::builder()
        .title("Start with my computer")
        .active(current.start_on_login)
        .build();
    let minimized = adw::SwitchRow::builder()
        .title("Start minimized")
        .subtitle("Tracking and the overlay run; the window stays out of the way")
        .active(current.start_minimized)
        .sensitive(current.start_on_login)
        .build();
    {
        let (path, on_error, minimized) = (path.clone(), on_error.clone(), minimized.clone());
        login.connect_active_notify(move |row| {
            let on = row.is_active();
            minimized.set_sensitive(on);
            let mini = minimized.is_active();
            let result = crate::startup::set_autostart(on, mini)
                .and_then(|()| update(&path, |s| s.start_on_login = on).map_err(|e| e.to_string()));
            if let Err(e) = result {
                on_error(&format!("Could not change startup: {e}"));
            }
        });
    }
    {
        let (path, on_error, login) = (path.clone(), on_error.clone(), login.clone());
        minimized.connect_active_notify(move |row| {
            let mini = row.is_active();
            let result = crate::startup::set_autostart(login.is_active(), mini).and_then(|()| {
                update(&path, |s| s.start_minimized = mini).map_err(|e| e.to_string())
            });
            if let Err(e) = result {
                on_error(&format!("Could not change startup: {e}"));
            }
        });
    }
    group.add(&login);
    group.add(&minimized);
    group
}

/// The overlay's look: the pill and the parts of the panel, plus the window hints.
fn overlay_groups(
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
) -> Vec<adw::PreferencesGroup> {
    let pill = adw::PreferencesGroup::builder()
        .title("Status light")
        .description("The small \"Tracking\" light in the corner of the game.")
        .build();
    let corner_row = adw::ComboRow::builder()
        .title("Corner")
        .model(&gtk::StringList::new(&CORNERS.map(|(_, name)| name)))
        .build();
    corner_row.set_selected(
        CORNERS
            .iter()
            .position(|(key, _)| *key == current.overlay_corner)
            .unwrap_or(0) as u32,
    );
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        corner_row.connect_selected_notify(move |row| {
            let Some((key, _)) = CORNERS.get(row.selected() as usize) else {
                return;
            };
            if let Err(e) = update(&path, |s| s.overlay_corner = (*key).to_string()) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    pill.add(&corner_row);
    let opacity_row = adw::ActionRow::builder()
        .title("How solid")
        .subtitle("Lower is more see-through")
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 30.0, 100.0, 5.0);
    scale.set_value(f64::from(current.overlay_opacity.clamp(30, 100)));
    scale.set_width_request(220);
    scale.set_valign(gtk::Align::Center);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        scale.connect_value_changed(move |scale| {
            let value = scale.value().round() as u8;
            if let Err(e) = update(&path, |s| s.overlay_opacity = value) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    opacity_row.add_suffix(&scale);
    pill.add(&opacity_row);

    let panel = adw::PreferencesGroup::builder()
        .title("Panel during a match")
        .description("What the panel shows while you hold the key in a match.")
        .build();
    type Setter = fn(&mut Settings, bool);
    let parts: [(&str, &str, bool, Setter); 3] = [
        (
            "Possession bar",
            "Who has the ball, across the top",
            current.overlay_show_possession,
            |s, v| s.overlay_show_possession = v,
        ),
        (
            "Your habits",
            "Boost and speed compared with your usual, on the left",
            current.overlay_show_habits,
            |s, v| s.overlay_show_habits = v,
        ),
        (
            "Players",
            "The scoreboard of the match, on the right",
            current.overlay_show_players,
            |s, v| s.overlay_show_players = v,
        ),
    ];
    for (title, subtitle, active, set) in parts {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(active)
            .build();
        let (path, on_error) = (path.clone(), on_error.clone());
        row.connect_active_notify(move |row| {
            let on = row.is_active();
            if let Err(e) = update(&path, |s| set(s, on)) {
                on_error(&format!("Could not save: {e}"));
            }
        });
        panel.add(&row);
    }

    let window = adw::PreferencesGroup::builder()
        .title("Game window")
        .description("The overlay only shows on the Rocket League window. If yours is not recognised, add a word from its name here (separate several with commas).")
        .build();
    let hints_row = adw::EntryRow::builder()
        .title("Extra window names")
        .show_apply_button(true)
        .text(current.game_window_hints.join(", "))
        .build();
    {
        let (path, on_error) = (path.clone(), on_error.clone());
        hints_row.connect_apply(move |row| {
            let hints: Vec<String> = row
                .text()
                .split(',')
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
                .collect();
            if let Err(e) = update(&path, |s| s.game_window_hints = hints) {
                on_error(&format!("Could not save: {e}"));
            }
        });
    }
    window.add(&hints_row);
    vec![pill, panel, window]
}

/// Version, licence and what the app promises about your data.
fn about_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Reclutched").build();
    for (title, value) in [
        ("Version", env!("CARGO_PKG_VERSION").to_string()),
        ("Licence", "GNU GPL v3 or later".to_string()),
        (
            "Privacy",
            "Only the game's official local Stats API is used. Everything stays on this computer."
                .to_string(),
        ),
    ] {
        group.add(
            &adw::ActionRow::builder()
                .title(title)
                .subtitle(value)
                .subtitle_selectable(true)
                .build(),
        );
    }
    let source = adw::ActionRow::builder()
        .title("Source code")
        .subtitle("github.com/OlusenBg/Reclutched")
        .activatable(true)
        .build();
    source.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    source.connect_activated(|_| {
        gtk::UriLauncher::new("https://github.com/OlusenBg/Reclutched").launch(
            None::<&gtk::Window>,
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
    });
    group.add(&source);
    group
}

/// Where one saved binding lives in the settings.
#[derive(Clone, Copy)]
struct Field {
    get: fn(&Settings) -> String,
    set: fn(&mut Settings, String),
}

/// A row that picks one of a few saved words.
fn combo_row(
    title: &str,
    subtitle: &str,
    options: &'static [(&'static str, &'static str)],
    selected: &str,
    path: &Rc<PathBuf>,
    on_error: &Rc<dyn Fn(&str)>,
    set: fn(&mut Settings, String),
) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(
            &options.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.set_selected(
        options
            .iter()
            .position(|(key, _)| *key == selected)
            .unwrap_or(0) as u32,
    );
    let (path, on_error) = (path.clone(), on_error.clone());
    row.connect_selected_notify(move |row| {
        let Some((key, _)) = options.get(row.selected() as usize) else {
            return;
        };
        if let Err(e) = update(&path, |s| set(s, (*key).to_string())) {
            on_error(&format!("Could not save: {e}"));
        }
    });
    row
}

/// A row with an on/off switch.
fn switch_row(
    title: &str,
    subtitle: &str,
    active: bool,
    path: &Rc<PathBuf>,
    on_error: &Rc<dyn Fn(&str)>,
    set: fn(&mut Settings, bool),
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(active)
        .build();
    let (path, on_error) = (path.clone(), on_error.clone());
    row.connect_active_notify(move |row| {
        let on = row.is_active();
        if let Err(e) = update(&path, |s| set(s, on)) {
            on_error(&format!("Could not save: {e}"));
        }
    });
    row
}

/// A row with a number and plus/minus buttons.
#[allow(clippy::too_many_arguments)]
fn spin_row(
    title: &str,
    subtitle: &str,
    range: (u32, u32),
    value: u32,
    path: &Rc<PathBuf>,
    on_error: &Rc<dyn Fn(&str)>,
    set: fn(&mut Settings, u32),
) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(f64::from(range.0), f64::from(range.1), 1.0);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_value(f64::from(value.clamp(range.0, range.1)));
    let (path, on_error) = (path.clone(), on_error.clone());
    row.connect_value_notify(move |row| {
        let value = row.value() as u32;
        if let Err(e) = update(&path, |s| set(s, value)) {
            on_error(&format!("Could not save: {e}"));
        }
    });
    row
}

const RECORD_MODES: [(&str, &str); 2] = [
    ("clips", "Clips only"),
    ("full", "Every match, start to finish"),
];
const CLIP_MODES: [(&str, &str); 3] = [
    ("manual", "Manual (press a key)"),
    ("auto", "Automatic (goals, saves, ...)"),
    ("both", "Both"),
];
const QUALITIES: [(&str, &str); 3] = [
    ("low", "Low (720p, small files)"),
    ("medium", "Medium (1080p)"),
    ("high", "High (your screen's own size)"),
];
const FRAME_RATES: [(&str, &str); 2] = [("30", "30 per second"), ("60", "60 per second")];

/// Recording, clips and sound, for the Capture section.
fn recording_groups(
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
    hooks: &Hooks,
) -> Vec<adw::PreferencesGroup> {
    let recording = adw::PreferencesGroup::builder()
        .title("Recording")
        .description("While capturing is on and the game is connected, Reclutched keeps the last minute in a temporary folder, so a clip can start before you press the key.")
        .build();
    let status = adw::ActionRow::builder().title("Status").build();
    {
        let (weak, text) = (status.downgrade(), hooks.capture.recording.clone());
        status.set_subtitle(&text());
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(900), move || {
            let Some(row) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            row.set_subtitle(&text());
            gtk::glib::ControlFlow::Continue
        });
    }
    recording.add(&status);
    recording.add(&combo_row(
        "What to record",
        "Clips keep short moments; full matches record every match and save it when it ends",
        &RECORD_MODES,
        &current.record_mode,
        path,
        on_error,
        |s, v| s.record_mode = v,
    ));
    recording.add(&combo_row(
        "Picture quality",
        "",
        &QUALITIES,
        &current.capture_quality,
        path,
        on_error,
        |s, v| s.capture_quality = v,
    ));
    recording.add(&combo_row(
        "Frame rate",
        "",
        &FRAME_RATES,
        &current.capture_fps.to_string(),
        path,
        on_error,
        |s, v| s.capture_fps = v.parse().unwrap_or(60),
    ));
    recording.add(&spin_row(
        "Delete full recordings after (days)",
        "0 keeps them for ever",
        (0, 365),
        current.full_delete_days,
        path,
        on_error,
        |s, v| s.full_delete_days = v,
    ));

    let clips = adw::PreferencesGroup::builder()
        .title("Clips")
        .description("A clip is the last seconds before the moment (plus three seconds after, for automatic ones).")
        .build();
    clips.add(&combo_row(
        "How clips are made",
        "",
        &CLIP_MODES,
        &current.clip_mode,
        path,
        on_error,
        |s, v| s.clip_mode = v,
    ));
    clips.add(&spin_row(
        "How far back",
        "Seconds before the moment (10 to 50)",
        (10, 50),
        current.clip_seconds,
        path,
        on_error,
        |s, v| s.clip_seconds = v,
    ));
    for (title, subtitle, active, set) in [
        (
            "Your goals",
            "A clip every time you score",
            current.clip_on_goal,
            (|s: &mut Settings, v| s.clip_on_goal = v) as fn(&mut Settings, bool),
        ),
        (
            "Your saves",
            "Saves and epic saves",
            current.clip_on_save,
            |s, v| s.clip_on_save = v,
        ),
        (
            "Your demolitions",
            "When you demolish someone",
            current.clip_on_demo,
            |s, v| s.clip_on_demo = v,
        ),
        (
            "The end of the match",
            "The final moments, for example an overtime winner",
            current.clip_on_match_end,
            |s, v| s.clip_on_match_end = v,
        ),
    ] {
        clips.add(&switch_row(title, subtitle, active, path, on_error, set));
    }

    let sound = adw::PreferencesGroup::builder()
        .title("Sound")
        .description("What is in the recording besides the picture.")
        .build();
    sound.add(&switch_row(
        "Game sound",
        "Everything you hear from your computer",
        current.audio_game,
        path,
        on_error,
        |s, v| s.audio_game = v,
    ));
    sound.add(&switch_row(
        "Microphone",
        "Your voice, mixed with the game sound",
        current.audio_mic,
        path,
        on_error,
        |s, v| s.audio_mic = v,
    ));
    vec![recording, clips, sound]
}

/// A keyboard key and a controller button for one action. Both can be set; either one works.
#[derive(Clone, Copy)]
struct Binding {
    key: Field,
    button: Field,
}

const OVERLAY_BINDING: Binding = Binding {
    key: Field {
        get: |s| s.overlay_key.clone(),
        set: |s, v| s.overlay_key = v,
    },
    button: Field {
        get: |s| s.overlay_button.clone(),
        set: |s, v| s.overlay_button = v,
    },
};

const CLIP_BINDING: Binding = Binding {
    key: Field {
        get: |s| s.clip_key.clone(),
        set: |s, v| s.clip_key = v,
    },
    button: Field {
        get: |s| s.clip_button.clone(),
        set: |s, v| s.clip_button = v,
    },
};

const SCREENSHOT_BINDING: Binding = Binding {
    key: Field {
        get: |s| s.screenshot_key.clone(),
        set: |s, v| s.screenshot_key = v,
    },
    button: Field {
        get: |s| s.screenshot_button.clone(),
        set: |s, v| s.screenshot_button = v,
    },
};

/// The key and the controller button for one action, as two rows in a group. Returns the group
/// and the key controller that must be added to the page (it catches the next key press while
/// you are setting the key).
fn binding_group(
    title: &str,
    description: &str,
    fields: Binding,
    path: &Rc<PathBuf>,
    current: &Settings,
    on_error: &Rc<dyn Fn(&str)>,
    hooks: &Hooks,
) -> (adw::PreferencesGroup, gtk::EventControllerKey) {
    let group = adw::PreferencesGroup::builder()
        .title(title)
        .description(description)
        .build();

    // --- the keyboard key ---
    let key_row = adw::ActionRow::builder()
        .title("Keyboard key")
        .subtitle(keys::key_label(&(fields.key.get)(current)))
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
                key_row.set_subtitle(&keys::key_label(&saved(&path, fields.key.get)));
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
            match update(&path, |s| (fields.key.set)(s, String::new())) {
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
                key_row.set_subtitle(&keys::key_label(&saved(&path, fields.key.get)));
                return gtk::glib::Propagation::Stop;
            };
            let text = keys::binding_text(code);
            match update(&path, |s| (fields.key.set)(s, text.clone())) {
                Ok(()) => key_row.set_subtitle(&keys::key_label(&text)),
                Err(e) => {
                    on_error(&format!("Could not save: {e}"));
                    key_row.set_subtitle(&keys::key_label(&saved(&path, fields.key.get)));
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
        .subtitle(pad_subtitle(&(fields.button.get)(current)))
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
                pad_row.set_subtitle(&subtitle(&saved(&path, fields.button.get)));
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
                    if let Err(e) = update(&path, |s| (fields.button.set)(s, text.clone())) {
                        on_error(&format!("Could not save: {e}"));
                    }
                } else {
                    on_error("No button was pressed. Is the controller connected and the permission set?");
                }
                pad_row.set_subtitle(&subtitle(&saved(&path, fields.button.get)));
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
            match update(&path, |s| (fields.button.set)(s, String::new())) {
                Ok(()) => pad_row.set_subtitle(&subtitle("")),
                Err(e) => on_error(&format!("Could not save: {e}")),
            }
        });
    }
    (group, controller)
}

fn saved(path: &Path, get: fn(&Settings) -> String) -> String {
    Settings::load(path).map(|s| get(&s)).unwrap_or_default()
}
