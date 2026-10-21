//! The loading screen: the logo and a spinner.

use gtk::prelude::*;

const LOGO: &[u8] = include_bytes!("../assets/logo.png");

pub fn build() -> gtk::Widget {
    let logo = gtk::Image::builder()
        .pixel_size(128)
        .halign(gtk::Align::Center)
        .build();
    if let Ok(texture) = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_static(LOGO)) {
        logo.set_paintable(Some(&texture));
    }

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
