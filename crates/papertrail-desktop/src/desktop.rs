// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{APP_ID, RESOURCE};
use adw::prelude::*;
use glib::translate::IntoGlib;
use gtk::{gio, glib};
use papertrail_core::{Note, Session, Settings};
use papertrail_storage::startup::{check_notes_folder, prepare_startup_folder};
use papertrail_storage::{Repository, SettingsFile};
use sourceview5::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
};

pub struct Model {
    pub settings: Settings,
    pub session: Session,
    pub all: bool,
}

pub struct Desktop {
    pub sync: papertrail_sync::control::Service,
    pub sync_job:
        RefCell<Option<std::sync::mpsc::Receiver<io::Result<papertrail_sync::control::Outcome>>>>,
    pub sync_unlocked: Cell<bool>,
    pub sync_status: RefCell<String>,
    pub sync_last: Cell<Instant>,
    pub sync_requested: Cell<Option<Instant>>,
    pub sync_retry: Cell<std::time::Duration>,
    pub sync_interactive: Cell<bool>,
    pub sync_pending: RefCell<Option<papertrail_sync::control::Command>>,
    pub sync_error: Cell<bool>,
    pub window: adw::ApplicationWindow,
    pub builder: gtk::Builder,
    pub(crate) sync_config_widgets: RefCell<Option<gtk::Builder>>,
    pub view: sourceview5::View,
    pub buffer: sourceview5::Buffer,
    pub search: sourceview5::SearchContext,
    pub model: RefCell<Model>,
    pub repository: Repository,
    pub settings_file: SettingsFile,
    pub persisted_settings: RefCell<Settings>,
    pub loading: Cell<bool>,
    pub timer: RefCell<Option<glib::SourceId>>,
    pub typography: gtk::CssProvider,
    pub folder_css: gtk::CssProvider,
    pub syncing_preferences: Cell<bool>,
    pub row_views: RefCell<BTreeMap<PathBuf, crate::sidebar::NoteRowView>>,
    pub folder_errors: RefCell<BTreeMap<PathBuf, String>>,
}

impl Desktop {
    pub fn new(app: &adw::Application) -> io::Result<Rc<Self>> {
        let home = glib::home_dir();
        let default = if std::env::var_os("FLATPAK_ID").is_some() {
            glib::user_data_dir().join("notes")
        } else {
            glib::user_special_dir(glib::UserDirectory::Documents)
                .unwrap_or_else(|| home.join("Documents"))
                .join("Paper Trail")
        };
        let settings_file = SettingsFile {
            path: glib::user_config_dir().join("paper-trail/settings.json"),
        };
        let settings = settings_file.load(default.clone(), &home)?;
        Self::with_settings(app, settings_file, settings, &default)
    }

