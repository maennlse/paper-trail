// SPDX-License-Identifier: GPL-3.0-or-later
use crate::desktop::Desktop;
use adw::prelude::*;
use gtk::{gio, glib};
use sourceview5::prelude::*;
use std::{io, rc::Rc};

pub const SHORTCUTS: &[(&str, &str, &[&str])] = &[
    ("new-note", "New Note", &["<Control>n"]),
    ("close-note", "Close Note", &["<Control>w"]),
    ("choose-folder", "Add Folder", &["<Control>o"]),
    ("print-note", "Print Note", &["<Control>p"]),
    ("show-preferences", "Preferences", &["<Control>comma"]),
    (
        "show-shortcuts",
        "Keyboard Shortcuts",
        &["<Control>question"],
    ),
    ("delete-note", "Delete Note", &["<Control>Delete"]),
    ("toggle-fullscreen", "Toggle Fullscreen", &["F11"]),
    ("toggle-sidebar", "Toggle Sidebar", &["F9", "<Control>b"]),
    ("toggle-info", "Document Properties", &["F10", "<Control>i"]),
    ("focus-sidebar-search", "Search Notes", &["<Control>k"]),
    ("toggle-editor-search", "Find", &["<Control>f"]),
    ("focus-replace", "Replace", &["<Control>h"]),
    ("find-next", "Next Match", &["F3", "<Control>g"]),
    (
        "find-previous",
        "Previous Match",
        &["<Shift>F3", "<Control><Shift>g"],
    ),
    ("zoom-in", "Zoom In", &["<Control>plus", "<Control>equal"]),
    ("zoom-out", "Zoom Out", &["<Control>minus"]),
    ("zoom-reset", "Reset Zoom", &["<Control>0"]),
    ("replace-current", "Replace Current", &["<Control>Return"]),
    ("replace-all", "Replace All", &["<Control><Shift>Return"]),
    ("toggle-line-numbers", "Toggle Line Numbers", &["F7"]),
    ("toggle-wrap", "Toggle Wrap", &["<Alt>z"]),
    ("toggle-monospace", "Toggle Monospace", &["<Control>m"]),
    ("close-search", "Close Search", &["Escape"]),
    ("about", "About", &["F1"]),
    ("quit", "Quit", &["<Control>q"]),
];

