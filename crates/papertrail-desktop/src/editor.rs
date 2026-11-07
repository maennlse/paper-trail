// SPDX-License-Identifier: GPL-3.0-or-later
use crate::desktop::Desktop;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use papertrail_core::{Note, Settings};
use sourceview5::prelude::*;
use std::{io, rc::Rc};

impl Desktop {
    pub fn setup_editor(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.buffer.connect_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.edit_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("editor_search_entry")
            .connect_search_changed(move |entry| {
                if let Some(d) = weak.upgrade() {
                    let text = entry.text();
                    d.search.settings().set_search_text(Some(&text));
                    d.search.set_highlight(!text.is_empty());
                    d.find(true);
                    d.search_status();
                }
            });
        let weak = Rc::downgrade(self);
        self.search.connect_occurrences_count_notify(move |_| {
            if let Some(d) = weak.upgrade() {
                d.search_status();
            }
        });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("editor_search_entry")
            .connect_activate(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.find(true);
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::Entry>("editor_replace_entry")
            .connect_activate(move |_| {
                if let Some(d) = weak.upgrade() {
                    if let Err(e) = d.replace(false) {
                        d.error(&e);
                    }
                }
            });
        for id in ["editor_search_entry", "editor_replace_entry"] {
            let keys = gtk::EventControllerKey::new();
            let weak = Rc::downgrade(self);
            keys.connect_key_pressed(move |_, key, _, state| {
                if key == gdk::Key::Return && state.contains(gdk::ModifierType::SHIFT_MASK) {
                    if let Some(d) = weak.upgrade() {
                        if id == "editor_search_entry" {
                            d.find(false);
                        } else if let Err(e) = d.replace(true) {
                            d.error(&e);
                        }
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            self.widget::<gtk::Widget>(id).add_controller(keys);
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(d) = weak.upgrade() {
                    // Dialogs own Escape while open, even when a search bar in
                    // the underlying window is still revealed.
                    if d.window.visible_dialog().is_some() {
                        return glib::Propagation::Proceed;
                    }
                    if d.widget::<gtk::Revealer>("sidebar_search_revealer")
                        .reveals_child()
                        || d.widget::<gtk::Revealer>("editor_search_revealer")
                            .reveals_child()
                    {
                        d.close_search();
                        return glib::Propagation::Stop;
                    }
                }
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let menu = gio::Menu::new();
        let cases = gio::Menu::new();
        for (name, title, case) in [
            ("lower", "Lowercase", sourceview5::ChangeCaseType::Lower),
            ("upper", "Uppercase", sourceview5::ChangeCaseType::Upper),
            ("toggle", "Toggle Case", sourceview5::ChangeCaseType::Toggle),
            ("title", "Title Case", sourceview5::ChangeCaseType::Title),
        ] {
            let name = format!("change-case-{name}");
            cases.append(Some(title), Some(&format!("win.{name}")));
            self.action(&name, move |d| {
                if let Some((mut start, mut end)) = d.buffer.selection_bounds() {
                    d.buffer.change_case(case, &mut start, &mut end);
                }
                Ok(())
            });
        }
        menu.append_submenu(Some("Change Case"), &cases);
        self.view.set_extra_menu(Some(&menu));
        let weak = Rc::downgrade(self);
        self.widget::<gtk::Button>("info_delete_button")
            .connect_clicked(move |_| {
                if let Some(d) = weak.upgrade() {
                    if let Some(path) = d.current_path() {
                        d.confirm_delete(path);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("language_search_entry")
            .connect_search_changed(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.rebuild_languages();
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("language_listbox")
            .connect_row_activated(move |_, row| {
                let Some(d) = weak.upgrade() else {
                    return;
                };
                let id = row.widget_name();
                if let Some(path) = d.current_path() {
                    let result = d.update_settings(|s| {
                        if id == "automatic" {
                            s.note_language_overrides.remove(&path);
                        } else {
                            s.note_language_overrides.insert(path, id.to_string());
                        }
                    });
                    if let Err(e) = result {
                        d.error(&e);
                    }
                    let m = d.model.borrow();
                    if let Some(note) = &m.session.current {
                        d.apply_language(&m.settings, note);
                        d.widget::<adw::ActionRow>("info_type_row")
                            .set_subtitle(&d.document_type());
                    }
                    drop(m);
                    if let Err(e) = d.refresh() {
                        d.error(&e);
                    }
                    d.rebuild_languages();
                    d.widget::<gtk::Popover>("language_popover").popdown();
                }
            });
        self.rebuild_languages();
    }
    pub fn language_label(&self, note: &Note) -> String {
        self.language_for(&self.model.borrow().settings, note)
            .map(|l| l.name().to_string())
            .unwrap_or_else(|| "Text".into())
    }
    pub fn document_type(&self) -> String {
        self.buffer
            .language()
            .map(|l| l.name().to_string())
            .unwrap_or_else(|| "Plain Text".into())
    }
    fn language_for(&self, settings: &Settings, note: &Note) -> Option<sourceview5::Language> {
        let manager = sourceview5::LanguageManager::default();
        settings
            .note_language_overrides
            .get(&note.path)
            .and_then(|id| manager.language(id))
            .or_else(|| manager.guess_language(Some(&note.path), None))
            .or_else(|| {
                // Native globs are case-sensitive. Retry an unrecognized
                // extension in lowercase, without changing the real filename.
                // Keep native matches first (e.g. .C can mean C++, not C).
                let extension = note.path.extension()?;
                let lowercase = extension.to_ascii_lowercase();
                if extension == lowercase {
                    return None;
                }
                manager.guess_language(Some(&note.path.with_extension(lowercase)), None)
            })
    }
    pub fn apply_language(&self, settings: &Settings, note: &Note) {
        let language = self.language_for(settings, note);
        self.buffer.set_language(language.as_ref());
        self.buffer.set_highlight_syntax(language.is_some());
    }
    pub fn rebuild_languages(&self) {
        let list: gtk::ListBox = self.widget("language_listbox");
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let query = papertrail_core::note::fold(
            &self
                .widget::<gtk::SearchEntry>("language_search_entry")
                .text(),
        );
        let manager = sourceview5::LanguageManager::default();
        let selected = self
            .current_path()
            .and_then(|p| {
                self.model
                    .borrow()
                    .settings
                    .note_language_overrides
                    .get(&p)
                    .cloned()
            })
            .unwrap_or_else(|| "automatic".into());
        let mut languages: Vec<_> = manager
            .language_ids()
            .iter()
            .filter_map(|id| manager.language(id))
            .filter(|l| !l.is_hidden())
            .map(|l| (l.name().to_string(), l.id().to_string()))
            .collect();
        languages.sort();
        languages.insert(0, ("Automatic".into(), "automatic".into()));
        for (name, id) in languages {
            if !papertrail_core::note::fold(&format!("{name} {id}")).contains(&query) {
                continue;
            }
            let row = adw::ActionRow::builder()
                .title(&name)
                .subtitle(if id == "automatic" {
                    "Use filename-based detection"
                } else {
                    &id
                })
                .activatable(true)
                .build();
            row.set_widget_name(&id);
            if id == selected {
                row.add_css_class("selected");
            }
            list.append(&row);
        }
    }
    pub fn open_search(&self, replace: bool) {
        self.widget::<gtk::Revealer>("editor_search_revealer")
            .set_reveal_child(true);
        self.widget::<gtk::Revealer>("editor_replace_revealer")
            .set_reveal_child(replace);
        self.widget::<gtk::ToggleButton>("editor_replace_toggle_button")
            .set_active(replace);
        if let Some((start, end)) = self.buffer.selection_bounds() {
            let text = self.buffer.text(&start, &end, true);
            if !text.contains('\n') {
                self.widget::<gtk::SearchEntry>("editor_search_entry")
                    .set_text(&text);
            }
        }
        self.widget::<gtk::Widget>(if replace {
            "editor_replace_entry"
        } else {
            "editor_search_entry"
        })
        .grab_focus();
    }
    pub fn close_search(&self) {
        let sidebar: gtk::Revealer = self.widget("sidebar_search_revealer");
        if sidebar.reveals_child() {
            sidebar.set_reveal_child(false);
            self.widget::<gtk::SearchEntry>("sidebar_search_entry")
                .set_text("");
        } else {
            self.close_editor_search();
        }
        self.view.grab_focus();
    }
    pub fn find(&self, forward: bool) {
        if self
            .search
            .settings()
            .search_text()
            .is_none_or(|s| s.is_empty())
        {
            return;
        }
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let i = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (i, i)
        });
        let from = if forward { end } else { start };
        let found = if forward {
            self.search.forward(&from)
        } else {
            self.search.backward(&from)
        }
        .or_else(|| {
            if forward {
                self.search.forward(&self.buffer.start_iter())
            } else {
                self.search.backward(&self.buffer.end_iter())
            }
        });
        if let Some((mut start, end, _)) = found {
            self.buffer.select_range(&start, &end);
            self.view.scroll_to_iter(&mut start, 0.25, false, 0.0, 0.0);
        }
        self.search_status();
    }
    pub fn close_editor_search(&self) {
        self.widget::<gtk::Revealer>("editor_search_revealer")
            .set_reveal_child(false);
        self.widget::<gtk::Revealer>("editor_replace_revealer")
            .set_reveal_child(false);
        self.widget::<gtk::SearchEntry>("editor_search_entry")
            .set_text("");
        self.widget::<gtk::Entry>("editor_replace_entry")
            .set_text("");
        self.widget::<gtk::ToggleButton>("editor_replace_toggle_button")
            .set_active(false);
        self.search.settings().set_search_text(None);
        self.search.set_highlight(false);
        self.widget::<gtk::Label>("editor_search_status_label")
            .set_label("");
    }
    pub fn search_status(&self) {
        let count = self.search.occurrences_count();
        let text = if self
            .search
            .settings()
            .search_text()
            .is_none_or(|s| s.is_empty())
        {
            String::new()
        } else if count < 0 {
            "Searching…".into()
        } else if count == 0 {
            "No matches".into()
        } else {
            let position = self
                .buffer
                .selection_bounds()
                .map(|(s, e)| self.search.occurrence_position(&s, &e))
                .unwrap_or(0);
            if position > 0 {
                format!("{position} of {count} matches")
            } else {
                format!("{count} matches")
            }
        };
        self.widget::<gtk::Label>("editor_search_status_label")
            .set_label(&text);
    }
    pub fn replace(&self, all: bool) -> io::Result<()> {
        if self
            .search
            .settings()
            .search_text()
            .is_none_or(|s| s.is_empty())
        {
            return Ok(());
        }
        let text = self.widget::<gtk::Entry>("editor_replace_entry").text();
        if all {
            let count = self.search.occurrences_count().max(0);
            self.search.replace_all(&text).map_err(io::Error::other)?;
            self.widget::<gtk::Label>("editor_search_status_label")
                .set_label(&format!("{count} replacements"));
        } else {
            if self
                .buffer
                .selection_bounds()
                .is_none_or(|(s, e)| self.search.occurrence_position(&s, &e) <= 0)
            {
                self.find(true);
            }
            if let Some((mut start, mut end)) = self.buffer.selection_bounds() {
                if self.search.occurrence_position(&start, &end) > 0 {
                    self.search
                        .replace(&mut start, &mut end, &text)
                        .map_err(io::Error::other)?;
                    self.find(true);
                }
            }
        }
        Ok(())
    }
    pub fn print_note(&self) -> io::Result<()> {
        let compositor = sourceview5::PrintCompositor::from_view(&self.view);
        compositor.set_wrap_mode(self.view.wrap_mode());
        let operation = gtk::PrintOperation::new();
        let c = compositor.clone();
        operation.connect_paginate(move |op, context| {
            let done = c.paginate(context);
            if done {
                op.set_n_pages(c.n_pages());
            }
            done
        });
        operation.connect_draw_page(move |_, context, page| compositor.draw_page(context, page));
        operation
            .run(gtk::PrintOperationAction::PrintDialog, Some(&self.window))
            .map_err(io::Error::other)?;
        Ok(())
    }
}