    pub fn with_settings(
        app: &adw::Application,
        settings_file: SettingsFile,
        mut settings: Settings,
        default_notes_dir: &Path,
    ) -> io::Result<Rc<Self>> {
        let persisted_settings = settings.clone();
        let recovery = prepare_startup_folder(&mut settings, default_notes_dir)?;
        let mut folder_errors = BTreeMap::new();
        if let Some(recovery) = &recovery {
            folder_errors.insert(recovery.unavailable.clone(), recovery.reason.clone());
        }
        let ui = gio::resources_lookup_data(
            &format!("{RESOURCE}/window.ui"),
            gio::ResourceLookupFlags::NONE,
        )
        .expect("Embedded UI");
        let ui = std::str::from_utf8(ui.as_ref())
            .expect("UTF-8 UI")
            .replace(
                "<template class=\"PaperTrailWindow\" parent=\"AdwApplicationWindow\">",
                "<object class=\"AdwApplicationWindow\" id=\"window\">",
            )
            .replace("</template>", "</object>");
        let builder = gtk::Builder::from_string(&ui);
        let window: adw::ApplicationWindow = builder.object("window").expect("Main window");
        window.set_application(Some(app));
        window.set_icon_name(Some(APP_ID));
        window.add_css_class("papertrail-window");
        window.set_default_size(settings.width, settings.height);
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_matching_brackets(true);
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_tab_width(4);
        view.set_insert_spaces_instead_of_tabs(true);
        view.set_left_margin(8);
        view.set_smart_home_end(sourceview5::SmartHomeEndType::Always);
        view.add_css_class("editor-view");
        view.set_hexpand(true);
        view.set_vexpand(true);
        builder
            .object::<gtk::ScrolledWindow>("editor_page")
            .unwrap()
            .set_child(Some(&view));
        builder
            .object::<gtk::Box>("sidebar_panel")
            .unwrap()
            .set_size_request(340, -1);
        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_case_sensitive(false);
        let search = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        let desktop = Rc::new(Self {
            sync: papertrail_sync::control::Service::new(&settings_file.path),
            sync_job: RefCell::new(None),
            sync_unlocked: Cell::new(false),
            sync_status: RefCell::new(
                "Local-only. Configure Sync to enable encrypted synchronization.".into(),
            ),
            sync_last: Cell::new(Instant::now()),
            sync_requested: Cell::new(None),
            sync_retry: Cell::new(std::time::Duration::from_secs(30)),
            sync_interactive: Cell::new(false),
            sync_pending: RefCell::new(None),
            sync_error: Cell::new(false),
            window,
            builder,
            sync_config_widgets: RefCell::new(None),
            view,
            buffer,
            search,
            model: RefCell::new(Model {
                settings,
                session: Session::default(),
                all: false,
            }),
            repository: Repository,
            settings_file,
            persisted_settings: RefCell::new(persisted_settings),
            loading: Cell::new(false),
            timer: RefCell::default(),
            typography: gtk::CssProvider::new(),
            folder_css: gtk::CssProvider::new(),
            syncing_preferences: Cell::new(false),
            row_views: RefCell::default(),
            folder_errors: RefCell::new(folder_errors),
        });
        desktop.install_style();
        desktop.setup_actions(app);
        desktop.setup_sync();
        desktop.setup_editor();
        desktop.setup_sidebar();
        desktop.apply_preferences();
        desktop.refresh()?;
        desktop.show_session();
        let weak = Rc::downgrade(&desktop);
        desktop.window.connect_close_request(move |_| {
            let Some(d) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if let Err(e) = d.flush() {
                d.error(&e);
                return glib::Propagation::Stop;
            }
            {
                let mut m = d.model.borrow_mut();
                m.settings.width = d.window.width().clamp(480, 7680);
                m.settings.height = d.window.height().clamp(320, 4320);
            }
            if let Err(e) = d.persist() {
                d.error(&e);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        if let Some(recovery) = recovery {
            let weak = Rc::downgrade(&desktop);
            glib::idle_add_local_once(move || {
                if let Some(desktop) = weak.upgrade() {
                    desktop.show_folder_recovery(&recovery);
                }
            });
        }
        Ok(desktop)
    }

    pub fn widget<T: IsA<glib::Object>>(&self, id: &str) -> T {
        self.builder
            .object(id)
            .or_else(|| {
                self.sync_config_widgets
                    .borrow()
                    .as_ref()
                    .and_then(|builder| builder.object(id))
            })
            .unwrap_or_else(|| panic!("Missing widget: {id}"))
    }
    pub fn error(&self, error: &dyn std::fmt::Display) {
        let dialog = adw::AlertDialog::new(
            Some("Could Not Complete Operation"),
            Some(&error.to_string()),
        );
        dialog.add_response("ok", "OK");
        dialog.set_close_response("ok");
        dialog.present(Some(&self.window));
    }
    pub fn persist(&self) -> io::Result<()> {
        let merged = self.settings_file.save_merged(
            &self.model.borrow().settings,
            &self.persisted_settings.borrow(),
            &glib::home_dir(),
        )?;
        self.model.borrow_mut().settings = merged.clone();
        *self.persisted_settings.borrow_mut() = merged;
        Ok(())
    }
    pub fn flush(&self) -> io::Result<()> {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        let _local = self.sync.local()?;
        let dirty = self.model.borrow().session.is_dirty();
        self.model.borrow_mut().session.flush(&self.repository)?;
        if dirty {
            self.schedule_sync();
        }
        Ok(())
    }
    pub fn close_note(self: &Rc<Self>) -> io::Result<()> {
        self.flush()?;
        {
            let mut local = self.sync.local()?;
            self.model.borrow_mut().session.close(&self.repository)?;
            local.protected.clear();
        }
        self.schedule_sync();
        self.refresh()?;
        self.show_session();
        Ok(())
    }
    pub fn open(self: &Rc<Self>, path: &Path) -> io::Result<()> {
        self.flush()?;
        {
            let mut local = self.sync.local()?;
            let mut model = self.model.borrow_mut();
            model.session.open(&self.repository, path)?;
            local.protected = vec![path.to_owned()];
            if path.parent() != Some(model.settings.notes_dir.as_path()) || model.all {
                model
                    .settings
                    .add_folder(path.parent().unwrap().to_owned(), true);
                model.all = false;
            }
        }
        self.persist()?;
        self.refresh()?;
        self.show_session();
        Ok(())
    }
    pub fn new_note(self: &Rc<Self>) -> io::Result<()> {
        self.flush()?;
        let folder = self.model.borrow().settings.notes_dir.clone();
        check_notes_folder(&folder)?;
        let note = {
            let _local = self.sync.local()?;
            self.repository.create(&folder).map_err(|error| {
                papertrail_storage::startup::folder_error(
                    "Could not create a note in folder",
                    &folder,
                    error,
                )
            })?
        };
        self.open(&note.path)
    }
    pub fn choose_active(self: &Rc<Self>, folder: Option<&Path>) -> io::Result<()> {
        self.flush()?;
        if let Some(folder) = folder {
            check_notes_folder(folder)?;
        }
        {
            let mut local = self.sync.local()?;
            let mut model = self.model.borrow_mut();
            model.session.close(&self.repository)?;
            local.protected.clear();
            model.all = folder.is_none();
            if let Some(folder) = folder {
                model.settings.add_folder(folder.to_owned(), true);
            }
        }
        self.schedule_sync();
        self.persist()?;
        self.refresh()?;
        self.show_session();
        Ok(())
    }
    pub fn rename_note(self: &Rc<Self>, old: &Path, name: &str) -> io::Result<()> {
        self.flush()?;
        let note = {
            let mut local = self.sync.local()?;
            let note = self.repository.rename(old, name)?;
            self.sync.relocate(old, &note.path)?;
            for path in &mut local.protected {
                if path == old {
                    *path = note.path.clone();
                }
            }
            note
        };
        self.relocate(old, note)?;
        self.refresh()?;
        self.show_session();
        Ok(())
    }
    pub fn move_note(self: &Rc<Self>, old: &Path, folder: &Path) -> io::Result<()> {
        self.flush()?;
        let note = {
            let mut local = self.sync.local()?;
            let note = self.repository.move_note(old, folder)?;
            self.sync.relocate(old, &note.path)?;
            for path in &mut local.protected {
                if path == old {
                    *path = note.path.clone();
                }
            }
            note
        };
        self.relocate(old, note)?;
        self.refresh()?;
        self.show_session();
        Ok(())
    }
    fn relocate(&self, old: &Path, note: Note) -> io::Result<()> {
        {
            let mut m = self.model.borrow_mut();
            m.settings.rename_note(old, &note.path);
            if m.session.current.as_ref().is_some_and(|n| n.path == old) {
                m.settings
                    .add_folder(note.path.parent().unwrap().to_owned(), true);
                m.session.current = Some(note);
            }
        }
        self.schedule_sync();
        self.persist()
    }
    pub fn remove_note(self: &Rc<Self>, path: &Path) -> io::Result<()> {
        self.flush()?;
        {
            let mut local = self.sync.local()?;
            self.repository.delete(path)?;
            let mut m = self.model.borrow_mut();
            m.session.discard_deleted(path);
            m.settings.delete_note(path);
            local.protected.retain(|p| p != path);
        }
        self.schedule_sync();
        self.persist()?;
        self.refresh()?;
        if self.model.borrow().session.current.is_none() {
            if let Some(note) = self.visible_notes()?.first() {
                return self.open(&note.path);
            }
        }
        self.show_session();
        Ok(())
    }
    pub fn show_session(&self) {
        let model = self.model.borrow();
        self.loading.set(true);
        if let Some(note) = &model.session.current {
            // Loading a different document must not retain its predecessor's undo history.
            let current_text =
                self.buffer
                    .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true);
            if current_text.as_str() != model.session.text {
                self.buffer.begin_irreversible_action();
                self.buffer.set_text(&model.session.text);
                self.buffer.end_irreversible_action();
            }
            self.widget::<gtk::Stack>("editor_stack")
                .set_visible_child(&self.widget::<gtk::ScrolledWindow>("editor_page"));
            self.widget::<gtk::Label>("editor_title_label")
                .set_label(&note.title);
            self.widget::<gtk::Label>("editor_filename_label")
                .set_label(&note.path.file_name().unwrap_or_default().to_string_lossy());
            self.apply_language(&model.settings, note);
            for (id, text) in [
                ("info_title_row", note.title.clone()),
                (
                    "info_name_row",
                    note.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    "info_location_row",
                    note.path.parent().unwrap().display().to_string(),
                ),
                ("info_type_row", self.document_type()),
            ] {
                self.widget::<adw::ActionRow>(id).set_subtitle(&text);
            }
        } else {
            self.buffer.set_text("");
            self.widget::<gtk::Stack>("editor_stack")
                .set_visible_child(&self.widget::<gtk::ScrolledWindow>("empty_page"));
            self.widget::<gtk::Revealer>("info_revealer")
                .set_reveal_child(false);
            self.widget::<gtk::Revealer>("info_revealer")
                .set_visible(false);
            self.close_editor_search();
            self.widget::<gtk::ToggleButton>("info_button")
                .set_active(false);
            let empty = self
                .widget::<gtk::Stack>("empty_state_stack")
                .visible_child()
                == Some(self.widget::<gtk::Widget>("empty_folder_page"));
            self.widget::<gtk::Label>("editor_title_label")
                .set_label(if empty { "Get started" } else { "Recent Notes" });
            self.widget::<gtk::Label>("editor_filename_label")
                .set_label("");
        }
        let enabled = model.session.current.is_some();
        for name in [
            "close-note",
            "delete-note",
            "print-note",
            "toggle-info",
            "toggle-editor-search",
            "focus-replace",
            "find-next",
            "find-previous",
            "replace-current",
            "replace-all",
        ] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(enabled);
            }
        }
        self.widget::<gtk::ToggleButton>("info_button")
            .set_sensitive(enabled);
        self.widget::<gtk::MenuButton>("language_popout_button")
            .set_sensitive(enabled);
        self.widget::<gtk::Button>("info_delete_button")
            .set_sensitive(enabled);
        self.rebuild_languages();
        self.loading.set(false);
        self.view.grab_focus();
    }

