//! The match list on the left: one calm row per match with a thin colour stripe for the result.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::MatchRow;

use crate::format::{clock_text, mode_name, result_class, score_text, when_label};

pub struct MatchList {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListBox,
    /// The match id of every row, in the same order as the rows.
    ids: RefCell<Vec<i64>>,
    /// True while we rebuild the list, so clearing it is not mistaken for a click.
    rebuilding: Cell<bool>,
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(classes.iter().map(|c| c.to_string()).collect::<Vec<_>>())
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build()
}

fn row_widget(row: &MatchRow, names: &PlaylistMap) -> gtk::ListBoxRow {
    let stripe = gtk::Box::builder()
        .css_classes(["stripe", result_class(&row.result)])
        .width_request(4)
        .build();

    let mode = label(&mode_name(names, row.playlist_id), &["row-mode"]);
    let when = label(&when_label(row.ended_at), &["row-when"]);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(&mode);
    text.append(&when);

    // A short note when something is unusual about the match.
    let note = if let Some(clock) = row.abandoned_at {
        Some(format!("Left with {} on the clock", clock_text(clock)))
    } else if row.overtime {
        Some("Overtime".to_string())
    } else {
        None
    };
    if let Some(note) = note {
        text.append(&label(&note, &["row-note"]));
    }

    let score = gtk::Label::builder()
        .label(score_text(row))
        .css_classes(["row-score", result_class(&row.result)])
        .valign(gtk::Align::Center)
        .build();

    let content = gtk::Box::builder().spacing(14).build();
    content.append(&stripe);
    content.append(&text);
    content.append(&score);
    content.set_margin_end(16);

    gtk::ListBoxRow::builder()
        .child(&content)
        .css_classes(["match-row"])
        .build()
}

impl MatchList {
    /// `on_select` is called with the match id when the user picks a row.
    pub fn new(on_select: impl Fn(i64) + 'static) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["match-list"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let this = Rc::new(Self {
            widget,
            list,
            ids: RefCell::new(Vec::new()),
            rebuilding: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_selected(move |_, row| {
            let (Some(this), Some(row)) = (weak.upgrade(), row) else {
                return;
            };
            if this.rebuilding.get() {
                return;
            }
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|i| this.ids.borrow().get(i).copied());
            if let Some(id) = id {
                on_select(id);
            }
        });
        this
    }

    /// Fills the list and selects `keep` if it is still there, otherwise the newest match.
    /// Returns the id that ended up selected.
    pub fn set_rows(
        &self,
        rows: &[MatchRow],
        names: &PlaylistMap,
        keep: Option<i64>,
    ) -> Option<i64> {
        self.rebuilding.set(true);
        self.list.remove_all();
        *self.ids.borrow_mut() = rows.iter().map(|r| r.id).collect();
        for row in rows {
            self.list.append(&row_widget(row, names));
        }
        let wanted = keep
            .and_then(|id| rows.iter().position(|r| r.id == id))
            .or(if rows.is_empty() { None } else { Some(0) });
        if let Some(i) = wanted {
            if let Some(row) = self.list.row_at_index(i as i32) {
                self.list.select_row(Some(&row));
            }
        }
        self.rebuilding.set(false);
        wanted.map(|i| rows[i].id)
    }
}
