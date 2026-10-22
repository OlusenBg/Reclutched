//! The in-game overlay: click-through windows drawn above everything on Wayland (layer-shell).
//! They never take mouse or keyboard input, so the game keeps working underneath.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use reclutched_core::rocketleague::storage::MovementAverages;
use reclutched_core::rocketleague::tracker::LiveSnapshot;

use crate::panel;
use crate::tracking::Status;

/// What the little pill in the corner says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillState {
    /// Connected to the game's Stats API: matches are being tracked.
    Tracking,
    /// The game is running but its Stats API is not answering.
    StatsApiOff,
    /// Tracking was switched off in the app.
    TrackingOff,
}

impl PillState {
    fn text(self) -> &'static str {
        match self {
            PillState::Tracking => "Tracking",
            PillState::StatsApiOff => "Stats API off",
            PillState::TrackingOff => "Tracking off",
        }
    }

    fn class(self) -> &'static str {
        match self {
            PillState::Tracking => "ok",
            PillState::StatsApiOff => "bad",
            PillState::TrackingOff => "off",
        }
    }
}

/// What the pill shows: the game window must have the focus (when we can tell), and the text
/// depends on whether the tracker is connected. `None` hides the pill.
pub fn pill_state(status: Status, game_focused: Option<bool>) -> Option<PillState> {
    match (game_focused, status) {
        (Some(false), _) => None,
        (Some(true), Status::Active) => Some(PillState::Tracking),
        (Some(true), Status::Waiting) => Some(PillState::StatsApiOff),
        (Some(true), Status::Off) => Some(PillState::TrackingOff),
        // We cannot tell which window has the focus: only say something when the game is connected.
        (None, Status::Active) => Some(PillState::Tracking),
        (None, _) => None,
    }
}

pub struct Overlay {
    pill: gtk::Window,
    dot: gtk::Label,
    text: gtk::Label,
    /// The big panel shown while the overlay key is held.
    panel: gtk::Window,
    panel_root: gtk::Overlay,
    panel_pieces: RefCell<Vec<gtk::Widget>>,
}

/// How see-through the pill is (1.0 = solid).
const PILL_OPACITY: f64 = 0.75;

/// Whether this desktop can draw the overlay (Wayland with the layer-shell protocol).
pub fn supported() -> bool {
    gtk4_layer_shell::is_supported()
}

/// Makes a window an overlay layer: above everything, no focus, and mouse clicks fall through.
fn make_overlay_layer(window: &gtk::Window) {
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("reclutched-overlay"));
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("overlay-window");
    window.set_decorated(false);
    window.set_focusable(false);
    // An empty input region: every click goes to the window below.
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(&gtk::cairo::Region::create());
        }
    });
}

impl Overlay {
    /// `None` when the desktop cannot draw an overlay.
    pub fn new(app: &adw::Application) -> Option<Rc<Self>> {
        if !supported() {
            return None;
        }
        let dot = gtk::Label::builder()
            .label("\u{25CF}")
            .css_classes(["pill-dot"])
            .build();
        let text = gtk::Label::builder().css_classes(["pill-text"]).build();
        let row = gtk::Box::builder()
            .spacing(8)
            .css_classes(["overlay-pill"])
            .build();
        row.append(&dot);
        row.append(&text);

        let pill = gtk::Window::builder()
            .application(app)
            .title("Reclutched overlay")
            .child(&row)
            .resizable(false)
            .build();
        make_overlay_layer(&pill);
        pill.set_anchor(Edge::Bottom, true);
        pill.set_anchor(Edge::Left, true);
        pill.set_margin(Edge::Bottom, 28);
        pill.set_margin(Edge::Left, 28);
        pill.set_opacity(PILL_OPACITY);

        // The panel covers the whole screen; only its cards are drawn, the rest stays see-through.
        let panel_root = gtk::Overlay::new();
        panel_root.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        let panel = gtk::Window::builder()
            .application(app)
            .title("Reclutched overlay panel")
            .child(&panel_root)
            .build();
        make_overlay_layer(&panel);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            panel.set_anchor(edge, true);
        }
        panel.set_exclusive_zone(-1);

        Some(Rc::new(Self {
            pill,
            dot,
            text,
            panel,
            panel_root,
            panel_pieces: RefCell::new(Vec::new()),
        }))
    }

    /// Shows (or refreshes) the panel for the match in `snapshot`; `None` means no match is on.
    pub fn show_panel(&self, snapshot: Option<&LiveSnapshot>, usual: Option<&MovementAverages>) {
        for old in self.panel_pieces.borrow_mut().drain(..) {
            self.panel_root.remove_overlay(&old);
        }
        let pieces = panel::build(snapshot, usual);
        for piece in &pieces {
            self.panel_root.add_overlay(piece);
        }
        *self.panel_pieces.borrow_mut() = pieces;
        if !self.panel.is_visible() {
            self.panel.present();
        }
    }

    pub fn hide_panel(&self) {
        self.panel.set_visible(false);
    }

    /// Shows the pill with this state, or hides it for `None`.
    pub fn set_pill(&self, state: Option<PillState>) {
        let Some(state) = state else {
            self.pill.set_visible(false);
            return;
        };
        self.text.set_label(state.text());
        for class in ["ok", "bad", "off"] {
            self.dot.remove_css_class(class);
        }
        self.dot.add_css_class(state.class());
        if !self.pill.is_visible() {
            self.pill.present();
        }
    }

    pub fn close(&self) {
        self.pill.destroy();
        self.panel.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_follows_focus_and_connection() {
        use PillState::*;
        // Game focused: say whether we are connected.
        assert_eq!(pill_state(Status::Active, Some(true)), Some(Tracking));
        assert_eq!(pill_state(Status::Waiting, Some(true)), Some(StatsApiOff));
        assert_eq!(pill_state(Status::Off, Some(true)), Some(TrackingOff));
        // Something else focused: nothing on screen.
        for status in [Status::Active, Status::Waiting, Status::Off] {
            assert_eq!(pill_state(status, Some(false)), None);
        }
        // Unknown focus: only a connected game is worth a pill.
        assert_eq!(pill_state(Status::Active, None), Some(Tracking));
        assert_eq!(pill_state(Status::Waiting, None), None);
        assert_eq!(pill_state(Status::Off, None), None);
    }
}
