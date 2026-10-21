//! The loading screen: a placeholder logo (the real one comes later) and a spinner.

use gtk::prelude::*;

pub fn build() -> gtk::Widget {
    let logo_letter = gtk::Label::new(Some("R"));
    let logo = gtk::Box::builder()
        .css_classes(["logo-placeholder"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    logo.append(&logo_letter);
    logo_letter.set_hexpand(true);
    logo_letter.set_vexpand(true);

    let wordmark = gtk::Label::builder()
        .label("RECLUTCHED")
        .css_classes(["wordmark"])
        .build();
    let tagline = gtk::Label::builder()
        .label("Game stats for Linux")
        .css_classes(["tagline"])
        .build();
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(26)
        .height_request(26)
        .margin_top(28)
        .build();

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    column.append(&logo);
    column.append(&wordmark);
    column.append(&tagline);
    column.append(&spinner);
    column.upcast()
}
