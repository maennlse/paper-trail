// SPDX-License-Identifier: GPL-3.0-or-later
mod actions;
mod desktop;
mod dialogs;
mod editor;
mod sidebar;
#[cfg(test)]
mod tests;

use adw::prelude::*;
use desktop::Desktop;
use gtk::{gio, glib};
use std::{cell::RefCell, rc::Rc};

pub const APP_ID: &str = "io.github.maennlse.paper-trail";
pub const RESOURCE: &str = "/io/github/maennlse/paper-trail";

fn main() -> glib::ExitCode {
    gio::resources_register_include!("papertrail.gresource")
        .expect("Embedded application resources");
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(
            if cfg!(debug_assertions) && std::env::var_os("FLATPAK_ID").is_none() {
                gio::ApplicationFlags::NON_UNIQUE
            } else {
                gio::ApplicationFlags::FLAGS_NONE
            },
        )
        .build();
    let state: Rc<RefCell<Option<Rc<Desktop>>>> = Rc::default();
    let current = state.clone();
    app.connect_activate(move |app| {
        if let Some(desktop) = current.borrow().as_ref().filter(|d| d.window.is_visible()) {
            desktop.window.present();
            return;
        }
        match Desktop::new(app) {
            Ok(desktop) => {
                desktop.window.present();
                *current.borrow_mut() = Some(desktop);
            }
            Err(e) => {
                eprintln!("Could not start Paper Trail: {e}");
                let parent = adw::ApplicationWindow::builder()
                    .application(app)
                    .title("Paper Trail")
                    .build();
                let dialog = adw::AlertDialog::new(
                    Some("Could Not Start Paper Trail"),
                    Some(&e.to_string()),
                );
                dialog.add_response("close", "Close");
                let weak_parent = parent.downgrade();
                dialog.connect_response(None, move |_, _| {
                    if let Some(parent) = weak_parent.upgrade() {
                        parent.close();
                    }
                });
                parent.present();
                dialog.present(Some(&parent));
            }
        }
    });
    app.connect_shutdown(move |_| {
        if let Some(d) = state.borrow().as_ref() {
            if let Err(e) = d.flush() {
                eprintln!("Could not save note during shutdown: {e}");
            }
        }
    });
    app.run()
}
