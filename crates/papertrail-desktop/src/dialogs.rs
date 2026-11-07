// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{actions::SHORTCUTS, desktop::Desktop, sidebar::label, APP_ID};
use adw::prelude::*;
use gtk::gio;
use papertrail_core::{
    colors,
    note::{fold, normalize_filename},
};
use std::{path::PathBuf, rc::Rc};

impl Desktop {
    pub fn show_folder_recovery(
        self: &Rc<Self>,
        recovery: &papertrail_storage::startup::FolderRecovery,
    ) {
        let body = format!(
            "Your saved notes folder is unavailable:\n{}\n\n{}\n\nPaper Trail is using:\n{}\n\nYour saved folder and note settings have been kept. Choose the original folder again to restore access; no notes have been moved or deleted.",
            recovery.unavailable.display(), recovery.reason, recovery.fallback.display()
        );
        let dialog = adw::AlertDialog::new(Some("Notes Folder Unavailable"), Some(&body));
        dialog.add_response("continue", "Continue");
        dialog.add_response("choose", "Choose Folder…");
        dialog.set_default_response(Some("choose"));
        dialog.set_close_response("continue");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "choose" {
                if let Some(desktop) = weak.upgrade() {
                    desktop.choose_folder();
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    pub fn about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Paper Trail")
            .application_icon(APP_ID)
            .developer_name("sm")
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/maennlse/paper-trail")
            .issue_url("https://github.com/maennlse/paper-trail/issues")
            .license_type(gtk::License::Gpl30)
            .comments(
                "Local plain-text notes with a focused sidebar shell and a GtkSourceView editor.",
            )
            .build();
        dialog.add_css_class("papertrail-dialog");
        dialog.present(Some(&self.window));
    }
    pub fn choose_folder(self: &Rc<Self>) {
        let chooser = gtk::FileDialog::builder()
            .title("Choose Notes Folder")
            .initial_folder(&gio::File::for_path(
                &self.model.borrow().settings.notes_dir,
            ))
            .build();
        let weak = Rc::downgrade(self);
        chooser.select_folder(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let Some(d) = weak.upgrade() {
                    match result {
                        Ok(folder) => {
                            if let Some(path) = folder.path() {
                                if let Err(e) = d.choose_active(Some(&path)) {
                                    d.error(&e);
                                }
                            }
                        }
                        Err(e)
                            if e.matches(gtk::DialogError::Dismissed)
                                || e.matches(gtk::DialogError::Cancelled) => {}
                        Err(e) => d.error(&e),
                    }
                }
            },
        );
    }
    pub fn confirm_delete(self: &Rc<Self>, path: PathBuf) -> adw::MessageDialog {
        let dialog = adw::MessageDialog::new(
            Some(&self.window),
            Some("Delete this note?"),
            Some("The note will be removed from the local notes folder."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                if let Some(d) = weak.upgrade() {
                    if let Err(e) = d.remove_note(&path) {
                        d.error(&e);
                    }
                }
            }
        });
        dialog.present();
        dialog
    }
    pub fn rename_popover(
        self: &Rc<Self>,
        path: PathBuf,
        anchor: &impl IsA<gtk::Widget>,
    ) -> gtk::Popover {
        let popover = gtk::Popover::new();
        popover.add_css_class("papertrail-popover");
        popover.add_css_class("rename-popover");
        popover.set_parent(anchor);
        popover.set_position(gtk::PositionType::Bottom);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&label("Rename note", &["heading"]));
        let entry = gtk::Entry::new();
        entry.add_css_class("rename-popover-entry");
        entry.set_text(&path.file_name().unwrap_or_default().to_string_lossy());
        entry.select_region(0, -1);
        content.append(&entry);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("flat");
        let weak_popover = popover.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(p) = weak_popover.upgrade() {
                p.popdown();
            }
        });
        actions.append(&cancel);
        let rename = gtk::Button::with_label("Rename");
        rename.add_css_class("suggested-action");
        actions.append(&rename);
        content.append(&actions);
        let weak = Rc::downgrade(self);
        let weak_entry = entry.downgrade();
        let weak_popover = popover.downgrade();
        let submit = Rc::new(move || {
            let (Some(d), Some(entry), Some(popover)) =
                (weak.upgrade(), weak_entry.upgrade(), weak_popover.upgrade())
            else {
                return;
            };
            let name = entry.text();
            popover.popdown();
            if !name.trim().is_empty() {
                if let Err(e) = d.rename_note(&path, &name) {
                    d.error(&e);
                }
            }
        });
        let on_entry = submit.clone();
        entry.connect_activate(move |_| on_entry());
        rename.connect_clicked(move |_| submit());
        popover.set_child(Some(&content));
        popover.connect_closed(|p| {
            if p.parent().is_some() {
                p.unparent();
            }
        });
        popover.popup();
        entry.grab_focus();
        popover
    }
    pub fn shortcuts(&self) -> adw::Dialog {
        let dialog = adw::Dialog::builder()
            .title("Keyboard Shortcuts")
            .content_width(560)
            .content_height(640)
            .build();
        dialog.add_css_class("papertrail-dialog");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search shortcuts"));
        content.append(&search);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        for (_, title, accels) in SHORTCUTS {
            let display = accels
                .iter()
                .map(|s| {
                    gtk::accelerator_parse(*s)
                        .map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string())
                        .unwrap_or_else(|| (*s).into())
                })
                .collect::<Vec<_>>()
                .join(" / ");
            let row = adw::ActionRow::builder().title(*title).build();
            row.add_suffix(&label(&display, &["dim-label", "caption"]));
            row.set_widget_name(&fold(&format!("{title} {display}")));
            list.append(&row);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&list)
            .build();
        content.append(&scroll);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_dialog = dialog.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.close();
                }
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        dialog.add_controller(keys);
        let weak = list.downgrade();
        search.connect_search_changed(move |s| {
            if let Some(list) = weak.upgrade() {
                let query = fold(&s.text());
                let mut child = list.first_child();
                while let Some(row) = child {
                    row.set_visible(row.widget_name().contains(&query));
                    child = row.next_sibling();
                }
            }
        });
        dialog.present(Some(&self.window));
        dialog
    }
    pub fn preferences(self: &Rc<Self>) {
        if let Some(dialog) = self
            .builder
            .object::<adw::PreferencesDialog>("preferences_dialog")
        {
            self.sync_preferences();
            dialog.present(Some(&self.window));
            return;
        }
        let dialog = adw::PreferencesDialog::builder()
            .title("Preferences")
            .content_width(620)
            .content_height((self.window.height() as f64 * 0.8) as i32)
            .build();
        dialog.add_css_class("papertrail-dialog");
        dialog.add_css_class("papertrail-preferences");
        let page = adw::PreferencesPage::new();
        let editor = adw::PreferencesGroup::builder().title("Editor").build();
        let settings = self.model.borrow().settings.clone();
        for (title, field, value) in [
            ("Show Line Numbers", 0, settings.show_line_numbers),
            ("Wrap Text", 1, settings.wrap_text),
            ("Highlight Current Line", 2, settings.highlight_current_line),
        ] {
            let row = adw::SwitchRow::builder().title(title).active(value).build();
            self.builder
                .expose_object(&format!("preference_switch_{field}"), &row);
            let weak = Rc::downgrade(self);
            row.connect_active_notify(move |row| {
                if let Some(d) = weak.upgrade() {
                    if d.syncing_preferences.get() {
                        return;
                    }
                    let result = d.update_settings(|s| {
                        *match field {
                            0 => &mut s.show_line_numbers,
                            1 => &mut s.wrap_text,
                            _ => &mut s.highlight_current_line,
                        } = row.is_active()
                    });
                    if let Err(e) = result {
                        d.error(&e);
                    }
                    if field < 2 {
                        if let Some(action) = d
                            .window
                            .lookup_action(if field == 0 {
                                "toggle-line-numbers"
                            } else {
                                "toggle-wrap"
                            })
                            .and_downcast::<gio::SimpleAction>()
                        {
                            action.set_state(&row.is_active().to_variant());
                        }
                    }
                }
            });
            editor.add(&row);
        }
        let fonts = adw::PreferencesGroup::new();
        let row = adw::ActionRow::builder()
            .title("Editor Font")
            .subtitle(if settings.use_custom_font {
                &settings.custom_font
            } else {
                "System Default"
            })
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        self.builder.expose_object("preference_font", &row);
        let weak = Rc::downgrade(self);
        row.connect_activated(move |row| {
            if let Some(d) = weak.upgrade() {
                let dialog = gtk::FontDialog::builder().modal(true).build();
                let initial = gtk::pango::FontDescription::from_string(
                    &d.model.borrow().settings.custom_font,
                );
                let weak = Rc::downgrade(&d);
                let row = row.downgrade();
                dialog.choose_font(
                    Some(&d.window),
                    Some(&initial),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let (Ok(font), Some(d)) = (result, weak.upgrade()) {
                            let font = font.to_string();
                            if let Err(e) = d.update_settings(|s| {
                                s.custom_font = font.to_string();
                                s.use_custom_font = true;
                            }) {
                                d.error(&e);
                            }
                            if let Some(row) = row.upgrade() {
                                row.set_subtitle(&font);
                            }
                        }
                    },
                );
            }
        });
        fonts.add(&row);
        let reset = adw::ActionRow::builder()
            .title("Use System Font")
            .activatable(true)
            .build();
        let weak = Rc::downgrade(self);
        let weak_row = row.downgrade();
        reset.connect_activated(move |_| {
            if let Some(d) = weak.upgrade() {
                if let Err(e) = d.update_settings(|s| s.use_custom_font = false) {
                    d.error(&e);
                }
                if let Some(row) = weak_row.upgrade() {
                    row.set_subtitle("System Default");
                }
            }
        });
        fonts.add(&reset);
        let folders = adw::PreferencesGroup::builder().title("Folders").build();
        let add = adw::ActionRow::builder()
            .title("Add Folder…")
            .activatable(true)
            .build();
        add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        let weak = Rc::downgrade(self);
        add.connect_activated(move |_| {
            if let Some(d) = weak.upgrade() {
                d.choose_folder();
            }
        });
        folders.add(&add);
        let folder_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        folder_list.add_css_class("preferences-folder-list");
        self.builder
            .expose_object("preference_folders", &folder_list);
        folders.add(&folder_list);
        page.add(&fonts);
        page.add(&editor);
        page.add(&folders);
        dialog.add(&page);
        self.builder.expose_object("preferences_dialog", &dialog);
        self.refresh_preferences_folders();
        dialog.present(Some(&self.window));
    }

    pub fn sync_preferences(&self) {
        let settings = self.model.borrow().settings.clone();
        self.syncing_preferences.set(true);
        for (field, value) in [
            settings.show_line_numbers,
            settings.wrap_text,
            settings.highlight_current_line,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(row) = self
                .builder
                .object::<adw::SwitchRow>(&format!("preference_switch_{field}"))
            {
                row.set_active(value);
            }
        }
        if let Some(row) = self.builder.object::<adw::ActionRow>("preference_font") {
            row.set_subtitle(if settings.use_custom_font {
                &settings.custom_font
            } else {
                "System Default"
            });
        }
        if let Some(dialog) = self
            .builder
            .object::<adw::PreferencesDialog>("preferences_dialog")
        {
            dialog.set_content_height((self.window.height().max(400) as f64 * 0.8) as i32);
        }
        self.syncing_preferences.set(false);
    }

    pub fn refresh_preferences_folders(self: &Rc<Self>) {
        let Some(folder_list) = self.builder.object::<gtk::Box>("preference_folders") else {
            return;
        };
        crate::sidebar::clear(&folder_list);
        let rows = gtk::ListBox::new();
        rows.set_selection_mode(gtk::SelectionMode::None);
        rows.add_css_class("boxed-list");
        folder_list.append(&rows);
        let settings = self.model.borrow().settings.clone();
        for path in &settings.note_folders {
            let row = adw::ActionRow::builder()
                .title(path.file_name().unwrap_or_default().to_string_lossy())
                .subtitle(path.display().to_string())
                .activatable(true)
                .build();
            let icon = gtk::Image::from_icon_name("folder-symbolic");
            icon.add_css_class(&colors::css_class(
                settings
                    .folder_colors
                    .get(path)
                    .map(String::as_str)
                    .unwrap_or("preset-0"),
            ));
            row.add_prefix(&icon);
            if *path == settings.notes_dir {
                row.add_suffix(&label("Current", &["caption", "dim-label"]));
            }
            if settings.note_folders.len() > 1 {
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.add_css_class("flat");
                remove.set_tooltip_text(Some("Remove Folder"));
                let path = path.clone();
                let weak = Rc::downgrade(self);
                remove.connect_clicked(move |_| {
                    if let Some(d) = weak.upgrade() {
                        if let Err(e) = d.close_folder(&path) {
                            d.error(&e);
                        }
                    }
                });
                row.add_suffix(&remove);
            }
            let path = path.clone();
            let weak = Rc::downgrade(self);
            row.connect_activated(move |_| {
                if let Some(d) = weak.upgrade() {
                    if let Err(e) = d.choose_active(Some(&path)) {
                        d.error(&e);
                    }
                }
            });
            rows.append(&row);
        }
    }
    pub fn edit_folder(self: &Rc<Self>, path: PathBuf) {
        let window = adw::Window::builder()
            .title("Edit Folder")
            .transient_for(&self.window)
            .modal(true)
            .default_width(420)
            .default_height(320)
            .build();
        window.add_css_class("papertrail-dialog");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let name = adw::EntryRow::builder()
            .title("Name")
            .text(path.file_name().unwrap_or_default().to_string_lossy())
            .build();
        content.append(&name);
        content.append(&label("Color", &["heading"]));
        let selected = self
            .model
            .borrow()
            .settings
            .folder_colors
            .get(&path)
            .cloned()
            .unwrap_or_else(|| "preset-0".into());
        let chosen = Rc::new(std::cell::RefCell::new(selected.clone()));
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(6)
            .max_children_per_line(6)
            .row_spacing(8)
            .column_spacing(8)
            .build();
        let mut group: Option<gtk::ToggleButton> = None;
        for (index, color) in colors::PRESETS.iter().enumerate() {
            let button = gtk::ToggleButton::new();
            button.add_css_class("flat");
            button.add_css_class("folder-color-picker");
            button.add_css_class(&colors::css_class(color));
            button.set_size_request(28, 28);
            if let Some(group) = &group {
                button.set_group(Some(group));
            } else {
                group = Some(button.clone());
            }
            let token = format!("preset-{index}");
            button.set_active(selected == token);
            let chosen = chosen.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    *chosen.borrow_mut() = token.clone();
                }
            });
            grid.insert(&button, -1);
        }
        content.append(&grid);
        let custom = gtk::Button::with_label("Custom Color…");
        let chosen_color = chosen.clone();
        let weak_window = window.downgrade();
        let weak_custom = custom.downgrade();
        let weak_group = group.as_ref().map(|g| g.downgrade());
        custom.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                let dialog = gtk::ColorDialog::builder()
                    .title("Select Folder Color")
                    .modal(true)
                    .with_alpha(false)
                    .build();
                let initial = colors::hex(&chosen_color.borrow())
                    .parse::<gtk::gdk::RGBA>()
                    .ok();
                let chosen = chosen_color.clone();
                let weak_custom = weak_custom.clone();
                let weak_group = weak_group.clone();
                dialog.choose_rgba(
                    Some(&window),
                    initial.as_ref(),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let Ok(rgba) = result {
                            let hex = format!(
                                "#{:02x}{:02x}{:02x}",
                                (rgba.red() * 255.0).round() as u8,
                                (rgba.green() * 255.0).round() as u8,
                                (rgba.blue() * 255.0).round() as u8
                            );
                            *chosen.borrow_mut() = hex.clone();
                            if let Some(b) = weak_custom.upgrade() {
                                b.set_label(&hex);
                            }
                            if let Some(group) = weak_group.and_then(|g| g.upgrade()) {
                                group.set_active(false);
                            }
                        }
                    },
                );
            }
        });
        content.append(&custom);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label("Cancel");
        let weak_window = window.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(w) = weak_window.upgrade() {
                w.close();
            }
        });
        actions.append(&cancel);
        let save = gtk::Button::with_label("Save");
        save.add_css_class("suggested-action");
        let weak_window = window.downgrade();
        let weak = Rc::downgrade(self);
        save.connect_clicked(move |_| {
            let (Some(d), Some(window)) = (weak.upgrade(), weak_window.upgrade()) else {
                return;
            };
            let result = (|| -> std::io::Result<()> {
                d.flush()?;
                let target = path
                    .parent()
                    .ok_or_else(|| std::io::Error::other("Cannot rename root folder"))?
                    .join(normalize_filename(&name.text(), "Untitled Folder"));
                if target != path {
                    if target.exists() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "A folder with that name already exists",
                        ));
                    }
                    std::fs::rename(&path, &target)?;
                    let mut m = d.model.borrow_mut();
                    m.settings.rename_folder(&path, &target);
                    if let Some(note) = &mut m.session.current {
                        if let Ok(suffix) = note.path.strip_prefix(&path) {
                            note.path = target.join(suffix);
                        }
                    }
                }
                d.model
                    .borrow_mut()
                    .settings
                    .folder_colors
                    .insert(target, chosen.borrow().clone());
                d.persist()?;
                d.refresh()?;
                d.show_session();
                Ok(())
            })();
            if let Err(e) = result {
                d.error(&e);
            } else {
                window.close();
            }
        });
        actions.append(&save);
        content.append(&actions);
        window.set_content(Some(&content));
        window.present();
    }
}
