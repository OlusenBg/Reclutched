//! Small building blocks shared by the detail view and the live window.

use adw::prelude::*;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let wraps = classes
        .iter()
        .any(|c| matches!(*c, "head-sub" | "muted-note" | "fact"));
    gtk::Label::builder()
        .label(text)
        .css_classes(classes.iter().map(|c| c.to_string()).collect::<Vec<_>>())
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(wraps)
        .build()
}

/// A player name that is cut off with "…" when it is very long (the full name shows on hover),
/// so one huge gamertag can never make the window wider than the screen.
pub fn name_label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = label(text, classes);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(22);
    l.set_tooltip_text(Some(text));
    l
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(spacing)
        .build()
}

pub fn section(title: &str) -> gtk::Label {
    let l = label(&title.to_uppercase(), &["section-label"]);
    l.set_margin_top(26);
    l
}

/// A muted one-liner for things we don't have (a switch was off, or the game didn't send it).
pub fn empty_note(text: &str) -> gtk::Label {
    let l = label(text, &["muted-note"]);
    l.set_margin_top(6);
    l
}

/// A big quiet number with a small caption.
pub fn tile(key: &str, value: &str, hint: &str) -> gtk::Box {
    let b = vbox(2);
    b.add_css_class("tile");
    b.set_hexpand(true);
    b.append(&label(&key.to_uppercase(), &["tile-key"]));
    b.append(&label(value, &["tile-val"]));
    let hint = label(hint, &["tile-hint"]);
    hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
    hint.set_max_width_chars(20);
    b.append(&hint);
    b
}

pub fn scroll_page(content: &gtk::Box) -> gtk::ScrolledWindow {
    content.set_margin_top(28);
    content.set_margin_bottom(32);
    content.set_margin_start(28);
    content.set_margin_end(28);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(content)
        .build();
    // Horizontal scrolling only appears if something is wider than the window. With `Never` the
    // width of the content would also set the minimum width of the whole window.
    gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_width(0)
        .vexpand(true)
        .build()
}

pub fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// A thin horizontal bar for a value between 0 and 1.
pub fn bar(fraction: f64) -> gtk::ProgressBar {
    gtk::ProgressBar::builder()
        .fraction(fraction.clamp(0.0, 1.0))
        .css_classes(["thin-bar"])
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build()
}

/// A bar split in three parts (own third, middle, enemy third) in proportion to the numbers.
pub fn thirds_bar(def: i64, mid: i64, att: i64) -> gtk::DrawingArea {
    let total = (def + mid + att).max(1) as f64;
    let parts = [def as f64 / total, mid as f64 / total, att as f64 / total];
    let area = gtk::DrawingArea::builder()
        .content_height(8)
        .hexpand(true)
        .build();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let colors = [(0.36, 0.30, 0.30), (0.55, 0.47, 0.47), (0.64, 0.15, 0.19)];
        let mut x = 0.0;
        for (part, (r, g, b)) in parts.iter().zip(colors) {
            let seg = (w * part - 2.0).max(0.0);
            if seg > 0.0 {
                cr.set_source_rgb(r, g, b);
                cr.rectangle(x, 0.0, seg, h);
                let _ = cr.fill();
            }
            x += w * part;
        }
    });
    area
}

/// A bar split in two parts, `first` (0..=100 percent) in `first_color` and the rest in
/// `second_color`. Colours are (r, g, b) in 0..=1.
pub fn split_bar(
    first: f64,
    first_color: (f64, f64, f64),
    second_color: (f64, f64, f64),
) -> gtk::DrawingArea {
    let first = (first / 100.0).clamp(0.0, 1.0);
    let area = gtk::DrawingArea::builder()
        .content_height(10)
        .hexpand(true)
        .build();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let cut = w * first;
        for (x0, x1, (r, g, b)) in [(0.0, cut, first_color), (cut, w, second_color)] {
            let gap = if x1 - x0 > 3.0 { 1.0 } else { 0.0 };
            if x1 - x0 > 0.0 {
                cr.set_source_rgb(r, g, b);
                cr.rectangle(x0 + gap, 0.0, (x1 - x0 - 2.0 * gap).max(0.0), h);
                let _ = cr.fill();
            }
        }
    });
    area
}

/// A caption on the left, a value on the right and a bar underneath.
pub fn meter(key: &str, value: &str, fraction: f64) -> gtk::Box {
    let b = vbox(6);
    let top = gtk::Box::builder().spacing(12).build();
    top.append(&label(&key.to_uppercase(), &["tile-key"]));
    let v = label(value, &["stat-small"]);
    v.set_hexpand(true);
    v.set_halign(gtk::Align::End);
    top.append(&v);
    b.append(&top);
    b.append(&bar(fraction));
    b
}

/// A small tile: caption over a number.
pub fn small_tile(key: &str, value: &str) -> gtk::Box {
    let b = vbox(2);
    b.add_css_class("tile");
    b.add_css_class("small");
    b.set_hexpand(true);
    b.append(&label(&key.to_uppercase(), &["tile-key"]));
    b.append(&label(value, &["tile-val"]));
    b
}

/// A coloured vertical strip (win/loss/abandoned classes) for the edge of a card.
pub fn stripe(class: &str) -> gtk::Box {
    gtk::Box::builder()
        .css_classes(["stripe".to_string(), class.to_string()])
        .width_request(4)
        .build()
}