    pub fn edit_changed(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let text = self
            .buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true)
            .to_string();
        {
            let mut model = self.model.borrow_mut();
            model.session.edit(text.clone(), Instant::now());
        }
        if self.model.borrow().session.current.is_none() {
            return;
        }
        let title: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(120)
            .collect();
        let title = if title.is_empty() {
            "Untitled Note"
        } else {
            &title
        };
        self.widget::<gtk::Label>("editor_title_label")
            .set_label(title);
        self.widget::<adw::ActionRow>("info_title_row")
            .set_subtitle(title);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Session::SAVE_DELAY,
            move || {
                if let Some(d) = weak.upgrade() {
                    d.timer.borrow_mut().take();
                    let result = d.model.borrow_mut().session.flush(&d.repository);
                    if let Err(e) = result {
                        d.error(&e);
                    } else if let Err(e) = d.refresh_saved_note() {
                        d.error(&e);
                    }
                }
            },
        ));
    }

    fn install_style(&self) {
        let display = gtk::prelude::WidgetExt::display(&self.window);
        let css = gtk::CssProvider::new();
        css.load_from_resource(&format!("{RESOURCE}/style.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &self.typography,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &self.folder_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        gtk::IconTheme::for_display(&display).add_resource_path(&format!("{RESOURCE}/icons"));
    }
    pub fn apply_preferences(&self) {
        let model = self.model.borrow();
        let s = &model.settings;
        let manager = adw::StyleManager::default();
        manager.set_color_scheme(match s.theme_mode.as_str() {
            "light" => adw::ColorScheme::ForceLight,
            "dark" => adw::ColorScheme::ForceDark,
            _ => adw::ColorScheme::Default,
        });
        self.view.set_show_line_numbers(s.show_line_numbers);
        self.view.set_wrap_mode(if s.wrap_text {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        });
        self.view.set_monospace(s.use_monospace);
        self.view
            .set_highlight_current_line(s.highlight_current_line);
        // This preference highlights the cursor's row, not the right margin.
        self.view.set_show_right_margin(false);
        for (name, value) in [
            ("toggle-line-numbers", s.show_line_numbers),
            ("toggle-wrap", s.wrap_text),
            ("toggle-monospace", s.use_monospace),
        ] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_state(&value.to_variant());
            }
        }
        if let Some(button) = self.builder.object::<gtk::Button>("zoom_reset_button") {
            button.set_label(&format!("{}%", (s.font_scale * 100.0).round()));
        }
        let declaration = if s.use_custom_font {
            let desc = gtk::pango::FontDescription::from_string(&s.custom_font);
            let family = desc
                .family()
                .unwrap_or_else(|| "Monospace".into())
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            format!(
                "font-family: '{family}'; font-style: {}; font-weight: {}; font-size: {:.2}{};",
                match desc.style() {
                    gtk::pango::Style::Italic => "italic",
                    gtk::pango::Style::Oblique => "oblique",
                    _ => "normal",
                },
                desc.weight().into_glib(),
                (if desc.size() > 0 {
                    desc.size()
                } else {
                    11 * gtk::pango::SCALE
                }) as f64
                    / gtk::pango::SCALE as f64
                    * s.font_scale,
                if desc.is_size_absolute() { "px" } else { "pt" }
            )
        } else {
            format!("font-size: {}%;", (s.font_scale * 100.0).round())
        };
        self.typography.load_from_string(&format!(
            ".editor-view, .editor-view text, .recent-note-preview {{ {declaration} }}"
        ));
        let scheme = sourceview5::StyleSchemeManager::default().scheme(if manager.is_dark() {
            "Adwaita-dark"
        } else {
            "Adwaita"
        });
        self.buffer.set_style_scheme(scheme.as_ref());
    }
    pub fn update_settings(&self, update: impl FnOnce(&mut Settings)) -> io::Result<()> {
        update(&mut self.model.borrow_mut().settings);
        self.persist()?;
        self.apply_preferences();
        self.sync_preferences();
        Ok(())
    }
    pub fn current_path(&self) -> Option<PathBuf> {
        self.model
            .borrow()
            .session
            .current
            .as_ref()
            .map(|n| n.path.clone())
    }
}
