//! Pages that slide to the left by themselves every few seconds, with little dots below.

use adw::prelude::*;

/// `pages` slide one after the other every `seconds`; with one page nothing moves.
pub fn carousel(pages: &[gtk::Widget], seconds: u32) -> gtk::Box {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeft)
        .transition_duration(500)
        .hhomogeneous(true)
        .vhomogeneous(true)
        .hexpand(true)
        .build();
    let dots = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .css_classes(["carousel-dots"])
        .build();
    let mut dot_widgets = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        stack.add_named(page, Some(&i.to_string()));
        let dot = gtk::Box::builder().css_classes(["carousel-dot"]).build();
        dots.append(&dot);
        dot_widgets.push(dot);
    }
    let show = {
        let (stack, dots) = (stack.downgrade(), dot_widgets.clone());
        move |index: usize| {
            let Some(stack) = stack.upgrade() else { return };
            stack.set_transition_type(gtk::StackTransitionType::SlideLeft);
            stack.set_visible_child_name(&index.to_string());
            for (i, dot) in dots.iter().enumerate() {
                if i == index {
                    dot.add_css_class("current");
                } else {
                    dot.remove_css_class("current");
                }
            }
        }
    };
    show(0);
    if pages.len() > 1 {
        let (count, current, stack) = (
            pages.len(),
            std::rc::Rc::new(std::cell::Cell::new(0usize)),
            stack.downgrade(),
        );
        gtk::glib::timeout_add_seconds_local(seconds, move || {
            // The stack is gone when the page was rebuilt: stop the timer.
            if stack.upgrade().is_none() {
                return gtk::glib::ControlFlow::Break;
            }
            let next = (current.get() + 1) % count;
            current.set(next);
            show(next);
            gtk::glib::ControlFlow::Continue
        });
    } else {
        dots.set_visible(false);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.append(&stack);
    column.append(&dots);
    column
}
