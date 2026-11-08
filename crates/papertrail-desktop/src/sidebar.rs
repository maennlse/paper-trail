// SPDX-License-Identifier: GPL-3.0-or-later
use crate::desktop::Desktop;
use adw::prelude::*;
use gtk::{gdk, glib};
use papertrail_core::{colors, note::sort_notes, Note, Store};
use std::{
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

pub struct NoteRowView {
    title: gtk::Label,
    modified: gtk::Label,
}

fn modified_text(note: &Note) -> String {
    let time: chrono::DateTime<chrono::Local> = note.modified.into();
    let now = chrono::Local::now();
    let format = if time.date_naive() == now.date_naive() {
        "%H:%M"
    } else if time.format("%Y").to_string() == now.format("%Y").to_string() {
        "%b %d"
    } else {
        "%Y-%m-%d"
    };
    time.format(format).to_string()
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    for class in classes {
        label.add_css_class(class);
    }
    label
}
pub fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

impl Desktop {
    fn folder_notes(&self, folder: &Path) -> io::Result<Vec<Note>> {
        match self.repository.list(folder) {
            Ok(notes) => {
                self.folder_errors.borrow_mut().remove(folder);
                Ok(notes)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::PermissionDenied
                        | io::ErrorKind::NotADirectory
                ) =>
            {
                self.folder_errors
                    .borrow_mut()
                    .insert(folder.to_owned(), error.to_string());
                Ok(vec![])
            }
            Err(error) => Err(papertrail_storage::startup::folder_error(
                "Could not read notes folder",
                folder,
                error,
            )),
        }
    }
    pub fn setup_sidebar(self: &Rc<Self>) {
        for id in [
            "sidebar_add_folder_button",
            "sidebar_inline_add_folder_button",
        ] {
            let weak = Rc::downgrade(self);
            self.widget::<gtk::Button>(id).connect_clicked(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.choose_folder();
                }
            });
        }
        self.widget::<gtk::Button>("sidebar_search_button")
            .set_action_name(Some("win.focus-sidebar-search"));
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("sidebar_search_entry")
            .connect_search_changed(move |_| {
                if let Some(d) = weak.upgrade() {
                    if let Err(e) = d.refresh() {
                        d.error(&e);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("sidebar_search_entry")
            .connect_stop_search(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.close_search();
                }
            });
        for id in ["sidebar_folders_header_spacer", "sidebar_header_wrap"] {
            // Native titlebar dragging excludes interactive descendants, unlike
            // beginning a compositor move on every press above the search button.
            let header = self.widget::<gtk::Widget>(id);
            let parent = header.parent().unwrap().downcast::<gtk::Box>().unwrap();
            parent.remove(&header);
            let handle = gtk::WindowHandle::new();
            handle.set_child(Some(&header));
            parent.prepend(&handle);
        }
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ScrolledWindow>("sidebar_folders_scroller")
            .vadjustment()
            .connect_changed(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.sync_add_button();
                }
            });
    }
    fn sync_add_button(&self) {
        let fits = self.widget::<gtk::Box>("folder_list").height() + 44
            <= self
                .widget::<gtk::Viewport>("sidebar_folders_viewport")
                .height();
        self.widget::<gtk::Box>("sidebar_inline_add_box")
            .set_visible(fits);
        self.widget::<gtk::Box>("sidebar_folder_add_box")
            .set_visible(!fits);
    }
    pub fn visible_notes(&self) -> io::Result<Vec<Note>> {
        let m = self.model.borrow();
        let query = self
            .widget::<gtk::SearchEntry>("sidebar_search_entry")
            .text()
            .trim()
            .to_owned();
        let folders = if m.all || !query.is_empty() {
            m.settings.note_folders.clone()
        } else {
            vec![m.settings.notes_dir.clone()]
        };
        let mut notes = vec![];
        for folder in folders {
            let listed = self.folder_notes(&folder)?;
            for note in listed {
                if query.is_empty() {
                    notes.push(note);
                } else {
                    let note = self.repository.load(&note.path)?;
                    if note.matches(&query) {
                        notes.push(note);
                    }
                }
            }
        }
        sort_notes(&mut notes, &m.settings.pinned_notes);
        Ok(notes)
    }
    pub fn refresh(self: &Rc<Self>) -> io::Result<()> {
        let notes = self.visible_notes()?;
        let folders = self.widget::<gtk::Box>("folder_list");
        clear(&folders);
        let settings = self.model.borrow().settings.clone();
        let all = self.model.borrow().all;
        let mut css = String::new();
        for color in colors::PRESETS
            .iter()
            .copied()
            .chain(settings.folder_colors.values().map(String::as_str))
        {
            css.push_str(&format!(
                ".{} {{ color: {}; }}\n",
                colors::css_class(color),
                colors::hex(color)
            ));
        }
        self.folder_css.load_from_string(&css);
        folders.append(&self.folder_row(None, "preset-11", all));
        for folder in &settings.note_folders {
            let token = settings
                .folder_colors
                .get(folder)
                .map(String::as_str)
                .unwrap_or("preset-0");
            folders.append(&self.folder_row(
                Some(folder),
                token,
                !all && *folder == settings.notes_dir,
            ));
        }
        let list = self.widget::<gtk::Box>("note_list");
        clear(&list);
        self.row_views.borrow_mut().clear();
        for note in &notes {
            list.append(&self.note_row(note));
        }
        let title = if all {
            "All".into()
        } else {
            settings.folder_name(&settings.notes_dir)
        };
        self.widget::<gtk::Label>("sidebar_title_label")
            .set_label(&title);
        let query = self
            .widget::<gtk::SearchEntry>("sidebar_search_entry")
            .text();
        self.widget::<gtk::Label>("sidebar_subtitle_label")
            .set_label(&format!(
                "{} {}",
                notes.len(),
                if query.is_empty() {
                    if notes.len() == 1 {
                        "note"
                    } else {
                        "notes"
                    }
                } else if notes.len() == 1 {
                    "match"
                } else {
                    "matches"
                }
            ));
        // Recent notes follow the folder/all scope rather than the current search query.
        let mut recent = vec![];
        for folder in if all {
            settings.note_folders.clone()
        } else {
            vec![settings.notes_dir.clone()]
        } {
            recent.extend(self.folder_notes(&folder)?);
        }
        sort_notes(&mut recent, &settings.pinned_notes);
        let grid: gtk::FlowBox = self.widget("recent_notes_grid");
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        for note in recent.iter().take(9) {
            grid.insert(&self.recent_tile(note), -1);
        }
        let empty = recent.is_empty();
        self.widget::<gtk::Stack>("empty_state_stack")
            .set_visible_child(&self.widget::<gtk::Widget>(if empty {
                "empty_folder_page"
            } else {
                "recent_notes_page"
            }));
        if self.current_path().is_none() {
            self.widget::<gtk::Label>("editor_title_label")
                .set_label(if empty { "Get started" } else { "Recent Notes" });
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(d) = weak.upgrade() {
                d.sync_add_button();
            }
        });
        self.refresh_preferences_folders();
        Ok(())
    }
    fn note_row(self: &Rc<Self>, note: &Note) -> gtk::Box {
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.add_css_class("note-row");
        let inset = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inset.add_css_class("note-row-outer");
        inset.set_margin_start(18);
        inset.set_margin_end(18);
        inset.set_margin_top(2);
        inset.set_margin_bottom(2);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("note-row-card");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.add_css_class("note-row-card-content");
        if self.current_path().as_ref() == Some(&note.path) {
            card.add_css_class("active");
        }
        let settings = self.model.borrow().settings.clone();
        if self.model.borrow().all
            || !self
                .widget::<gtk::SearchEntry>("sidebar_search_entry")
                .text()
                .is_empty()
        {
            card.add_css_class("folder-colored");
        }
        card.add_css_class(&colors::css_class(
            settings
                .folder_colors
                .get(note.path.parent().unwrap())
                .map(String::as_str)
                .unwrap_or("preset-0"),
        ));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = label(&note.title, &["heading", "note-row-title"]);
        header.append(&title);
        if settings.pinned_notes.contains(&note.path) {
            let icon = gtk::Image::from_icon_name("view-pin-symbolic");
            icon.add_css_class("note-row-pin");
            header.append(&icon);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        let modified = label(&modified_text(note), &["caption", "dim-label"]);
        header.append(&modified);
        self.row_views
            .borrow_mut()
            .insert(note.path.clone(), NoteRowView { title, modified });
        body.append(&header);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let filename = label(
            &note.path.file_name().unwrap_or_default().to_string_lossy(),
            &["caption", "dim-label", "note-row-filename"],
        );
        filename.set_hexpand(true);
        row.append(&filename);
        row.append(&label(
            &self.language_label(note),
            &["caption", "note-row-language"],
        ));
        body.append(&row);
        card.append(&body);
        inset.append(&card);
        outer.append(&inset);
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let path = note.path.clone();
        let weak = Rc::downgrade(self);
        gesture.connect_released(move |_, _, _, _| {
            if let Some(d) = weak.upgrade() {
                if d.current_path().as_ref() != Some(&path) {
                    if let Err(e) = d.open(&path) {
                        d.error(&e);
                    }
                }
            }
        });
        inset.add_controller(gesture);
        let menu = gtk::Popover::new();
        menu.add_css_class("papertrail-popover");
        menu.add_css_class("note-row-menu-popover");
        menu.set_parent(&inset);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let pinned = settings.pinned_notes.contains(&note.path);
        let path = note.path.clone();
        content.append(
            &self.menu_button(if pinned { "Unpin" } else { "Pin to Top" }, move |d| {
                d.update_settings(|s| s.set_pinned(&path, !pinned))?;
                d.refresh()
            }),
        );
        let path = note.path.clone();
        let anchor = inset.downgrade();
        content.append(&self.menu_button("Rename", move |d| {
            if let Some(anchor) = anchor.upgrade() {
                d.rename_popover(path.clone(), &anchor);
            }
            Ok(())
        }));
        let path = note.path.clone();
        content.append(&self.menu_button("Open Folder", move |d| {
            let launcher = gtk::FileLauncher::new(Some(&gio_file(&path)));
            let weak = Rc::downgrade(d);
            launcher.open_containing_folder(
                Some(&d.window),
                None::<&gtk::gio::Cancellable>,
                move |result| {
                    if let (Err(e), Some(d)) = (result, weak.upgrade()) {
                        d.error(&e);
                    }
                },
            );
            Ok(())
        }));
        let move_button = gtk::MenuButton::builder()
            .label("Move to Folder")
            .has_frame(false)
            .build();
        let targets = gtk::Popover::new();
        targets.add_css_class("papertrail-popover");
        let target_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for folder in settings
            .note_folders
            .iter()
            .filter(|p| Some(p.as_path()) != note.path.parent())
        {
            let path = note.path.clone();
            let folder = folder.clone();
            let title = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            target_box.append(&self.menu_button(&title, move |d| d.move_note(&path, &folder)));
        }
        move_button.set_sensitive(target_box.first_child().is_some());
        targets.set_child(Some(&target_box));
        move_button.set_popover(Some(&targets));
        content.append(&move_button);
        let path = note.path.clone();
        let delete = self.menu_button("Delete", move |d| {
            d.confirm_delete(path.clone());
            Ok(())
        });
        delete.add_css_class("destructive-action");
        content.append(&delete);
        menu.set_child(Some(&content));
        let secondary = gtk::GestureClick::new();
        secondary.set_button(3);
        let weak_menu = menu.downgrade();
        secondary.connect_pressed(move |_, _, x, y| {
            if let Some(menu) = weak_menu.upgrade() {
                menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                menu.popup();
            }
        });
        inset.add_controller(secondary);
        let weak_menu = menu.downgrade();
        inset.connect_destroy(move |_| {
            if let Some(menu) = weak_menu.upgrade() {
                menu.unparent();
            }
        });
        outer
    }
    pub fn menu_button(
        self: &Rc<Self>,
        title: &str,
        callback: impl Fn(&Rc<Self>) -> io::Result<()> + 'static,
    ) -> gtk::Button {
        let button = gtk::Button::new();
        button.set_child(Some(&label(title, &[])));
        button.add_css_class("flat");
        button.add_css_class("modelbutton");
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |b| {
            if let Some(popover) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
            }
            if let Some(d) = weak.upgrade() {
                if let Err(e) = callback(&d) {
                    d.error(&e);
                }
            }
        });
        button
    }
    pub fn refresh_saved_note(self: &Rc<Self>) -> io::Result<()> {
        if !self
            .widget::<gtk::SearchEntry>("sidebar_search_entry")
            .text()
            .is_empty()
        {
            return self.refresh();
        }
        if let Some(note) = &self.model.borrow().session.current {
            if let Some(row) = self.row_views.borrow().get(&note.path) {
                row.title.set_label(&note.title);
                row.modified.set_label(&modified_text(note));
            }
            self.widget::<gtk::Label>("editor_title_label")
                .set_label(&note.title);
            self.widget::<adw::ActionRow>("info_title_row")
                .set_subtitle(&note.title);
        }
        Ok(())
    }
    fn folder_row(
        self: &Rc<Self>,
        folder: Option<&Path>,
        token: &str,
        active: bool,
    ) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.add_css_class("folder-row");
        button.add_css_class(&colors::css_class(token));
        if active {
            button.add_css_class("active");
        }
        let title = folder
            .map(|p| self.model.borrow().settings.folder_name(p))
            .unwrap_or_else(|| "All".into());
        button.set_tooltip_text(Some(
            &folder
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Show notes from all folders".into()),
        ));
        if let Some(error) = folder.and_then(|path| self.folder_errors.borrow().get(path).cloned())
        {
            button.set_tooltip_text(Some(&format!(
                "{}\n\nFolder unavailable: {error}\nChoose the folder again to restore access.",
                folder.unwrap().display()
            )));
            button.set_opacity(0.55);
        }
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("folder-row-card");
        let overlay = gtk::Overlay::new();
        let icon = gtk::Image::from_icon_name("folder-symbolic");
        icon.set_pixel_size(34);
        overlay.set_child(Some(&icon));
        let initials = label(
            &if folder.is_some() {
                colors::badge(&title)
            } else {
                "All".into()
            },
            &["folder-row-initials"],
        );
        initials.set_ellipsize(gtk::pango::EllipsizeMode::None);
        initials.set_halign(gtk::Align::Center);
        initials.set_valign(gtk::Align::Center);
        overlay.add_overlay(&initials);
        let indicator = gtk::Box::new(gtk::Orientation::Vertical, 0);
        indicator.add_css_class("folder-row-active-indicator");
        indicator.set_halign(gtk::Align::Start);
        indicator.set_valign(gtk::Align::Center);
        overlay.add_overlay(&indicator);
        card.append(&overlay);
        button.set_child(Some(&card));
        let path = folder.map(Path::to_owned);
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                if let Err(e) = d.choose_active(path.as_deref()) {
                    d.error(&e);
                }
            }
        });
        if let Some(folder) = folder {
            let menu = gtk::Popover::new();
            menu.add_css_class("papertrail-popover");
            menu.set_position(gtk::PositionType::Right);
            menu.set_parent(&button);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let path = folder.to_owned();
            content.append(&self.menu_button("Edit Folder", move |d| {
                d.edit_folder(path.clone());
                Ok(())
            }));
            let path = folder.to_owned();
            let close = self.menu_button("Close Folder", move |d| d.close_folder(&path));
            close.set_sensitive(self.model.borrow().settings.note_folders.len() > 1);
            content.append(&close);
            menu.set_child(Some(&content));
            let secondary = gtk::GestureClick::new();
            secondary.set_button(3);
            let weak_menu = menu.downgrade();
            secondary.connect_released(move |_, _, _, _| {
                if let Some(m) = weak_menu.upgrade() {
                    m.popup();
                }
            });
            button.add_controller(secondary);
            let weak_menu = menu.downgrade();
            button.connect_destroy(move |_| {
                if let Some(m) = weak_menu.upgrade() {
                    m.unparent();
                }
            });
            let drag = gtk::DragSource::new();
            drag.set_actions(gdk::DragAction::MOVE);
            let path = folder.to_string_lossy().into_owned();
            drag.connect_prepare(move |_, _, _| {
                Some(gdk::ContentProvider::for_value(&path.to_value()))
            });
            button.add_controller(drag);
            let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
            let path = folder.to_owned();
            let weak = Rc::downgrade(self);
            let weak_button = button.downgrade();
            drop.connect_drop(move |_, value, _, y| {
                let (Some(d), Some(button), Ok(source)) =
                    (weak.upgrade(), weak_button.upgrade(), value.get::<String>())
                else {
                    return false;
                };
                let mut folders = d.model.borrow().settings.note_folders.clone();
                let source = PathBuf::from(source);
                if source == path || !folders.contains(&source) {
                    return false;
                }
                folders.retain(|p| p != &source);
                let Some(index) = folders.iter().position(|p| p == &path) else {
                    return false;
                };
                folders.insert(
                    index + usize::from(y >= button.height() as f64 / 2.0),
                    source,
                );
                d.model.borrow_mut().settings.reorder(folders);
                if let Err(e) = d.persist().and_then(|_| d.refresh()) {
                    d.error(&e);
                }
                true
            });
            button.add_controller(drop);
        }
        button
    }
    fn recent_tile(self: &Rc<Self>, note: &Note) -> gtk::Button {
        let tile = gtk::Button::new();
        tile.add_css_class("flat");
        tile.add_css_class("recent-note-tile");
        tile.set_size_request(-1, 210);
        tile.set_hexpand(true);
        tile.set_vexpand(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.append(&label(&note.title, &["title-4", "recent-note-title"]));
        content.append(&label(
            &note.path.file_name().unwrap_or_default().to_string_lossy(),
            &["caption", "dim-label"],
        ));
        let preview = gtk::TextView::new();
        preview.buffer().set_text(&note.preview);
        preview.set_editable(false);
        preview.set_cursor_visible(false);
        preview.set_can_focus(false);
        preview.set_vexpand(true);
        preview.add_css_class("recent-note-preview");
        content.append(&preview);
        let date: chrono::DateTime<chrono::Local> = note.modified.into();
        content.append(&label(
            &date.format("%b %d, %H:%M").to_string(),
            &["caption", "dim-label"],
        ));
        tile.set_child(Some(&content));
        let path = note.path.clone();
        let weak = Rc::downgrade(self);
        tile.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                if let Err(e) = d.open(&path) {
                    d.error(&e);
                }
            }
        });
        tile
    }
    pub fn close_folder(self: &Rc<Self>, path: &Path) -> io::Result<()> {
        if self.model.borrow().settings.note_folders.len() <= 1 {
            return Ok(());
        }
        self.flush()?;
        let mut settings = self.model.borrow().settings.clone();
        settings.remove_folder(path);
        if papertrail_storage::startup::check_notes_folder(&settings.notes_dir).is_err() {
            let next = settings.note_folders.iter().find(|folder| papertrail_storage::startup::check_notes_folder(folder).is_ok())
                .cloned().ok_or_else(|| io::Error::other("Cannot close the only available notes folder. Choose another accessible folder first."))?;
            settings.notes_dir = next;
        }
        let next = settings.notes_dir.clone();
        self.model.borrow_mut().settings = settings;
        self.choose_active(Some(&next))
    }
}
fn gio_file(path: &Path) -> gtk::gio::File {
    gtk::gio::File::for_path(path)
}
