// SPDX-License-Identifier: GPL-3.0-or-later
use crate::desktop::Desktop;
use adw::prelude::*;
use gtk::{gio, glib};
use papertrail_sync::control::{Command, PassphraseAction};
use std::{
    io,
    rc::Rc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct FolderDeletionView {
    id: String,
    dialog: glib::WeakRef<adw::AlertDialog>,
    folders: glib::WeakRef<adw::PreferencesGroup>,
    row: glib::WeakRef<adw::ActionRow>,
    selected: glib::WeakRef<gtk::CheckButton>,
    conflict: Option<glib::WeakRef<gtk::CheckButton>>,
}

impl FolderDeletionView {
    pub(crate) fn new(
        id: &str,
        dialog: &adw::AlertDialog,
        folders: &adw::PreferencesGroup,
        row: &adw::ActionRow,
        selected: &gtk::CheckButton,
        conflict: Option<&gtk::CheckButton>,
    ) -> Self {
        Self {
            id: id.into(),
            dialog: dialog.downgrade(),
            folders: folders.downgrade(),
            row: row.downgrade(),
            selected: selected.downgrade(),
            conflict: conflict.map(|w| w.downgrade()),
        }
    }

    pub(crate) fn set_busy(&self, busy: bool) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.set_response_enabled("save", !busy);
        }
        if let Some(row) = self.row.upgrade() {
            row.set_sensitive(!busy);
        }
        if let Some(conflict) = self.conflict.as_ref().and_then(|w| w.upgrade()) {
            conflict.set_sensitive(!busy);
        }
    }

    fn finish(self, desktop: &Rc<Desktop>) {
        self.set_busy(false);
        let (Some(dialog), Some(folders), Some(row)) = (
            self.dialog.upgrade(),
            self.folders.upgrade(),
            self.row.upgrade(),
        ) else {
            return;
        };
        if !dialog.is_visible() {
            return;
        }
        match desktop.sync.snapshot() {
            Ok(Some(state)) => {
                let Some(catalog) = state.catalog else { return };
                if !catalog.deleted_folders.contains(&self.id) {
                    return;
                }
                if let Some(selected) = self.selected.upgrade() {
                    selected.set_active(false);
                }
                if let Some(conflict) = self.conflict.and_then(|w| w.upgrade()) {
                    conflict.set_active(false);
                    folders.remove(&conflict);
                }
                folders.remove(&row);
                if let (Some(binding), Some(data)) =
                    (state.bindings.get(&self.id), catalog.folders.get(&self.id))
                {
                    if !binding.path.is_dir() {
                        return;
                    }
                    folders.add(&desktop.sync_local_folder_row(
                        &self.id,
                        &data.name,
                        &binding.path,
                        &dialog,
                    ));
                }
            }
            Ok(None) => (),
            Err(error) => desktop.error(&error),
        }
    }
}

fn replace_menu_action(menu: &gio::Menu, index: i32, label: &str, action: &str) {
    let action = format!("win.{action}");
    let unchanged = menu
        .item_attribute_value(index, gio::MENU_ATTRIBUTE_LABEL, None)
        .and_then(|value| value.get::<String>())
        .as_deref()
        == Some(label)
        && menu
            .item_attribute_value(index, gio::MENU_ATTRIBUTE_ACTION, None)
            .and_then(|value| value.get::<String>())
            .as_deref()
            == Some(action.as_str());
    if !unchanged {
        menu.remove(index);
        menu.insert(index, Some(label), Some(&action));
    }
}

fn show_create_vault(menu: &gio::Menu, show: bool) {
    let visible = menu
        .item_attribute_value(2, "action", None)
        .and_then(|value| value.get::<String>())
        .as_deref()
        == Some("win.sync-create");
    if show && !visible {
        menu.insert(2, Some("Create Vault"), Some("win.sync-create"));
    } else if !show && visible {
        menu.remove(2);
    }
}

