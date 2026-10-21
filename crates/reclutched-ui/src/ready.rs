//! The page shown after setup (and on every later launch) until the real app exists.

use adw::prelude::*;
use reclutched_core::rocketleague::settings::Settings;

use crate::flow::mode_label;

/// `on_rerun` is called when the user wants to go through the setup again.
pub fn build(settings: &Settings, on_rerun: impl Fn() + 'static) -> gtk::Widget {
    let title = gtk::Label::builder()
        .label("Reclutched is ready")
        .css_classes(["setup-title"])
        .halign(gtk::Align::Start)
        .build();
    let sub = gtk::Label::builder()
        .label("The app itself is coming next. Your choices are saved.")
        .css_classes(["setup-sub"])
        .halign(gtk::Align::Start)
        .build();

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .build();
    column.append(&title);
    column.append(&sub);

    let facts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(26)
        .build();
    facts.append(&fact(
        "USERNAME",
        settings.username.as_deref().unwrap_or(""),
    ));
    facts.append(&fact("MODE", mode_label(settings.display_mode)));
    column.append(&facts);

    let rerun = gtk::Button::builder()
        .label("Run setup again")
        .css_classes(["flat-nav"])
        .halign(gtk::Align::Start)
        .margin_top(30)
        .build();
    rerun.connect_clicked(move |_| on_rerun());
    column.append(&rerun);

    column.set_width_request(500);
    let clamp = adw::Clamp::builder()
        .maximum_size(500)
        .child(&column)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .margin_top(96)
        .margin_start(24)
        .margin_end(24)
        .build();
    clamp.upcast()
}

pub fn fact(key: &str, value: &str) -> gtk::Widget {
    let k = gtk::Label::builder()
        .label(key)
        .css_classes(["summary-key"])
        .halign(gtk::Align::Start)
        .build();
    let v = gtk::Label::builder()
        .label(value)
        .css_classes(["summary-val"])
        .halign(gtk::Align::Start)
        .build();
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    b.append(&k);
    b.append(&v);
    b.upcast()
}
