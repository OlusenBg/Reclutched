//! The list on the left: a Today group with its matches, one row per earlier day, and groups for months and
//! years (see `history.rs` for what goes where). Rows have a thin colour stripe.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use crate::history::{Entry, Target};

pub struct HistoryList {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListBox,
    /// The entry behind every row, in the same order as the rows.
    entries: RefCell<Vec<Entry>>,
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

/// One row for any entry. Also used for the lists inside a day, month or year page.
pub fn entry_row(entry: &Entry) -> gtk::ListBoxRow {
    let stripe = gtk::Box::builder()
        .css_classes(["stripe", entry.tone.css_class()])
        .width_request(4)
        .build();

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(&label(&entry.title, &["row-mode"]));
    if !entry.subtitle.is_empty() {
        text.append(&label(&entry.subtitle, &["row-when"]));
    }

    let content = gtk::Box::builder().spacing(14).build();
    content.set_margin_end(16);
    content.set_margin_start(i32::from(entry.depth) * 22);
    content.append(&stripe);
    content.append(&text);
    if !entry.right.is_empty() {
        content.append(
            &gtk::Label::builder()
                .label(&entry.right)
                .css_classes(["row-score", entry.tone.css_class()])
                .valign(gtk::Align::Center)
                .build(),
        );
    }
    if let Some(group) = &entry.group {
        content.append(
            &gtk::Image::builder()
                .icon_name(if group.expanded {
                    "pan-down-symbolic"
                } else {
                    "pan-end-symbolic"
                })
                .valign(gtk::Align::Center)
                .css_classes(["row-chevron"])
                .build(),
        );
    }

    gtk::ListBoxRow::builder()
        .child(&content)
        .css_classes(["match-row"])
        .build()
}

impl HistoryList {
    /// `on_select` is called when the user picks a row; `on_toggle` with a group's key when a
    /// month or year row is clicked (to open or close it).
    pub fn new(
        on_select: impl Fn(Target) + 'static,
        on_toggle: impl Fn(String) + 'static,
    ) -> Rc<Self> {
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
            entries: RefCell::new(Vec::new()),
            rebuilding: Cell::new(false),
        });
        let entry_at = |this: &Rc<Self>, row: &gtk::ListBoxRow| {
            usize::try_from(row.index())
                .ok()
                .and_then(|i| this.entries.borrow().get(i).cloned())
        };
        {
            let weak = Rc::downgrade(&this);
            this.list.connect_row_selected(move |_, row| {
                let (Some(this), Some(row)) = (weak.upgrade(), row) else {
                    return;
                };
                if this.rebuilding.get() {
                    return;
                }
                if let Some(entry) = entry_at(&this, row) {
                    on_select(entry.target);
                }
            });
        }
        {
            let weak = Rc::downgrade(&this);
            this.list.connect_row_activated(move |_, row| {
                let Some(this) = weak.upgrade() else { return };
                if let Some(group) = entry_at(&this, row).and_then(|e| e.group) {
                    on_toggle(group.key);
                }
            });
        }
        this
    }

    /// Fills the list and highlights the row for `selected`. When that row is not in the list
    /// (for example a match from another day, opened from a page) nothing is highlighted.
    pub fn set_entries(&self, entries: Vec<Entry>, selected: &Target) {
        self.rebuilding.set(true);
        self.list.remove_all();
        for entry in &entries {
            self.list.append(&entry_row(entry));
        }
        let row = entries
            .iter()
            .position(|e| e.target == *selected)
            .and_then(|i| self.list.row_at_index(i as i32));
        match row {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        *self.entries.borrow_mut() = entries;
        self.rebuilding.set(false);
    }
}
