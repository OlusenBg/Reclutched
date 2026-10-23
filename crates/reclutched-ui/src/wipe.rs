//! "Wipe data" in the settings: delete today, a month, a year or everything, after a question.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::periods::Grain;
use reclutched_core::rocketleague::storage::Store;

use crate::history::period_title;
use crate::mainview::local_now;

/// A period you can wipe: its key ("2026-10") and what to call it ("October 2026, 120 matches").
struct Option_ {
    key: String,
    text: String,
}

fn options(db_path: &PathBuf, grain: Grain) -> Vec<Option_> {
    let (offset, today) = local_now();
    Store::open(db_path)
        .and_then(|s| s.period_summaries(grain, offset))
        .unwrap_or_default()
        .into_iter()
        .map(|p| Option_ {
            text: format!(
                "{}, {} match{}",
                period_title(&p.key, today),
                p.matches,
                if p.matches == 1 { "" } else { "es" }
            ),
            key: p.key,
        })
        .collect()
}

/// Asks first, then deletes and tells the result through `toast`.
fn confirm_and_wipe(
    parent: &adw::ApplicationWindow,
    db_path: PathBuf,
    what: String,
    key: Option<String>,
    toast: Rc<dyn Fn(&str)>,
    done: Rc<dyn Fn()>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete this data?")
        .body(format!(
            "This permanently deletes {what}: the matches with their players, goals and movement. It cannot be undone."
        ))
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let (offset, _) = local_now();
        let result = Store::open(&db_path).and_then(|mut store| match &key {
            Some(key) => store.delete_period(key, offset),
            None => store.delete_everything(),
        });
        match result {
            Ok(0) => toast("There was nothing to delete"),
            Ok(n) => toast(&format!(
                "Deleted {n} match{}",
                if n == 1 { "" } else { "es" }
            )),
            Err(err) => toast(&format!("Could not delete: {err}")),
        }
        done();
    });
    dialog.present(Some(parent));
}

/// The group for the "Your data" page.
pub fn group(
    parent: &adw::ApplicationWindow,
    db_path: PathBuf,
    toast: Rc<dyn Fn(&str)>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Wipe data")
        .description("Delete matches you no longer want. Deleting is permanent.")
        .build();

    let wipe_button = || {
        gtk::Button::builder()
            .label("Delete")
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action"])
            .build()
    };

    // Today.
    let today_row = adw::ActionRow::builder().title("Today").build();
    let today_button = wipe_button();
    today_row.add_suffix(&today_button);
    group.add(&today_row);

    // A month and a year, picked from the ones that have matches.
    let month_row = adw::ComboRow::builder().title("A month").build();
    let month_button = wipe_button();
    month_row.add_suffix(&month_button);
    group.add(&month_row);
    let year_row = adw::ComboRow::builder().title("A year").build();
    let year_button = wipe_button();
    year_row.add_suffix(&year_button);
    group.add(&year_row);

    // Everything.
    let all_row = adw::ActionRow::builder()
        .title("Everything")
        .subtitle("Every match and every remembered player")
        .build();
    let all_button = wipe_button();
    all_row.add_suffix(&all_button);
    group.add(&all_row);

    // The month and year lists are read again after every deletion.
    let months: Rc<std::cell::RefCell<Vec<Option_>>> = Rc::default();
    let years: Rc<std::cell::RefCell<Vec<Option_>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let (db_path, months, years) = (db_path.clone(), months.clone(), years.clone());
        let (month_row, year_row) = (month_row.clone(), year_row.clone());
        let (mb, yb) = (month_button.clone(), year_button.clone());
        Rc::new(move || {
            for (grain, store, row, button) in [
                (Grain::Month, &months, &month_row, &mb),
                (Grain::Year, &years, &year_row, &yb),
            ] {
                let found = options(&db_path, grain);
                let names: Vec<&str> = found.iter().map(|o| o.text.as_str()).collect();
                row.set_model(Some(&gtk::StringList::new(&names)));
                row.set_sensitive(!found.is_empty());
                button.set_sensitive(!found.is_empty());
                *store.borrow_mut() = found;
            }
        })
    };
    refresh();

    {
        let (parent, db, toast, refresh) = (
            parent.clone(),
            db_path.clone(),
            toast.clone(),
            refresh.clone(),
        );
        today_button.connect_clicked(move |_| {
            let (_, today) = local_now();
            confirm_and_wipe(
                &parent,
                db.clone(),
                "everything you played today".into(),
                Some(today.key()),
                toast.clone(),
                refresh.clone(),
            );
        });
    }
    for (button, row, list, noun) in [
        (&month_button, &month_row, &months, "month"),
        (&year_button, &year_row, &years, "year"),
    ] {
        let (parent, db, toast, refresh) = (
            parent.clone(),
            db_path.clone(),
            toast.clone(),
            refresh.clone(),
        );
        let (row, list) = (row.clone(), list.clone());
        button.connect_clicked(move |_| {
            let picked = {
                let list = list.borrow();
                list.get(row.selected() as usize)
                    .map(|o| (o.key.clone(), o.text.clone()))
            };
            if let Some((key, text)) = picked {
                confirm_and_wipe(
                    &parent,
                    db.clone(),
                    format!("the {noun} {text}"),
                    Some(key),
                    toast.clone(),
                    refresh.clone(),
                );
            }
        });
    }
    {
        let (parent, db, toast, refresh) = (parent.clone(), db_path, toast, refresh);
        all_button.connect_clicked(move |_| {
            confirm_and_wipe(
                &parent,
                db.clone(),
                "all your data".into(),
                None,
                toast.clone(),
                refresh.clone(),
            );
        });
    }
    group
}