impl Desktop {
    pub fn action(
        self: &Rc<Self>,
        name: &str,
        callback: impl Fn(&Rc<Self>) -> io::Result<()> + 'static,
    ) {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(d) = weak.upgrade() {
                if let Err(e) = callback(&d) {
                    d.error(&e);
                }
            }
        });
        self.window.add_action(&action);
    }

    pub fn setup_actions(self: &Rc<Self>, app: &adw::Application) {
        for (name, _, accels) in SHORTCUTS {
            app.set_accels_for_action(&format!("win.{name}"), accels);
        }
        for (name, accelerator) in [("quit", "<Control>q"), ("about", "F1")] {
            app.remove_action(name);
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(d) = weak.upgrade() {
                    if name == "quit" {
                        d.window.close();
                    } else {
                        d.about();
                    }
                }
            });
            app.add_action(&action);
            app.set_accels_for_action(&format!("win.{name}"), &[]);
            app.set_accels_for_action(&format!("app.{name}"), &[accelerator]);
        }
        self.action("new-note", |d| d.new_note());
        self.action("close-note", |d| d.close_note());
        self.action("choose-folder", |d| {
            d.choose_folder();
            Ok(())
        });
        self.action("delete-note", |d| {
            if let Some(p) = d.current_path() {
                d.confirm_delete(p);
            }
            Ok(())
        });
        self.action("quit", |d| {
            d.window.close();
            Ok(())
        });
        self.action("about", |d| {
            d.about();
            Ok(())
        });
        self.action("show-preferences", |d| {
            d.preferences();
            Ok(())
        });
        self.action("show-shortcuts", |d| {
            d.shortcuts();
            Ok(())
        });
        self.action("print-note", |d| d.print_note());
        self.action("toggle-fullscreen", |d| {
            if d.window.is_fullscreen() {
                d.window.unfullscreen();
            } else {
                d.window.fullscreen();
            }
            Ok(())
        });
        self.action("toggle-sidebar", |d| {
            let panel: gtk::Box = d.widget("sidebar_panel");
            let visible = !panel.is_visible();
            panel.set_visible(visible);
            d.widget::<gtk::Separator>("sidebar_divider")
                .set_visible(visible);
            Ok(())
        });
        self.action("toggle-info", |d| {
            let panel: gtk::Revealer = d.widget("info_revealer");
            let reveal = !panel.reveals_child();
            panel.set_visible(true);
            panel.set_reveal_child(reveal);
            d.widget::<gtk::ToggleButton>("info_button")
                .set_active(reveal);
            Ok(())
        });
        for (name, mode) in [
            ("theme-system", "system"),
            ("theme-light", "light"),
            ("theme-dark", "dark"),
        ] {
            self.action(name, move |d| {
                d.update_settings(|s| s.theme_mode = mode.into())
            });
        }
        for (name, delta) in [("zoom-in", 0.1), ("zoom-out", -0.1), ("zoom-reset", 0.0)] {
            self.action(name, move |d| {
                d.update_settings(|s| {
                    s.font_scale = if delta == 0.0 {
                        1.0
                    } else {
                        ((s.font_scale * 100.0).round() / 100.0 + delta).clamp(0.7, 2.5)
                    }
                })
            });
        }
        for (name, field) in [
            ("toggle-line-numbers", 0),
            ("toggle-wrap", 1),
            ("toggle-monospace", 2),
        ] {
            let s = &self.model.borrow().settings;
            let initial = match field {
                0 => s.show_line_numbers,
                1 => s.wrap_text,
                _ => s.use_monospace,
            };
            let action = gio::SimpleAction::new_stateful(name, None, &initial.to_variant());
            let weak = Rc::downgrade(self);
            action.connect_change_state(move |action, value| {
                if let (Some(d), Some(value)) =
                    (weak.upgrade(), value.and_then(|v| v.get::<bool>()))
                {
                    if let Err(e) = d.update_settings(|s| {
                        *match field {
                            0 => &mut s.show_line_numbers,
                            1 => &mut s.wrap_text,
                            _ => &mut s.use_monospace,
                        } = value
                    }) {
                        d.error(&e);
                    }
                    action.set_state(&value.to_variant());
                }
            });
            self.window.add_action(&action);
        }
        self.action("focus-sidebar-search", |d| {
            let revealer: gtk::Revealer = d.widget("sidebar_search_revealer");
            let visible = !revealer.reveals_child();
            revealer.set_reveal_child(visible);
            if visible {
                d.widget::<gtk::SearchEntry>("sidebar_search_entry")
                    .grab_focus();
            }
            Ok(())
        });
        self.action("toggle-editor-search", |d| {
            d.open_search(false);
            Ok(())
        });
        self.action("focus-replace", |d| {
            d.open_search(true);
            Ok(())
        });
        self.action("toggle-replace", |d| {
            let r: gtk::Revealer = d.widget("editor_replace_revealer");
            r.set_reveal_child(!r.reveals_child());
            Ok(())
        });
        self.action("close-search", |d| {
            d.close_search();
            Ok(())
        });
        self.action("find-next", |d| {
            d.find(true);
            Ok(())
        });
        self.action("find-previous", |d| {
            d.find(false);
            Ok(())
        });
        self.action("replace-current", |d| d.replace(false));
        self.action("replace-all", |d| d.replace(true));
        let menu = gio::Menu::new();
        let section = gio::Menu::new();
        let custom = gio::MenuItem::new(None, None);
        custom.set_attribute_value("custom", Some(&"theme-selector".to_variant()));
        section.append_item(&custom);
        menu.append_section(None, &section);
        let section = gio::Menu::new();
        for (text, action) in [
            ("Print Note", "print-note"),
            ("Preferences", "show-preferences"),
            ("Keyboard Shortcuts", "show-shortcuts"),
        ] {
            section.append(Some(text), Some(&format!("win.{action}")));
        }
        menu.append_section(None, &section);
        menu.append(Some("About Paper Trail"), Some("win.about"));
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.add_css_class("papertrail-menu-popover");
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let themes = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        themes.add_css_class("theme-selector");
        themes.add_css_class("theme-selector-row");
        let mut group: Option<gtk::CheckButton> = None;
        for (mode, class, title) in [
            ("system", "follow", "Follow System Style"),
            ("light", "light", "Light Style"),
            ("dark", "dark", "Dark Style"),
        ] {
            let button = gtk::CheckButton::new();
            button.add_css_class("theme-selector-button");
            button.add_css_class(class);
            button.set_tooltip_text(Some(title));
            button.set_hexpand(true);
            if let Some(group) = &group {
                button.set_group(Some(group));
            } else {
                group = Some(button.clone());
            }
            button.set_active(self.model.borrow().settings.theme_mode == mode);
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |b| {
                if b.is_active() {
                    if let Some(d) = weak.upgrade() {
                        if let Err(e) = d.update_settings(|s| s.theme_mode = mode.into()) {
                            d.error(&e);
                        }
                    }
                }
            });
            themes.append(&button);
        }
        controls.append(&themes);
        controls.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        zoom.add_css_class("menu-zoom-row");
        for (label, action) in [("−", "zoom-out"), ("100%", "zoom-reset"), ("+", "zoom-in")] {
            let button = gtk::Button::with_label(label);
            button.add_css_class("flat");
            button.set_hexpand(true);
            button.set_action_name(Some(&format!("win.{action}")));
            zoom.append(&button);
            if action == "zoom-reset" {
                self.builder.expose_object("zoom_reset_button", &button);
                let weak = Rc::downgrade(self);
                let weak_button = button.downgrade();
                popover.connect_show(move |_| {
                    if let (Some(d), Some(b)) = (weak.upgrade(), weak_button.upgrade()) {
                        b.set_label(&format!(
                            "{}%",
                            (d.model.borrow().settings.font_scale * 100.0).round()
                        ));
                    }
                });
            }
        }
        controls.append(&zoom);
        popover.add_child(&controls, "theme-selector");
        self.widget::<gtk::MenuButton>("notes_menu_button")
            .set_popover(Some(&popover));
        let weak = Rc::downgrade(self);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(d) = weak.upgrade() {
                let manager = sourceview5::StyleSchemeManager::default();
                let scheme = manager.scheme(if adw::StyleManager::default().is_dark() {
                    "Adwaita-dark"
                } else {
                    "Adwaita"
                });
                d.buffer.set_style_scheme(scheme.as_ref());
            }
        });
        let _ = glib::Propagation::Proceed;
    }
}