fn login_prompt_kind(vault_exists: Option<bool>) -> u8 {
    // A failed/unknown lookup must never start creation of a replacement vault.
    if vault_exists == Some(false) {
        0
    } else {
        1
    }
}

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
        let operation = gio::Menu::new();
        operation.append(Some("Sync Now"), Some("win.sync-now"));
        menu.append_section(None, &operation);
        self.builder
            .expose_object("sync_operation_menu", &operation);
        let actions = gio::Menu::new();
        for (label, action) in [
            ("Configure Sync", "sync-configure"),
            ("Sign In", "sync-login"),
            ("Create Vault", "sync-create"),
            ("Unlock Vault", "sync-unlock"),
            ("Change Passphrase", "sync-passphrase"),
        ] {
            actions.append(Some(label), Some(&format!("win.{action}")));
        }
        menu.append_section(None, &actions);
        self.builder.expose_object("sync_actions_menu", &actions);
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
        self.action("sync-cancel", |d| {
            d.sync_cancel_operation();
            Ok(())
        });
        self.action("sync-lock", |d| d.sync_start(Command::Lock));
        self.action("sync-logout", |d| d.sync_start(Command::Logout));
        for (action, kind) in [
            ("sync-create", 0),
            ("sync-unlock", 1),
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
                && !d.sync_paused.get()
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
            if let Err(e) = self.sync_start(Command::Restore) {
                *self.sync_status.borrow_mut() = e.to_string();
            }
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
        if self.sync_unlocked.get() && !self.sync_paused.get() {
            self.sync_requested
                .set(Some(Instant::now() + Duration::from_secs(2)));
        }
    }
    pub(crate) fn sync_indicator(&self) {
        let busy = self.sync_job.borrow().is_some();
        let cancelled = self
            .sync_cancel
            .borrow()
            .as_ref()
            .is_some_and(|c| c.is_cancelled());
        let cancellable = busy && self.sync_cancel.borrow().is_some();
        let (label, action) = if cancelled && busy {
            ("Cancelling…", "sync-cancel")
        } else if cancellable {
            ("Cancel Operation", "sync-cancel")
        } else if busy {
            ("Operation Running…", "sync-now")
        } else {
            ("Sync Now", "sync-now")
        };
        replace_menu_action(
            &self.widget::<gio::Menu>("sync_operation_menu"),
            0,
            label,
            action,
        );
        let actions = self.widget::<gio::Menu>("sync_actions_menu");
        let (label, action) = if self.sync.signed_in() {
            ("Sign Out", "sync-logout")
        } else {
            ("Sign In", "sync-login")
        };
        replace_menu_action(&actions, 1, label, action);
        let create_visible = self.sync.vault_exists() != Some(true);
        show_create_vault(&actions, create_visible);
        let (label, action) = if self.sync_unlocked.get() {
            ("Lock Vault", "sync-lock")
        } else {
            ("Unlock Vault", "sync-unlock")
        };
        replace_menu_action(&actions, if create_visible { 3 } else { 2 }, label, action);
        if let Some(action) = self
            .window
            .lookup_action("sync-now")
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(!busy);
        }
        if let Some(action) = self
            .window
            .lookup_action("sync-cancel")
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(cancellable && !cancelled);
        }
        let icon = self.widget::<gtk::Image>("sync_icon");
        let animate =
            gtk::Settings::default().is_some_and(|s| s.property::<bool>("gtk-enable-animations"));
        if busy && animate {
            icon.add_css_class("sync-spinning");
        } else {
            icon.remove_css_class("sync-spinning");
        }
        let button = self.widget::<gtk::MenuButton>("sync_menu_button");
        let tooltip = format!("Sync — {}", self.sync_status.borrow());
        if button.tooltip_text().as_deref() != Some(tooltip.as_str()) {
            button.set_tooltip_text(Some(&tooltip));
        }
        if self.sync_error.get() {
            button.add_css_class("warning");
        } else {
            button.remove_css_class("warning");
        }
        let label = self.widget::<gtk::Label>("sync_status_label");
        if label.text().as_str() != self.sync_status.borrow().as_str() {
            label.set_label(&self.sync_status.borrow());
        }
        for name in [
            "sync-configure",
            "sync-login",
            "sync-unlock",
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
        if let Some(action) = self
            .window
            .lookup_action("sync-create")
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(!busy && create_visible);
        }
    }
    pub(crate) fn sync_cancel_operation(&self) {
        let cancel = self.sync_cancel.borrow();
        let Some(cancel) = cancel.as_ref() else {
            return;
        };
        if cancel.cancel() {
            if let Some(launch) = self.sync_browser_launch.borrow().as_ref() {
                launch.cancel();
            }
            self.sync_paused.set(true);
            self.sync_requested.set(None);
            *self.sync_status.borrow_mut() =
                "Cancelling… Waiting for the current request to finish.".into();
        } else {
            *self.sync_status.borrow_mut() =
                "Sign-in already approved; finishing credential storage…".into();
        }
        self.sync_indicator();
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
        if matches!(&command, Command::Lock | Command::Logout) {
            self.sync_unlocked.set(false);
            self.sync_requested.set(None);
        }
        self.sync_prompt_after_login.set(matches!(
            &command,
            Command::Login | Command::BrowserLogin { .. }
        ));
        let command = if matches!(&command, Command::Login) {
            let (events, receiver) = std::sync::mpsc::channel();
            *self.sync_browser_events.borrow_mut() = Some(receiver);
            Command::BrowserLogin { events }
        } else {
            command
        };
        self.sync_interactive.set(!matches!(
            &command,
            Command::BackgroundSync { .. } | Command::Restore
        ));
        let cancel = matches!(
            &command,
            Command::Login
                | Command::BrowserLogin { .. }
                | Command::Restore
                | Command::Sync { .. }
                | Command::BackgroundSync { .. }
        )
        .then(papertrail_sync::control::Cancellation::default);
        let message = match &command {
            Command::BrowserLogin { .. } => "Starting browser sign-in…",
            Command::Restore => "Restoring saved sign-in… This does not open a browser.",
            Command::DeleteFolder { .. } => {
                "Deleting folder from server… Local files will be kept."
            }
            _ => "Sync operation running…",
        };
        let job = if let Some(cancel) = &cancel {
            self.sync.submit_cancellable(command, cancel.clone())
        } else {
            self.sync.submit(command)
        };
        *self.sync_cancel.borrow_mut() = cancel;
        *self.sync_job.borrow_mut() = Some(job);
        *self.sync_status.borrow_mut() = message.into();
        self.sync_error.set(false);
        self.sync_requested.set(None);
        self.sync_last.set(Instant::now());
        self.sync_indicator();
        Ok(())
    }
    pub(crate) fn sync_now(&self) -> io::Result<()> {
        self.sync_paused.set(false);
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
        self.sync_poll_browser();
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
        let prompt_after_login = self.sync_prompt_after_login.replace(false);
        self.sync_job.borrow_mut().take();
        self.sync_browser_events.borrow_mut().take();
        if let Some(launch) = self.sync_browser_launch.borrow_mut().take() {
            launch.cancel();
        }
        let cancelled = self
            .sync_cancel
            .borrow_mut()
            .take()
            .is_some_and(|c| c.is_cancelled());
        if cancelled {
            self.sync_paused.set(true);
            self.sync_requested.set(None);
        }
        let pending = self.sync_pending.borrow_mut().take();
        let folder_deletion = self.sync_folder_deletion.borrow_mut().take();
        match result {
            Ok(result) => {
                let was_unlocked = self
                    .sync_unlocked
                    .replace(result.unlocked && pending.is_none());
                self.sync_retry.set(Duration::from_secs(30));
                self.sync_error.set(false);
                *self.sync_status.borrow_mut() = result.message.clone();
                if let Some(warning) = &result.warning {
                    *self.sync_status.borrow_mut() = format!("{}. {warning}", result.message);
                    self.sync_error.set(true);
                    if self.sync_interactive.get() {
                        self.error(&io::Error::other(warning.clone()));
                    }
                }
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
                } else if prompt_after_login
                    && !result.unlocked
                    && !cancelled
                    && pending.is_none()
                    && self.window.visible_dialog().is_none()
                {
                    self.sync_secret(login_prompt_kind(self.sync.vault_exists()));
                } else if result.unlocked
                    && !was_unlocked
                    && pending.is_none()
                    && result.warning.is_none()
                    && !cancelled
                {
                    if let Err(e) = self.sync_now() {
                        self.error(&e);
                    }
                } else if result.unlocked && result.warning.is_some() {
                    self.schedule_sync();
                }
            }
            Err(error) => {
                *self.sync_status.borrow_mut() = if cancelled {
                    "Operation cancelled. Automatic sync paused; choose Sync Now to resume.".into()
                } else {
                    error.to_string()
                };
                let interrupted = cancelled || error.kind() == io::ErrorKind::Interrupted;
                self.sync_error.set(!interrupted);
                self.sync_requested.set(None);
                self.sync_retry
                    .set((self.sync_retry.get() * 2).min(Duration::from_secs(300)));
                if self.sync_interactive.get() && !interrupted {
                    self.error(&error);
                }
            }
        }
        self.sync_last.set(Instant::now());
        self.sync_indicator();
        if let Some(view) = folder_deletion {
            view.finish(self);
        }
        if let Some(command) = pending {
            if let Err(e) = self.sync_start(command) {
                self.error(&e);
            }
        }
    }
    fn sync_poll_browser(&self) {
        use papertrail_sync::auth::BrowserEvent;
        loop {
            let event = self
                .sync_browser_events
                .borrow()
                .as_ref()
                .and_then(|rx| rx.try_recv().ok());
            let Some(event) = event else { break };
            let cancelled = self
                .sync_cancel
                .borrow()
                .as_ref()
                .is_some_and(|c| c.is_cancelled());
            match event {
                BrowserEvent::Progress(message) if !cancelled => {
                    *self.sync_status.borrow_mut() = message.into();
                    self.sync_indicator();
                }
                BrowserEvent::Progress(_) => (),
                BrowserEvent::Open { url, reply } => {
                    if cancelled {
                        let _ = reply.send(Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Sign-in cancelled",
                        )));
                        continue;
                    }
                    let launcher = gtk::UriLauncher::new(&url);
                    let cancel = gio::Cancellable::new();
                    launcher.launch(Some(&self.window), Some(&cancel), move |result| {
                        let _ = reply.send(result.map_err(|_| io::Error::other(
                            "Could not open the system browser; check the default HTTPS browser and desktop portal",
                        )));
                    });
                    *self.sync_browser_launch.borrow_mut() = Some(cancel);
                }
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
        // A worker may finish after Close Folder changed the device's selection.
        // Never let its stale settings snapshot put that bookmark back.
        let enabled: Vec<_> = self
            .sync
            .folder_choices(&model.settings)?
            .into_iter()
            .filter(|choice| choice.selected)
            .filter_map(|choice| choice.path)
            .collect();
        let folder_enabled = |path: &std::path::Path| {
            enabled
                .iter()
                .any(|p| papertrail_sync::same_directory(p, path))
        };
        for path in &updated.note_folders {
            if folder_enabled(path)
                && !baseline.note_folders.contains(path)
                && !model.settings.note_folders.contains(path)
            {
                model.settings.add_folder(path.clone(), false);
            }
        }
        for (path, value) in &updated.folder_colors {
            if folder_enabled(path)
                && baseline.folder_colors.get(path) != Some(value)
                && model.settings.folder_colors.get(path) == baseline.folder_colors.get(path)
            {
                model
                    .settings
                    .folder_colors
                    .insert(path.clone(), value.clone());
            }
        }
        for (path, value) in &updated.folder_labels {
            if folder_enabled(path)
                && baseline.folder_labels.get(path) != Some(value)
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
        self.widget::<gtk::MenuButton>("sync_menu_button").popdown();
        let title = match kind {
            0 => "Create Encrypted Vault",
            1 => "Unlock Vault",
            2 => "Recover Vault",
            _ => "Change Vault Passphrase",
        };
        let body = match kind {
            1 => "Enter your vault passphrase to unlock encrypted sync. If you have forgotten it, use your recovery key instead. Local note files remain plaintext.",
            2 => "Enter the recovery key saved when you created the vault or last changed its passphrase. This unlocks your existing vault; it does not reset sign-in. Recovery keys are never saved.",
            _ => "Encryption is separate from sign-in. Local note files remain plaintext. New passphrases must contain at least 16 characters.",
        };
        let dialog = adw::AlertDialog::new(Some(title), Some(body));
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
        let remember = adw::SwitchRow::new();
        remember.set_title("Remember encryption password on this device");
        remember.set_subtitle("Automatically unlock and sync at startup. Anyone with access to your unlocked keyring can unlock sync.");
        if kind != 2 {
            group.add(&remember);
        }
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Cancel");
        if kind == 1 {
            dialog.add_response("recover", "Use Recovery Key Instead");
        } else if kind == 2 {
            dialog.add_response("password", "Use Passphrase Instead");
        }
        dialog.add_response("ok", "Continue");
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |dialog, response| {
            let secret = Zeroizing::new(password.text().to_string());
            let repeated = Zeroizing::new(confirm.text().to_string());
            password.set_text("");
            confirm.set_text("");
            if (kind == 1 && response == "recover") || (kind == 2 && response == "password") {
                dialog.close();
                if let Some(d) = weak.upgrade() {
                    d.sync_secret(if kind == 1 { 2 } else { 1 });
                }
                return;
            }
            if response != "ok" {
                return;
            }
            if let Some(d) = weak.upgrade() {
                if (kind == 0 || kind == 3) && secret.as_str() != repeated.as_str() {
                    d.error(&io::Error::other("Passphrases do not match"));
                    return;
                }
                let command = if kind == 2 {
                    Command::Recover(secret)
                } else {
                    Command::Passphrase {
                        action: match kind {
                            0 => PassphraseAction::Create,
                            1 => PassphraseAction::Unlock,
                            _ => PassphraseAction::Change,
                        },
                        passphrase: secret,
                        remember: remember.is_active(),
                    }
                };
                if let Err(e) = d.sync_start(command) {
                    d.error(&e);
                }
            }
        });
        dialog.present(Some(&self.window));
    }
    fn sync_local_folder_row(
        self: &Rc<Self>,
        id: &str,
        name: &str,
        path: &std::path::Path,
        dialog: &adw::AlertDialog,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(name);
        row.set_subtitle(&format!("Local only · {}", path.display()));
        let again = gtk::Button::with_label("Sync as New Folder…");
        again.add_css_class("flat");
        again.set_valign(gtk::Align::Center);
        again.set_sensitive(self.sync_job.borrow().is_none());
        row.add_suffix(&again);
        let weak = Rc::downgrade(self);
        let id = id.to_owned();
        let name = name.to_owned();
        let config_dialog = dialog.downgrade();
        again.connect_clicked(move |_| {
            let Some(d) = weak.upgrade() else { return };
            let confirm = adw::AlertDialog::new(
                Some(&format!("Sync “{name}” as a new folder?")),
                Some("Uploads the local notes as a new synced folder with new note identities. The deleted folder and its history will not be restored. Other devices may download a separate copy."),
            );
            confirm.add_responses(&[("cancel", "Cancel"), ("sync", "Sync as New Folder")]);
            confirm.set_default_response(Some("cancel"));
            confirm.set_close_response("cancel");
            let weak = Rc::downgrade(&d);
            let id = id.clone();
            let config_dialog = config_dialog.clone();
            confirm.connect_response(None, move |_, response| {
                if response != "sync" { return; }
                if let Some(d) = weak.upgrade() {
                    match d.sync.sync_as_new_folder(&id) {
                        Ok(()) => {
                            if let Some(dialog) = config_dialog.upgrade() { dialog.close(); }
                            d.schedule_sync();
                        }
                        Err(e) => d.error(&e),
                    }
                }
            });
            confirm.present(Some(&d.window));
        });
        row
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
        let dialog = adw::AlertDialog::new(Some("Configure Encrypted Sync"),Some("Choose folders to sync. Unchecking keeps local and remote notes. The trash button permanently deletes a folder’s notes and history from the server, keeping local files on all devices. Mapping a different directory copies notes there without removing the originals; close its open notes first. Server/account settings are fixed once a vault is bound."));
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
            let conflict = choice.conflict.then(|| {
                gtk::CheckButton::with_label(
                    "Accept remote folder name/color (keep local directory)",
                )
            });
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
            if let Some(id) = &choice.id {
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
                if state
                    .as_ref()
                    .and_then(|s| s.catalog.as_ref())
                    .is_some_and(|catalog| catalog.folders.contains_key(id))
                {
                    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
                    delete.set_tooltip_text(Some(
                        if self.sync_unlocked.get() && self.sync.signed_in() {
                            "Delete from Server…"
                        } else {
                            "Delete from Server… (sign in and unlock the vault first)"
                        },
                    ));
                    delete.update_property(&[gtk::accessible::Property::Label(
                        "Delete from Server…",
                    )]);
                    delete.set_valign(gtk::Align::Center);
                    delete.add_css_class("flat");
                    delete.set_sensitive(
                        self.sync_unlocked.get()
                            && self.sync.signed_in()
                            && self.sync_job.borrow().is_none(),
                    );
                    row.add_suffix(&delete);
                    let weak = Rc::downgrade(self);
                    let deletion_view = FolderDeletionView::new(
                        id,
                        &dialog,
                        &folders,
                        &row,
                        &selected,
                        conflict.as_ref(),
                    );
                    let id = id.clone();
                    let name = choice.name.clone();
                    delete.connect_clicked(move |_| {
                    let Some(d) = weak.upgrade() else { return };
                    let confirm = adw::AlertDialog::new(
                        Some(&format!("Delete “{name}” from the server?")),
                        Some("Permanently deletes this folder’s synced notes and revision history. Local files on all devices are kept.\n\nOther devices will stop syncing this folder when they next connect. It won’t be uploaded again automatically.\n\nServer backups are managed separately by the server operator."),
                    );
                    confirm.add_responses(&[("cancel", "Cancel"), ("delete", "Delete from Server")]);
                    confirm.set_default_response(Some("cancel"));
                    confirm.set_close_response("cancel");
                    confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                    let weak = Rc::downgrade(&d);
                    let id = id.clone();
                    let deletion_view = deletion_view.clone();
                    confirm.connect_response(None, move |_, response| {
                        if response != "delete" { return; }
                        if let Some(d) = weak.upgrade() {
                            let settings = Box::new(d.model.borrow().settings.clone());
                            match d.sync_start(Command::DeleteFolder { id: id.clone(), settings }) {
                                Ok(()) => {
                                    deletion_view.set_busy(true);
                                    d.sync_folder_deletion.replace(Some(deletion_view.clone()));
                                }
                                Err(e) => d.error(&e),
                            }
                        }
                    });
                    confirm.present(Some(&d.window));
                });
                }
            }
            folders.add(&row);
            if let Some(accept) = conflict {
                folders.add(&accept);
                conflicts.push((choice.id.clone().unwrap(), accept));
            }
            rows.push((choice.id, path, selected));
        }
        content.append(&folders);
        if let Some(state) = &state {
            if let Some(catalog) = &state.catalog {
                for id in &catalog.deleted_folders {
                    let Some(binding) = state.bindings.get(id) else {
                        continue;
                    };
                    if !binding.path.is_dir() {
                        continue;
                    }
                    let Some(data) = catalog.folders.get(id) else {
                        continue;
                    };
                    folders.add(&self.sync_local_folder_row(
                        id,
                        &data.name,
                        &binding.path,
                        &dialog,
                    ));
                }
            }
        }
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
                if !self.sync.signed_in() {
                    self.sync_unlocked.set(false);
                }
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

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn login_prompt_only_offers_creation_for_confirmed_absence() {
        assert_eq!(login_prompt_kind(Some(false)), 0);
        assert_eq!(login_prompt_kind(Some(true)), 1);
        assert_eq!(login_prompt_kind(None), 1);
    }

    #[test]
    fn hiding_create_vault_preserves_other_items_and_can_be_reversed_for_another_profile() {
        let menu = gio::Menu::new();
        for action in [
            "sync-configure",
            "sync-login",
            "sync-create",
            "sync-unlock",
            "sync-passphrase",
        ] {
            menu.append(Some(action), Some(&format!("win.{action}")));
        }
        show_create_vault(&menu, false);
        show_create_vault(&menu, false);
        assert_eq!(menu.n_items(), 4);
        assert_eq!(
            menu.item_attribute_value(2, "action", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.sync-unlock")
        );
        show_create_vault(&menu, true);
        show_create_vault(&menu, true);
        assert_eq!(menu.n_items(), 5);
        assert_eq!(
            menu.item_attribute_value(2, "action", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.sync-create")
        );
        assert_eq!(
            menu.item_attribute_value(3, "action", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.sync-unlock")
        );
    }

    #[test]
    fn changing_an_action_reuses_its_slot_and_unchanged_updates_do_not_rebuild_it() {
        let menu = gio::Menu::new();
        menu.append(Some("Sign In"), Some("win.sync-login"));
        let changes = Rc::new(std::cell::Cell::new(0));
        let recorded = changes.clone();
        menu.connect_items_changed(move |_, _, _, _| recorded.set(recorded.get() + 1));
        replace_menu_action(&menu, 0, "Sign In", "sync-login");
        assert_eq!(changes.get(), 0);
        replace_menu_action(&menu, 0, "Sign Out", "sync-logout");
        assert_eq!(menu.n_items(), 1);
        assert_eq!(
            menu.item_attribute_value(0, "label", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("Sign Out")
        );
        assert_eq!(
            menu.item_attribute_value(0, "action", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.sync-logout")
        );
        let after_change = changes.get();
        assert!(after_change > 0);
        replace_menu_action(&menu, 0, "Sign Out", "sync-logout");
        assert_eq!(changes.get(), after_change);
        replace_menu_action(&menu, 0, "Sign In", "sync-login");
        assert_eq!(menu.n_items(), 1);
        assert_eq!(
            menu.item_attribute_value(0, "action", None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.sync-login")
        );
    }
}
