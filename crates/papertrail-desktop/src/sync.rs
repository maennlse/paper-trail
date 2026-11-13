// SPDX-License-Identifier: GPL-3.0-or-later
use crate::desktop::Desktop;
use adw::prelude::*;
use gtk::{gio, glib};
use papertrail_sync::control::Command;
use std::{
    io,
    rc::Rc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

impl Desktop {
    pub fn setup_sync(self: &Rc<Self>) {
        // Some Adwaita versions only ship the generic circular-arrow symbol.
        if !gtk::IconTheme::for_display(&self.view.display())
            .has_icon("emblem-synchronizing-symbolic")
        {
            self.widget::<gtk::Image>("sync_icon")
                .set_icon_name(Some("view-refresh-symbolic"));
        }
        let menu = gio::Menu::new();
        let status = gio::MenuItem::new(None, None);
        status.set_attribute_value("custom", Some(&"sync-status-label".to_variant()));
        let section = gio::Menu::new();
        section.append_item(&status);
        menu.append_section(None, &section);
        for actions in [
            vec![("Sync Now", "sync-now")],
            vec![
                ("Configure Sync", "sync-configure"),
                ("Sign In", "sync-login"),
                ("Create Vault", "sync-create"),
                ("Unlock Vault", "sync-unlock"),
                ("Recover Vault", "sync-recover"),
                ("Change Passphrase", "sync-passphrase"),
            ],
            vec![("Lock Vault", "sync-lock"), ("Sign Out", "sync-logout")],
        ] {
            let section = gio::Menu::new();
            for (label, action) in actions {
                section.append(Some(label), Some(&format!("win.{action}")));
            }
            menu.append_section(None, &section);
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_max_width_chars(34);
        label.set_xalign(0.0);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_margin_top(6);
        label.set_margin_bottom(6);
        label.add_css_class("dim-label");
        popover.add_child(&label, "sync-status-label");
        self.builder.expose_object("sync_status_label", &label);
        self.widget::<gtk::MenuButton>("sync_menu_button")
            .set_popover(Some(&popover));
        self.action("sync-configure", |d| {
            d.sync_configure();
            Ok(())
        });
        self.action("sync-login", |d| d.sync_start(Command::Login));
        self.action("sync-now", |d| d.sync_now());
        self.action("sync-lock", |d| d.sync_start(Command::Lock));
        self.action("sync-logout", |d| d.sync_start(Command::Logout));
        for (action, kind) in [
            ("sync-create", 0),
            ("sync-unlock", 1),
            ("sync-recover", 2),
            ("sync-passphrase", 3),
        ] {
            self.action(action, move |d| {
                d.sync_secret(kind);
                Ok(())
            });
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(200), move || {
            let Some(d) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            d.sync_poll();
            let due = d
                .sync_requested
                .get()
                .is_some_and(|time| Instant::now() >= time)
                || d.sync_last.get().elapsed() >= d.sync_retry.get();
            if d.sync_unlocked.get()
                && d.sync_job.borrow().is_none()
                && due
                && d.window.visible_dialog().is_none()
            {
                if let Err(e) = d.sync_now() {
                    *d.sync_status.borrow_mut() = e.to_string();
                    d.sync_requested.set(None);
                    d.sync_last.set(Instant::now());
                    d.sync_error.set(true);
                    d.sync_indicator();
                }
            }
            glib::ControlFlow::Continue
        });
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if let Some(d) = weak.upgrade() {
                if window.is_active()
                    && d.sync_unlocked.get()
                    && d.sync_last.get().elapsed() >= Duration::from_secs(30)
                {
                    d.schedule_sync();
                }
            }
        });
        if self.sync.configured() {
            *self.sync_status.borrow_mut() =
                "Sync configured. Sign in if needed, then unlock the vault to sync this session."
                    .into();
        }
        self.sync_indicator();
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(self);
            settings.connect_notify_local(Some("gtk-enable-animations"), move |_, _| {
                if let Some(d) = weak.upgrade() {
                    d.sync_indicator();
                }
            });
        }
    }
    pub(crate) fn schedule_sync(&self) {
        if self.sync_unlocked.get() {
            self.sync_requested
                .set(Some(Instant::now() + Duration::from_secs(2)));
        }
    }
    pub(crate) fn sync_indicator(&self) {
        let busy = self.sync_job.borrow().is_some();
        let icon = self.widget::<gtk::Image>("sync_icon");
        let animate =
            gtk::Settings::default().is_some_and(|s| s.property::<bool>("gtk-enable-animations"));
        if busy && animate {
            icon.add_css_class("sync-spinning");
        } else {
            icon.remove_css_class("sync-spinning");
        }
        let button = self.widget::<gtk::MenuButton>("sync_menu_button");
        button.set_tooltip_text(Some(&format!("Sync — {}", self.sync_status.borrow())));
        if self.sync_error.get() {
            button.add_css_class("warning");
        } else {
            button.remove_css_class("warning");
        }
        self.widget::<gtk::Label>("sync_status_label")
            .set_label(&self.sync_status.borrow());
        for name in [
            "sync-configure",
            "sync-login",
            "sync-create",
            "sync-unlock",
            "sync-recover",
            "sync-passphrase",
        ] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(!busy);
            }
        }
    }
    pub(crate) fn sync_start(&self, command: Command) -> io::Result<()> {
        if self.sync_job.borrow().is_some() {
            match command {
                Command::Lock | Command::Logout => {
                    self.sync_unlocked.set(false);
                    self.sync_requested.set(None);
                    *self.sync_pending.borrow_mut() = Some(command);
                    *self.sync_status.borrow_mut() =
                        "Finishing the current sync, then locking/signing out…".into();
                    self.sync_indicator();
                    return Ok(());
                }
                Command::BackgroundSync { .. } => {
                    self.schedule_sync();
                    return Ok(());
                }
                _ => (),
            }
            return Err(io::Error::other("A sync operation is already running"));
        }
        self.sync_interactive
            .set(!matches!(&command, Command::BackgroundSync { .. }));
        *self.sync_job.borrow_mut() = Some(self.sync.submit(command));
        *self.sync_status.borrow_mut() = "Sync operation running…".into();
        self.sync_error.set(false);
        self.sync_requested.set(None);
        self.sync_last.set(Instant::now());
        self.sync_indicator();
        Ok(())
    }
    pub(crate) fn sync_now(&self) -> io::Result<()> {
        if self.sync_job.borrow().is_some() {
            self.schedule_sync();
            return Ok(());
        }
        self.persist()?;
        let model = self.model.borrow();
        let command = Command::BackgroundSync {
            settings: Box::new(model.settings.clone()),
        };
        drop(model);
        self.sync_start(command)
    }
    pub(crate) fn sync_poll(self: &Rc<Self>) {
        let result = self
            .sync_job
            .borrow()
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err(io::Error::other("Sync worker disconnected"))),
            });
        let Some(result) = result else {
            return;
        };
        self.sync_job.borrow_mut().take();
        let pending = self.sync_pending.borrow_mut().take();
        match result {
            Ok(result) => {
                let was_unlocked = self
                    .sync_unlocked
                    .replace(result.unlocked && pending.is_none());
                self.sync_retry.set(Duration::from_secs(30));
                self.sync_error.set(false);
                *self.sync_status.borrow_mut() = result.message.clone();
                if result.synced {
                    let mut refresh = result.local_changed;
                    if let Some((baseline, updated)) = result.settings_update {
                        match self.sync_metadata(&baseline, &updated) {
                            Ok(changed) => refresh |= changed,
                            Err(e) => {
                                *self.sync_status.borrow_mut() = e.to_string();
                                self.sync_error.set(true);
                            }
                        }
                    }
                    if refresh {
                        // Do not reset editor text, undo history, selection or focus.
                        let adjustment = self
                            .widget::<gtk::ScrolledWindow>("sidebar_notes_scroller")
                            .vadjustment();
                        let scroll = adjustment.value();
                        if let Err(e) = self.refresh() {
                            *self.sync_status.borrow_mut() = e.to_string();
                            self.sync_error.set(true);
                        }
                        adjustment.set_value(scroll);
                    }
                }
                if let Some(recovery) = result.recovery.filter(|_| pending.is_none()) {
                    let dialog = adw::AlertDialog::new(Some("Save Your Recovery Key"),Some(&format!("{}\n\nLosing the passphrase, this key, and all unlocked devices means your notes cannot be recovered.",result.message)));
                    let label = gtk::Label::new(Some(&recovery));
                    label.set_selectable(true);
                    label.set_wrap(true);
                    dialog.set_extra_child(Some(&label));
                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("saved", "I Saved the Key");
                    dialog.set_close_response("cancel");
                    let weak = Rc::downgrade(self);
                    dialog.connect_response(None, move |dialog, response| {
                        dialog.set_extra_child(gtk::Widget::NONE);
                        if let Some(d) = weak.upgrade() {
                            let command = if response == "saved" {
                                Command::ConfirmRecovery
                            } else {
                                Command::Lock
                            };
                            if let Err(e) = d.sync_start(command) {
                                d.error(&e);
                            }
                        }
                    });
                    dialog.present(Some(&self.window));
                } else if result.unlocked && !was_unlocked && pending.is_none() {
                    if let Err(e) = self.sync_now() {
                        self.error(&e);
                    }
                }
            }
            Err(error) => {
                *self.sync_status.borrow_mut() = error.to_string();
                self.sync_error.set(true);
                self.sync_requested.set(None);
                self.sync_retry
                    .set((self.sync_retry.get() * 2).min(Duration::from_secs(300)));
                if self.sync_interactive.get() {
                    self.error(&error);
                }
            }
        }
        self.sync_last.set(Instant::now());
        self.sync_indicator();
        if let Some(command) = pending {
            if let Err(e) = self.sync_start(command) {
                self.error(&e);
            }
        }
    }
    fn sync_metadata(
        &self,
        baseline: &papertrail_core::Settings,
        updated: &papertrail_core::Settings,
    ) -> io::Result<bool> {
        let mut model = self.model.borrow_mut();
        let old_pins = model.settings.pinned_notes.clone();
        let old_languages = model.settings.note_language_overrides.clone();
        let old_folders = model.settings.note_folders.clone();
        let old_colors = model.settings.folder_colors.clone();
        let old_labels = model.settings.folder_labels.clone();
        for path in &updated.note_folders {
            if !baseline.note_folders.contains(path) && !model.settings.note_folders.contains(path)
            {
                model.settings.add_folder(path.clone(), false);
            }
        }
        for (path, value) in &updated.folder_colors {
            if baseline.folder_colors.get(path) != Some(value)
                && model.settings.folder_colors.get(path) == baseline.folder_colors.get(path)
            {
                model
                    .settings
                    .folder_colors
                    .insert(path.clone(), value.clone());
            }
        }
        for (path, value) in &updated.folder_labels {
            if baseline.folder_labels.get(path) != Some(value)
                && model.settings.folder_labels.get(path) == baseline.folder_labels.get(path)
            {
                model
                    .settings
                    .folder_labels
                    .insert(path.clone(), value.clone());
            }
        }
        let paths: std::collections::BTreeSet<_> = baseline
            .pinned_notes
            .iter()
            .chain(&updated.pinned_notes)
            .cloned()
            .collect();
        for path in paths {
            let old = baseline.pinned_notes.contains(&path);
            let new = updated.pinned_notes.contains(&path);
            if old != new && model.settings.pinned_notes.contains(&path) == old {
                model.settings.set_pinned(&path, new);
            }
        }
        let paths: std::collections::BTreeSet<_> = baseline
            .note_language_overrides
            .keys()
            .chain(updated.note_language_overrides.keys())
            .cloned()
            .collect();
        for path in paths {
            let old = baseline.note_language_overrides.get(&path);
            let new = updated.note_language_overrides.get(&path);
            if old != new && model.settings.note_language_overrides.get(&path) == old {
                if let Some(new) = new {
                    model
                        .settings
                        .note_language_overrides
                        .insert(path, new.clone());
                } else {
                    model.settings.note_language_overrides.remove(&path);
                }
            }
        }
        let changed = old_pins != model.settings.pinned_notes
            || old_languages != model.settings.note_language_overrides
            || old_folders != model.settings.note_folders
            || old_colors != model.settings.folder_colors
            || old_labels != model.settings.folder_labels;
        drop(model);
        if changed {
            self.persist()?;
        }
        Ok(changed)
    }
    fn sync_secret(self: &Rc<Self>, kind: u8) {
        let title = match kind {
            0 => "Create Encrypted Vault",
            1 => "Unlock Vault",
            2 => "Recover Vault",
            _ => "Change Vault Passphrase",
        };
        let dialog = adw::AlertDialog::new(Some(title),Some("Encryption is separate from sign-in. Local note files remain plaintext. New passphrases must contain at least 16 characters."));
        let group = adw::PreferencesGroup::new();
        let password = adw::PasswordEntryRow::new();
        password.set_title(if kind == 2 {
            "Recovery key"
        } else {
            "Vault passphrase"
        });
        group.add(&password);
        let confirm = adw::PasswordEntryRow::new();
        if kind == 0 || kind == 3 {
            confirm.set_title("Confirm passphrase");
            group.add(&confirm);
        }
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Continue");
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let secret = Zeroizing::new(password.text().to_string());
            let repeated = Zeroizing::new(confirm.text().to_string());
            password.set_text("");
            confirm.set_text("");
            if response != "ok" {
                return;
            }
            if let Some(d) = weak.upgrade() {
                if (kind == 0 || kind == 3) && secret.as_str() != repeated.as_str() {
                    d.error(&io::Error::other("Passphrases do not match"));
                    return;
                }
                let command = match kind {
                    0 => Command::Create(secret),
                    1 => Command::Unlock(secret),
                    2 => Command::Recover(secret),
                    _ => Command::ChangePassphrase(secret),
                };
                if let Err(e) = d.sync_start(command) {
                    d.error(&e);
                }
            }
        });
        dialog.present(Some(&self.window));
    }
    fn sync_configure(self: &Rc<Self>) {
        let settings = self.model.borrow().settings.clone();
        let initial = match self.sync.saved_config() {
            Ok(value) => value,
            Err(e) => {
                self.error(&e);
                return;
            }
        };
        let state = match self.sync.snapshot() {
            Ok(value) => value,
            Err(e) => {
                self.error(&e);
                return;
            }
        };
        let choices = match self.sync.folder_choices(&settings) {
            Ok(value) => value,
            Err(e) => {
                self.error(&e);
                return;
            }
        };
        let bound = state.as_ref().is_some_and(|s| !s.vault_id.is_empty());
        // Configure Sync is rebuilt from current state on every opening. Its
        // transient rows must not reuse names in the permanent window builder.
        let config_widgets = gtk::Builder::new();
        self.sync_config_widgets
            .replace(Some(config_widgets.clone()));
        let dialog = adw::AlertDialog::new(Some("Configure Encrypted Sync"),Some("Choose folders to sync. Unchecking keeps local and remote notes. Mapping a different directory copies notes there without removing the originals; close its open notes first. Server/account settings are fixed once a vault is bound."));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        let group = adw::PreferencesGroup::new();
        group.set_title("Account");
        let server = adw::EntryRow::new();
        server.set_title("HTTPS sync server");
        server.set_text(initial.as_ref().map(|c| c.server.as_str()).unwrap_or(""));
        server.set_editable(!bound);
        group.add(&server);
        let discovery = adw::SwitchRow::new();
        discovery.set_title("Discover authentication from sync server");
        discovery.set_subtitle("Turn off for manual configuration or older servers");
        discovery.set_active(initial.as_ref().is_none_or(|c| c.discover_auth));
        group.add(&discovery);
        let issuer = adw::EntryRow::new();
        issuer.set_title("Exact HTTPS OIDC issuer");
        issuer.set_text(initial.as_ref().map(|c| c.issuer.as_str()).unwrap_or(""));
        issuer.set_editable(!bound);
        group.add(&issuer);
        let client = adw::EntryRow::new();
        client.set_title("Public native client ID");
        client.set_text(initial.as_ref().map(|c| c.client_id.as_str()).unwrap_or(""));
        client.set_editable(!bound);
        group.add(&client);
        let scopes = adw::EntryRow::new();
        scopes.set_title("Scopes");
        scopes.set_text(
            initial
                .as_ref()
                .map(|c| c.scopes.as_str())
                .unwrap_or("openid offline_access"),
        );
        group.add(&scopes);
        for row in [&issuer, &client, &scopes] {
            discovery
                .bind_property("active", row, "sensitive")
                .invert_boolean()
                .sync_create()
                .build();
        }
        content.append(&group);
        for (id, row) in [
            ("sync_config_server", &server),
            ("sync_config_issuer", &issuer),
            ("sync_config_client", &client),
            ("sync_config_scopes", &scopes),
        ] {
            config_widgets.expose_object(id, row);
        }
        config_widgets.expose_object("sync_config_discovery", &discovery);
        let downloads = adw::EntryRow::new();
        downloads.set_title("New remote folders are downloaded into");
        let root = initial
            .as_ref()
            .and_then(|c| c.download_root().ok())
            .unwrap_or_else(|| {
                settings
                    .notes_dir
                    .parent()
                    .unwrap_or(&settings.notes_dir)
                    .join("Paper Trail Synced")
            });
        downloads.set_text(&root.to_string_lossy());
        let folders = adw::PreferencesGroup::new();
        folders.set_title("Folders");
        folders.add(&downloads);
        let auto_download = adw::SwitchRow::new();
        auto_download.set_title("Download folders created on other devices");
        auto_download.set_active(initial.as_ref().is_none_or(|c| c.auto_download));
        folders.add(&auto_download);
        let mut rows = Vec::new();
        let mut conflicts = Vec::new();
        for choice in choices {
            let row = adw::ActionRow::new();
            row.set_use_markup(false);
            row.set_title(&choice.name);
            let selected = gtk::CheckButton::new();
            selected.set_valign(gtk::Align::Center);
            selected.set_active(
                choice.selected
                    || (initial.is_none() && choice.path.as_ref() == Some(&settings.notes_dir)),
            );
            row.add_prefix(&selected);
            row.set_activatable_widget(Some(&selected));
            let path = Rc::new(std::cell::RefCell::new(choice.path.clone()));
            row.set_subtitle(&format!(
                "{} · {}",
                choice
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Automatic download directory".into()),
                choice.id.as_deref().unwrap_or("New synced folder")
            ));
            if choice.id.is_some() {
                let choose = gtk::Button::from_icon_name("folder-open-symbolic");
                choose.set_tooltip_text(Some("Choose local directory"));
                choose.set_valign(gtk::Align::Center);
                choose.add_css_class("flat");
                row.add_suffix(&choose);
                let weak = Rc::downgrade(self);
                let chosen = path.clone();
                let weak_row = row.downgrade();
                choose.connect_clicked(move |_| {
                    if let Some(d) = weak.upgrade() {
                        let files = gtk::FileDialog::builder()
                            .title("Map Synced Folder to Local Directory")
                            .build();
                        let chosen = chosen.clone();
                        let weak_row = weak_row.clone();
                        files.select_folder(
                            Some(&d.window),
                            None::<&gio::Cancellable>,
                            move |result| {
                                if let Ok(file) = result {
                                    if let Some(path) = file.path() {
                                        if let Some(row) = weak_row.upgrade() {
                                            row.set_subtitle(&path.to_string_lossy());
                                        }
                                        *chosen.borrow_mut() = Some(path);
                                    }
                                }
                            },
                        );
                    }
                });
            }
            folders.add(&row);
            if choice.conflict {
                let accept = gtk::CheckButton::with_label(
                    "Accept remote folder name/color (keep local directory)",
                );
                folders.add(&accept);
                conflicts.push((choice.id.clone().unwrap(), accept));
            }
            rows.push((choice.id, path, selected));
        }
        content.append(&folders);
        let mut legacy_rows = Vec::new();
        if let Some(state) = &state {
            if !state.legacy_pending.is_empty() {
                let legacy = adw::PreferencesGroup::new();
                legacy.set_title("Confirm Legacy Folder Assignments");
                let catalog = state.catalog.as_ref().or(state.catalog_draft.as_ref());
                let options: Vec<_> = catalog
                    .map(|c| {
                        c.folders
                            .iter()
                            .map(|(id, data)| (id.clone(), format!("{} ({})", data.name, &id[..8])))
                            .collect()
                    })
                    .unwrap_or_default();
                let names: Vec<_> = std::iter::once("Choose a folder".to_owned())
                    .chain(options.iter().map(|(_, name)| name.clone()))
                    .collect();
                let list =
                    gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
                for note in &state.legacy_pending {
                    let row = adw::ComboRow::new();
                    row.set_use_markup(false);
                    row.set_title(&note.filename);
                    row.set_subtitle(&format!("Legacy folder: {} · {}", note.folder, note.id));
                    row.set_model(Some(&list));
                    row.set_selected(0);
                    legacy.add(&row);
                    legacy_rows.push((note.id.clone(), row, options.clone()));
                }
                content.append(&legacy);
            }
        }
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(360)
            .max_content_height(520)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        dialog.set_extra_child(Some(&scroll));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("save", "Save");
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "save" {
                return;
            }
            if let Some(d) = weak.upgrade() {
                let mut config = initial.clone().unwrap_or_default();
                config.version = 2;
                config.server = server.text().trim().into();
                config.issuer = issuer.text().trim().into();
                config.client_id = client.text().trim().into();
                config.scopes = scopes.text().into();
                config.discover_auth = discovery.is_active();
                config.download_dir = Some(downloads.text().trim().into());
                config.folders.clear();
                config.auto_download = auto_download.is_active();
                for (id, path, selected) in &rows {
                    if let Some(id) = id {
                        if selected.is_active() {
                            config.excluded_folders.remove(id);
                            config.selected_folders.insert(id.clone());
                        } else {
                            config.excluded_folders.insert(id.clone());
                            config.selected_folders.remove(id);
                        }
                        if let Some(path) = &*path.borrow() {
                            config.folder_mappings.insert(id.clone(), path.clone());
                        }
                    }
                    if selected.is_active() {
                        if let Some(path) = &*path.borrow() {
                            if !config.folders.contains(path) {
                                config.folders.push(path.clone());
                            }
                        }
                    }
                }
                for (id, row, options) in &legacy_rows {
                    if let Some((folder, _)) = row
                        .selected()
                        .checked_sub(1)
                        .and_then(|i| options.get(i as usize))
                    {
                        config
                            .legacy_note_mappings
                            .insert(id.clone(), folder.clone());
                    }
                }
                let accepted = conflicts
                    .iter()
                    .filter(|(_, accept)| accept.is_active())
                    .map(|(id, _)| id.clone())
                    .collect();
                if config.discover_auth {
                    d.sync_discover(config, initial.clone(), accepted);
                } else {
                    d.sync_save_configuration(config, initial.as_ref(), &accepted);
                }
            }
        });
        let weak = Rc::downgrade(self);
        dialog.connect_closed(move |_| {
            if let Some(d) = weak.upgrade() {
                // An older dialog may finish its closing animation after a new
                // one opens. Do not discard the new dialog's widget registry.
                let current = d.sync_config_widgets.borrow().as_ref() == Some(&config_widgets);
                if current {
                    d.sync_config_widgets.replace(None);
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn sync_save_configuration(
        &self,
        config: papertrail_sync::Config,
        previous: Option<&papertrail_sync::Config>,
        accepted: &[String],
    ) {
        let result = (|| {
            if self.sync.saved_config()?.as_ref() != previous {
                return Err(io::Error::other(
                    "Sync configuration changed during setup; reopen Configure Sync",
                ));
            }
            self.sync.configure(config)?;
            for id in accepted {
                self.sync
                    .accept_remote_folder(id, &self.model.borrow().settings)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                *self.sync_status.borrow_mut() =
                    "Configuration saved. Sign in/unlock if needed; selected folders will sync."
                        .into();
                self.sync_indicator();
                self.schedule_sync();
            }
            Err(e) => self.error(&e),
        }
    }

    fn sync_discover(
        self: &Rc<Self>,
        config: papertrail_sync::Config,
        previous: Option<papertrail_sync::Config>,
        accepted: Vec<String>,
    ) {
        let progress = adw::AlertDialog::new(
            Some("Discovering Authentication"),
            Some("Contacting the sync server and validating its identity provider…"),
        );
        progress.add_response("cancel", "Cancel");
        progress.set_close_response("cancel");
        let cancelled = Rc::new(std::cell::Cell::new(false));
        let flag = cancelled.clone();
        progress.connect_response(None, move |_, _| flag.set(true));
        progress.present(Some(&self.window));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(papertrail_sync::auth::config::prepare(config));
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(d) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if cancelled.get() {
                return glib::ControlFlow::Break;
            }
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(_) => Err(io::Error::other(
                    "Authentication discovery worker disconnected",
                )),
            };
            progress.close();
            match result {
                Err(e) => d.error(&e),
                Ok(config) => d.sync_confirm_discovered(config, previous.clone(), accepted.clone()),
            }
            glib::ControlFlow::Break
        });
    }
    pub(crate) fn sync_confirm_discovered(
        self: &Rc<Self>,
        config: papertrail_sync::Config,
        previous: Option<papertrail_sync::Config>,
        accepted: Vec<String>,
    ) {
        if papertrail_sync::auth::config::needs_confirmation(previous.as_ref(), &config) {
            let confirm = adw::AlertDialog::new(
                Some("Trust This Identity Provider?"),
                Some(&papertrail_sync::auth::config::trust_description(
                    previous.as_ref(),
                    &config,
                )),
            );
            confirm.set_body_use_markup(false);
            confirm.add_response("cancel", "Cancel");
            confirm.add_response("accept", "Trust Identity Provider");
            confirm.set_close_response("cancel");
            confirm.set_default_response(Some("cancel"));
            let weak = Rc::downgrade(self);
            confirm.connect_response(None, move |_, response| {
                if response == "accept" {
                    if let Some(d) = weak.upgrade() {
                        d.sync_save_configuration(config.clone(), previous.as_ref(), &accepted);
                    }
                }
            });
            confirm.present(Some(&self.window));
        } else {
            self.sync_save_configuration(config, previous.as_ref(), &accepted);
        }
    }
}
