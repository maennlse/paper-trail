//! Run serially on a display: xvfb-run -a dbus-run-session -- cargo test -p papertrail-desktop.
use crate::{desktop::Desktop, APP_ID};
use adw::prelude::*;
use gtk::{gio, glib};
use papertrail_core::Settings;
use papertrail_storage::SettingsFile;
use sourceview5::prelude::*;
use std::{
    fs,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug)]
struct UiDiagnostic {
    level: glib::LogLevel,
    domain: String,
    message: String,
}
type UiDiagnostics = Arc<Mutex<Vec<UiDiagnostic>>>;

fn record_ui_diagnostics() -> UiDiagnostics {
    let diagnostics = UiDiagnostics::default();
    let recorded = diagnostics.clone();
    // GTK uses structured logging. Always forward to GLib's normal writer: this
    // checks application warnings without hiding renderer/platform diagnostics.
    glib::log_set_writer_func(move |level, fields| {
        let value = |key| {
            fields
                .iter()
                .find(|field| field.key() == key)
                .and_then(|field| field.value_str())
                .unwrap_or("")
        };
        let domain = value("GLIB_DOMAIN");
        if matches!(level, glib::LogLevel::Warning | glib::LogLevel::Critical)
            && matches!(domain, "Gtk" | "Adwaita" | "Gdk")
        {
            if let Ok(mut diagnostics) = recorded.lock() {
                diagnostics.push(UiDiagnostic {
                    level,
                    domain: domain.into(),
                    message: value("MESSAGE").into(),
                });
            }
        }
        glib::log_writer_default(level, fields)
    });
    diagnostics
}

fn assert_no_application_diagnostics(d: &Desktop, diagnostics: &UiDiagnostics) {
    let recorded = diagnostics.lock().unwrap();
    let failures: Vec<_> = recorded
        .iter()
        .filter(|diagnostic| {
            diagnostic.level == glib::LogLevel::Critical
                || (matches!(diagnostic.domain.as_str(), "Gtk" | "Adwaita")
                    && (diagnostic.message.contains("Trying to measure")
                        || diagnostic
                            .message
                            .contains("can only be placed into GtkListBox")))
        })
        .collect();
    for diagnostic in &failures {
        if let Some(pointer) = diagnostic
            .message
            .split_whitespace()
            .find(|word| word.starts_with("0x"))
        {
            if let Some(mut widget) = descendant(d.window.upcast_ref(), &|widget| {
                format!("{:p}", widget.as_ptr()) == pointer
            }) {
                eprintln!(
                    "Diagnostic allocation: {}x{}, margins top/bottom {}/{}, request mode {:?}",
                    widget.width(),
                    widget.height(),
                    widget.margin_top(),
                    widget.margin_bottom(),
                    widget.request_mode()
                );
                let mut child = widget.first_child();
                while let Some(item) = child {
                    eprintln!(
                        "Diagnostic child: {} {}x{}",
                        item.type_().name(),
                        item.width(),
                        item.height()
                    );
                    child = item.next_sibling();
                }
                let mut path = Vec::new();
                loop {
                    path.push(
                        widget
                            .buildable_id()
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| widget.type_().name().into()),
                    );
                    let Some(parent) = widget.parent() else {
                        break;
                    };
                    widget = parent;
                }
                path.reverse();
                eprintln!("Diagnostic widget: {}", path.join("/"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Application GTK diagnostics: {failures:#?}"
    );
}

fn settle() {
    let context = glib::MainContext::default();
    let end = Instant::now() + Duration::from_millis(100);
    while Instant::now() < end {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn current_line_highlight_regression(d: &Rc<Desktop>) {
    for theme in ["light", "dark"] {
        d.update_settings(|s| s.theme_mode = theme.into()).unwrap();
        current_line_highlight_in_current_theme(d);
    }
}

fn startup_folder_recovery_regression(app: &adw::Application, root: &std::path::Path) {
    let portal = root.join("doc/revoked/notes");
    fs::create_dir_all(portal.parent().unwrap()).unwrap();
    let blocked = root.join("not-a-directory");
    fs::write(&blocked, "keep").unwrap();
    let default = root.join("app-data/notes");
    let file = SettingsFile {
        path: root.join("config/settings.json"),
    };
    let mut settings = Settings::new(portal.clone());
    settings.add_folder(blocked.clone(), false);
    settings
        .folder_colors
        .insert(portal.clone(), "#aabbcc".into());
    settings.pinned_notes.push(portal.join("keep.txt"));
    settings
        .note_language_overrides
        .insert(portal.join("keep.txt"), "rust".into());
    file.save(&settings).unwrap();
    let before = fs::read(&file.path).unwrap();
    let loaded = file.load(default.clone(), root).unwrap();
    let d = Desktop::with_settings(app, file, loaded, &default).unwrap();
    assert!(
        !portal.exists(),
        "Startup must not recreate a portal bookmark"
    );
    assert_eq!(d.model.borrow().settings.notes_dir, default);
    assert_eq!(
        fs::read(&d.settings_file.path).unwrap(),
        before,
        "Startup recovery must not discard saved settings"
    );
    d.window.present();
    wait_until(|| d.window.visible_dialog().is_some());
    let alert = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(alert.heading().as_deref(), Some("Notes Folder Unavailable"));
    assert!(alert.body().contains(&portal.display().to_string()));
    assert!(alert.has_response("choose"));
    assert!(alert.has_response("continue"));
    alert.close();
    wait_until(|| !alert.is_mapped());
    d.new_note().unwrap();
    let new_note = d.current_path().unwrap();
    d.buffer.set_text("Recovery needle");
    d.flush().unwrap();
    d.choose_active(None).unwrap();
    assert_eq!(d.visible_notes().unwrap().len(), 1);
    assert!(d.folder_errors.borrow().contains_key(&portal));
    assert!(d.folder_errors.borrow().contains_key(&blocked));
    d.widget::<gtk::SearchEntry>("sidebar_search_entry")
        .set_text("needle");
    assert_eq!(d.visible_notes().unwrap()[0].path, new_note);
    assert!(
        d.close_folder(&default).is_err(),
        "Do not remove the only usable folder"
    );
    assert_eq!(d.model.borrow().settings.notes_dir, default);
    assert!(d.choose_active(Some(&portal)).is_err());
    assert!(!portal.exists());
    d.widget::<gtk::SearchEntry>("sidebar_search_entry")
        .set_text("");
    // Simulate the original folder being re-selected/re-authorized by its user.
    fs::create_dir(&portal).unwrap();
    fs::write(portal.join("keep.txt"), "Restored note").unwrap();
    d.choose_active(Some(&portal)).unwrap();
    assert!(!d.folder_errors.borrow().contains_key(&portal));
    d.open(&portal.join("keep.txt")).unwrap();
    assert_eq!(d.buffer.language().unwrap().id(), "rust");
    assert!(d
        .model
        .borrow()
        .settings
        .pinned_notes
        .contains(&portal.join("keep.txt")));
    assert_eq!(d.model.borrow().settings.folder_colors[&portal], "#aabbcc");
    assert_eq!(fs::read_to_string(&new_note).unwrap(), "Recovery needle");
    d.window.close();
    settle();
    let saved = d.settings_file.load(default, root).unwrap();
    assert!(saved.note_folders.contains(&portal));
    assert!(saved.note_folders.contains(&blocked));
    assert_eq!(fs::read_to_string(blocked).unwrap(), "keep");
}

fn activate_language(d: &Desktop, id: &str) {
    let languages = d.widget::<gtk::ListBox>("language_listbox");
    let row = descendant(languages.upcast_ref(), &|w| {
        w.is::<gtk::ListBoxRow>() && w.widget_name() == id
    })
    .unwrap_or_else(|| panic!("Missing language choice: {id}"));
    languages.emit_by_name::<()>(
        "row-activated",
        &[&row.downcast::<gtk::ListBoxRow>().unwrap()],
    );
}

fn editor_padding_and_filetype_regressions(d: &Rc<Desktop>, folder: &std::path::Path) {
    fs::create_dir_all(folder).unwrap();
    d.choose_active(Some(folder)).unwrap();
    for (filename, expected) in [
        ("note.txt", None),
        ("main.rs", Some("rust")),
        ("script.py", Some("python3")),
        ("data.json", Some("json")),
        ("script.js", Some("js")),
        ("script.sh", Some("sh")),
        ("main.RS", Some("rust")),
        ("script.PY", Some("python3")),
        ("data.JSON", Some("json")),
        ("source.C", Some("cpp")),
        ("unknown.unrecognized-extension", None),
        ("README", None),
    ] {
        let path = folder.join(filename);
        fs::write(&path, "fn main() {}\n").unwrap();
        d.open(&path).unwrap();
        assert_eq!(
            d.buffer.language().map(|l| l.id().to_string()).as_deref(),
            expected,
            "Filetype for {filename}"
        );
        assert_eq!(d.buffer.is_highlight_syntax(), expected.is_some());
        let expected_label = expected.map(|id| {
            sourceview5::LanguageManager::default()
                .language(id)
                .unwrap()
                .name()
                .to_string()
        });
        assert_eq!(
            d.document_type(),
            expected_label.as_deref().unwrap_or("Plain Text")
        );
        assert_eq!(
            d.widget::<adw::ActionRow>("info_type_row")
                .subtitle()
                .as_deref(),
            Some(d.document_type().as_str())
        );
        let note = d.model.borrow().session.current.clone().unwrap();
        assert_eq!(
            d.language_label(&note),
            expected_label.as_deref().unwrap_or("Text")
        );
        assert!(!d
            .model
            .borrow()
            .settings
            .note_language_overrides
            .contains_key(&path));
    }

    // The text inset must survive both gutter modes and preference updates.
    for line_numbers in [true, false] {
        d.update_settings(|s| s.show_line_numbers = line_numbers)
            .unwrap();
        settle();
        assert_eq!(d.view.left_margin(), 8);
        assert_eq!(d.view.shows_line_numbers(), line_numbers);
        if !line_numbers {
            let rect = d.view.iter_location(&d.buffer.start_iter());
            let (x, _) =
                d.view
                    .buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
            assert!(
                x >= 8,
                "The first character must be inset from the editor edge"
            );
        }
    }

    let original = folder.join("rename.txt");
    fs::write(&original, "original").unwrap();
    d.open(&original).unwrap();
    d.buffer.set_text("fn main() {}\n");
    d.rename_note(&original, "renamed.rs").unwrap();
    let rust_path = d.current_path().unwrap();
    assert_eq!(fs::read_to_string(&rust_path).unwrap(), "fn main() {}\n");
    assert_eq!(d.buffer.language().unwrap().id(), "rust");
    assert_eq!(
        d.widget::<gtk::Label>("editor_filename_label").text(),
        "renamed.rs"
    );
    assert_eq!(
        d.widget::<adw::ActionRow>("info_type_row")
            .subtitle()
            .as_deref(),
        Some("Rust")
    );
    d.rename_note(&rust_path, "renamed.py").unwrap();
    let python_path = d.current_path().unwrap();
    assert_eq!(d.buffer.language().unwrap().id(), "python3");

    // A deliberate override wins over the extension, survives rename/reopen,
    // and clearing it immediately restores automatic extension detection.
    activate_language(d, "rust");
    d.rename_note(&python_path, "renamed.json").unwrap();
    let json_path = d.current_path().unwrap();
    assert_eq!(d.buffer.language().unwrap().id(), "rust");
    assert_eq!(
        d.model.borrow().settings.note_language_overrides[&json_path],
        "rust"
    );
    d.close_note().unwrap();
    d.open(&json_path).unwrap();
    assert_eq!(d.buffer.language().unwrap().id(), "rust");
    activate_language(d, "automatic");
    assert_eq!(d.buffer.language().unwrap().id(), "json");
    assert!(!d
        .model
        .borrow()
        .settings
        .note_language_overrides
        .contains_key(&json_path));
    d.rename_note(&json_path, "renamed.unrecognized-extension")
        .unwrap();
    assert!(d.buffer.language().is_none());
    assert!(!d.buffer.is_highlight_syntax());
    assert_eq!(d.document_type(), "Plain Text");
    d.close_note().unwrap();
}

fn current_line_highlight_in_current_theme(d: &Rc<Desktop>) {
    d.new_note().unwrap();
    d.update_settings(|s| {
        s.highlight_current_line = true;
        s.use_monospace = true;
        s.use_custom_font = true;
        s.custom_font = "Monospace 10".into();
        s.font_scale = 1.0;
        s.show_line_numbers = false;
        s.wrap_text = false;
    })
    .unwrap();
    d.buffer
        .set_text("Current-line highlight\nSecond line\nThird line");
    d.buffer.place_cursor(&d.buffer.start_iter());
    d.view.set_cursor_visible(false);
    focus_editor(d);
    assert!(d.view.is_highlight_current_line());
    assert!(!d.view.shows_right_margin());
    let pixels = || {
        settle();
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&d.view)).snapshot(
            &snapshot,
            d.view.width() as f64,
            d.view.height() as f64,
        );
        let texture = d
            .window
            .renderer()
            .unwrap()
            .render_texture(snapshot.to_node().unwrap(), None);
        let stride = texture.width() as usize * 4;
        let mut bytes = vec![0; stride * texture.height() as usize];
        texture.download(&mut bytes, stride);
        if let Some(path) = std::env::var_os("PAPERTRAIL_SCREENSHOT") {
            texture
                .save_to_png(std::path::PathBuf::from(path).with_extension(
                    if d.view.is_highlight_current_line() {
                        "current-line-on.png"
                    } else {
                        "current-line-off.png"
                    },
                ))
                .unwrap();
        }
        bytes
    };
    let shown = pixels();
    d.buffer.place_cursor(&d.buffer.iter_at_line(1).unwrap());
    assert!(
        shown != pixels(),
        "The highlight must follow the cursor to the next row"
    );
    d.buffer.place_cursor(&d.buffer.start_iter());
    assert!(
        shown == pixels(),
        "Moving the cursor back must restore the first-row highlight"
    );
    d.preferences();
    settle();
    let switch = d
        .builder
        .object::<adw::SwitchRow>("preference_switch_2")
        .unwrap();
    assert!(switch.is_active());
    assert_eq!(switch.title(), "Highlight Current Line");
    switch.set_active(false);
    assert!(!d.view.is_highlight_current_line());
    assert!(!d.view.shows_right_margin());
    assert!(!d.model.borrow().settings.highlight_current_line);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&d.settings_file.path).unwrap()).unwrap();
    assert_eq!(saved["highlight_current_line"], false);
    assert!(saved.get("show_ruler").is_none());
    d.builder
        .object::<adw::PreferencesDialog>("preferences_dialog")
        .unwrap()
        .close();
    settle();
    settle();
    settle();
    focus_editor(d);
    let hidden = pixels();
    assert!(
        shown != hidden,
        "Turning highlighting off must change the rendered editor"
    );
    let width = d.view.width() as usize;
    let changed_rows: Vec<_> = shown
        .chunks_exact(4)
        .zip(hidden.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(pixel, _)| pixel / width)
        .collect();
    let changed_height = changed_rows.last().unwrap() - changed_rows.first().unwrap() + 1;
    assert!(
        changed_height <= d.view.iter_location(&d.buffer.start_iter()).height() as usize + 2,
        "Only one horizontal row may change, not a vertical margin or shaded strip"
    );
    d.preferences();
    settle();
    switch.set_active(true);
    assert!(d.view.is_highlight_current_line());
    assert!(!d.view.shows_right_margin());
    d.builder
        .object::<adw::PreferencesDialog>("preferences_dialog")
        .unwrap()
        .close();
    settle();
    settle();
    settle();
    focus_editor(d);
    assert!(
        shown == pixels(),
        "Turning highlighting back on must restore its rendering"
    );
}

fn focus_editor(d: &Desktop) {
    d.window.present();
    if std::env::var_os("PAPERTRAIL_INPUT_TESTS").is_some() {
        let window = x11_window(d);
        assert!(std::process::Command::new("xdotool")
            .args(["windowfocus", &window])
            .status()
            .unwrap()
            .success());
    }
    d.view.grab_focus();
    wait_until(|| d.window.is_active());
}

fn x11_window(d: &Desktop) -> String {
    let title = format!("Paper Trail input test {}", std::process::id());
    d.window.set_title(Some(&title));
    settle();
    let output = std::process::Command::new("xdotool")
        .args([
            "search",
            "--onlyvisible",
            "--all",
            "--pid",
            &std::process::id().to_string(),
            "--name",
            &format!("^{title}$"),
        ])
        .output()
        .expect("Input tests require xdotool and X11");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .next()
        .expect("X11 application window")
        .to_owned()
}

fn descendant(
    widget: &gtk::Widget,
    matches: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = descendant(&widget, matches) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn sidebar_and_shortcuts_regressions(d: &Rc<Desktop>) {
    let search: gtk::Button = d.widget("sidebar_search_button");
    assert_eq!(
        search.action_name().as_deref(),
        Some("win.focus-sidebar-search")
    );
    assert!(d
        .widget::<gtk::Widget>("sidebar_folders_header_spacer")
        .parent()
        .unwrap()
        .is::<gtk::WindowHandle>());
    search.emit_clicked();
    assert!(d
        .widget::<gtk::Revealer>("sidebar_search_revealer")
        .reveals_child());
    search.emit_clicked();
    assert!(!d
        .widget::<gtk::Revealer>("sidebar_search_revealer")
        .reveals_child());

    let dialog = d.shortcuts();
    settle();
    assert!(dialog.child().unwrap().is::<adw::ToolbarView>());
    assert!(dialog.can_close());
    let close = descendant(dialog.upcast_ref(), &|w| {
        w.is::<gtk::Button>() && w.has_css_class("close")
    })
    .expect("Shortcuts dialog must expose a close button");
    close.downcast::<gtk::Button>().unwrap().emit_clicked();
    wait_until(|| d.window.visible_dialog().is_none());
    wait_until(|| !dialog.is_mapped());

    // CI's isolated Xvfb display exercises actual event propagation, not just
    // clicked signals (which cannot detect a parent gesture stealing a press).
    if std::env::var_os("PAPERTRAIL_INPUT_TESTS").is_none() {
        return;
    }
    let window = x11_window(d);
    let input = |args: &[&str]| {
        assert!(std::process::Command::new("xdotool")
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    input(&["windowfocus", &window]);
    settle();
    let click = |widget: &gtk::Widget| {
        let point = widget
            .compute_point(
                &d.window,
                &gtk::graphene::Point::new(
                    widget.width() as f32 / 2.0,
                    widget.height() as f32 / 2.0,
                ),
            )
            .unwrap();
        // X11 client-surface coordinates include the client-side shadow;
        // GtkRoot widget coordinates do not.
        let (offset_x, offset_y) = d.window.surface_transform();
        input(&[
            "mousemove",
            "--window",
            &window,
            &format!("{}", (point.x() as f64 - offset_x) as i32),
            &format!("{}", (point.y() as f64 - offset_y) as i32),
            "click",
            "1",
        ]);
    };
    click(search.upcast_ref());
    wait_until(|| {
        d.widget::<gtk::Revealer>("sidebar_search_revealer")
            .reveals_child()
    });
    let focus = gtk::prelude::RootExt::focus(&d.window).unwrap();
    let entry = d.widget::<gtk::SearchEntry>("sidebar_search_entry");
    assert!(focus == entry || focus.is_ancestor(&entry));
    click(search.upcast_ref());
    wait_until(|| {
        !d.widget::<gtk::Revealer>("sidebar_search_revealer")
            .reveals_child()
    });

    input(&["key", "ctrl+k"]);
    wait_until(|| {
        d.widget::<gtk::Revealer>("sidebar_search_revealer")
            .reveals_child()
    });
    let dialog = d.shortcuts();
    settle();
    settle();
    settle();
    input(&["key", "Escape"]);
    wait_until(|| d.window.visible_dialog().is_none());
    wait_until(|| !dialog.is_mapped());
    assert!(
        d.widget::<gtk::Revealer>("sidebar_search_revealer")
            .reveals_child(),
        "Dialog Escape must not close the underlying search"
    );
    assert!(dialog.parent().is_none());
    let dialog = d.shortcuts();
    settle();
    settle();
    settle();
    let close = descendant(dialog.upcast_ref(), &|w| {
        w.is::<gtk::Button>() && w.has_css_class("close")
    })
    .unwrap();
    click(&close);
    wait_until(|| d.window.visible_dialog().is_none());
    wait_until(|| !dialog.is_mapped());
    input(&["key", "Escape"]);
    wait_until(|| {
        !d.widget::<gtk::Revealer>("sidebar_search_revealer")
            .reveals_child()
    });
    d.window.set_title(Some("Paper Trail"));
}

#[track_caller]
fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "GTK operation timed out");
        settle();
    }
}

fn capture(d: &Desktop, suffix: &str) {
    let Some(path) = std::env::var_os("PAPERTRAIL_SCREENSHOT") else {
        return;
    };
    settle();
    settle();
    settle();
    let path = std::path::PathBuf::from(path);
    let path = if suffix.is_empty() {
        path
    } else {
        path.with_extension(format!("{suffix}.png"))
    };
    let paintable = gtk::WidgetPaintable::new(Some(&d.window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, d.window.width() as f64, d.window.height() as f64);
    let node = snapshot.to_node().unwrap();
    d.window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(path)
        .unwrap();
}

fn background_sync_is_non_intrusive(app: &adw::Application, root: &std::path::Path) {
    use papertrail_sync::control::Outcome;
    let gtk_settings = gtk::Settings::default().unwrap();
    let animations = gtk_settings.property::<bool>("gtk-enable-animations");
    gtk_settings.set_property("gtk-enable-animations", true);
    let folder = root.join("notes");
    let d = Desktop::with_settings(
        app,
        SettingsFile {
            path: root.join("settings.json"),
        },
        Settings::new(folder.clone()),
        &folder,
    )
    .unwrap();
    d.window.present();
    d.new_note().unwrap();
    let first = d.current_path().unwrap();
    d.buffer.set_text("Saved before the network request");
    d.flush().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<Outcome>>();
    *d.sync_job.borrow_mut() = Some(rx);
    d.sync_unlocked.set(true);
    d.sync_indicator();
    assert!(d.window.content().unwrap().is_sensitive());
    assert!(d.view.is_sensitive());
    let sync = d.widget::<gtk::MenuButton>("sync_menu_button");
    assert!(gtk::IconTheme::for_display(&d.view.display()).has_icon(
        d.widget::<gtk::Image>("sync_icon")
            .icon_name()
            .unwrap()
            .as_str()
    ));
    assert!(sync.is_sensitive());
    assert!(d
        .widget::<gtk::Image>("sync_icon")
        .has_css_class("sync-spinning"));
    gtk_settings.set_property("gtk-enable-animations", false);
    assert!(!d
        .widget::<gtk::Image>("sync_icon")
        .has_css_class("sync-spinning"));
    gtk_settings.set_property("gtk-enable-animations", true);
    assert!(d.window.lookup_action("sync-status").is_none());
    assert!(!d.window.lookup_action("sync-login").unwrap().is_enabled());
    settle();
    let bounds = sync.compute_bounds(&d.window).unwrap();
    assert!(bounds.x() < 64.0);
    assert!(bounds.y() > (d.window.height() - 100) as f32);
    sync.popup();
    settle();
    assert!(sync.popover().unwrap().is_visible());
    capture(&d, "sync-menu");
    sync.popdown();
    // Navigation, local saves, renames and new typing all continue during a job.
    d.new_note().unwrap();
    let second = d.current_path().unwrap();
    assert_eq!(d.sync.local().unwrap().protected, vec![second.clone()]);
    d.rename_note(&second, "renamed.txt").unwrap();
    let renamed = d.current_path().unwrap();
    assert_eq!(d.sync.local().unwrap().protected, vec![renamed]);
    d.open(&first).unwrap();
    d.buffer.set_text("Still typing during sync");
    assert!(d.model.borrow().session.is_dirty());
    tx.send(Err(std::io::Error::other(
        "Network temporarily unavailable",
    )))
    .unwrap();
    d.sync_poll();
    assert!(
        d.window.visible_dialog().is_none(),
        "background failure opened a modal dialog"
    );
    assert!(d.sync_unlocked.get());
    assert!(d.sync_error.get());
    assert!(!d
        .widget::<gtk::Image>("sync_icon")
        .has_css_class("sync-spinning"));
    assert!(d.window.lookup_action("sync-login").unwrap().is_enabled());
    assert_eq!(d.model.borrow().session.text, "Still typing during sync");
    // A delayed result must merge metadata, not replace current preferences or text.
    let baseline = d.model.borrow().settings.clone();
    let mut updated = baseline.clone();
    updated.set_pinned(&first, true);
    updated
        .note_language_overrides
        .insert(first.clone(), "python".into());
    d.update_settings(|s| {
        s.theme_mode = "dark".into();
        s.note_language_overrides
            .insert(first.clone(), "rust".into());
    })
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    *d.sync_job.borrow_mut() = Some(rx);
    tx.send(Ok(Outcome {
        message: "Synced quietly".into(),
        recovery: None,
        synced: true,
        unlocked: true,
        settings_update: Some((Box::new(baseline), Box::new(updated))),
        local_changed: false,
    }))
    .unwrap();
    d.sync_poll();
    assert_eq!(d.model.borrow().settings.theme_mode, "dark");
    assert_eq!(
        d.model.borrow().settings.note_language_overrides[&first],
        "rust"
    );
    assert!(d.model.borrow().settings.pinned_notes.contains(&first));
    assert_eq!(d.model.borrow().session.text, "Still typing during sync");
    assert!(d.model.borrow().session.is_dirty());
    assert!(d.window.visible_dialog().is_none());
    d.sync_unlocked.set(false);
    capture(&d, "sync-idle");
    d.window.close();
    settle();
    gtk_settings.set_property("gtk-enable-animations", animations);
}

fn sync_configuration_prefills_and_preserves_multiple_selected_folders(
    app: &adw::Application,
    root: &std::path::Path,
) {
    let a = root.join("Personal");
    let b = root.join("Work");
    let c = root.join("Not selected");
    for path in [&a, &b, &c] {
        fs::create_dir_all(path).unwrap();
    }
    let mut settings = Settings::new(a.clone());
    settings.add_folder(b.clone(), false);
    settings.add_folder(c.clone(), false);
    let d = Desktop::with_settings(
        app,
        SettingsFile {
            path: root.join("settings.json"),
        },
        settings,
        &a,
    )
    .unwrap();
    d.sync
        .configure(papertrail_sync::Config {
            version: 2,
            server: "https://sync.example".into(),
            issuer: "https://issuer.example".into(),
            client_id: "public-native".into(),
            scopes: "openid offline_access custom-audience".into(),
            folders: vec![a.clone(), b.clone()],
            download_dir: Some(root.join("Downloads")),
            ..papertrail_sync::Config::default()
        })
        .unwrap();
    d.choose_active(Some(&c)).unwrap();
    d.window.present();
    settle();
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    assert!(!d
        .widget::<adw::SwitchRow>("sync_config_discovery")
        .is_active());
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_server").text(),
        "https://sync.example"
    );
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_issuer").text(),
        "https://issuer.example"
    );
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_client").text(),
        "public-native"
    );
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_scopes").text(),
        "openid offline_access custom-audience"
    );
    capture(&d, "sync-configure-folders");
    d.widget::<adw::EntryRow>("sync_config_scopes")
        .set_text("openid offline_access changed-scope");
    let dialog = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    dialog.emit_by_name::<()>("response", &[&"save"]);
    dialog.close();
    settle();
    let config = d.sync.saved_config().unwrap().unwrap();
    assert!(config.folders.contains(&a));
    assert!(config.folders.contains(&b));
    assert!(!config.folders.contains(&c));
    assert_eq!(config.scopes, "openid offline_access changed-scope");
    assert_eq!(config.download_dir, Some(root.join("Downloads")));
    assert!(d.window.lookup_action("sync-status").is_none());
    d.window.close();
    settle();
}

fn sync_discovery_trust_is_explicit(app: &adw::Application, root: &std::path::Path) {
    let notes = root.join("notes");
    fs::create_dir_all(&notes).unwrap();
    let d = Desktop::with_settings(
        app,
        SettingsFile {
            path: root.join("settings.json"),
        },
        Settings::new(notes.clone()),
        &notes,
    )
    .unwrap();
    d.window.present();
    settle();
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    assert!(d
        .widget::<adw::SwitchRow>("sync_config_discovery")
        .is_active());
    assert!(!d
        .widget::<adw::EntryRow>("sync_config_issuer")
        .is_sensitive());
    d.window.visible_dialog().unwrap().close();
    settle();
    let config = papertrail_sync::Config {
        server: "https://sync.example".into(),
        issuer: "https://issuer.example".into(),
        client_id: "native".into(),
        scopes: "openid offline_access".into(),
        discover_auth: true,
        folders: vec![notes],
        ..papertrail_sync::Config::default()
    };
    d.sync_confirm_discovered(config.clone(), None, vec![]);
    settle();
    let dialog = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(dialog.body().contains("https://sync.example"));
    assert!(dialog.body().contains("https://issuer.example"));
    assert!(!d.sync.configured());
    dialog.emit_by_name::<()>("response", &[&"cancel"]);
    dialog.close();
    settle();
    assert!(!d.sync.configured());
    d.sync_confirm_discovered(config, None, vec![]);
    settle();
    let dialog = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    dialog.emit_by_name::<()>("response", &[&"accept"]);
    dialog.close();
    settle();
    let saved = d.sync.saved_config().unwrap().unwrap();
    assert!(saved.discover_auth);
    d.sync_confirm_discovered(saved.clone(), Some(saved.clone()), vec![]);
    settle();
    assert!(d.window.visible_dialog().is_none());
    let mut changed = saved.clone();
    changed.issuer = "https://new-issuer.example".into();
    d.sync_confirm_discovered(changed, Some(saved.clone()), vec![]);
    settle();
    let dialog = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(dialog.body().contains("Previously accepted"));
    dialog.emit_by_name::<()>("response", &[&"cancel"]);
    dialog.close();
    settle();
    assert!(d.sync.saved_config().unwrap().unwrap() == saved);
    d.window.close();
    settle();
}

fn transient_dialog_regressions(app: &adw::Application, root: &std::path::Path) {
    let notes = root.join("Original notes");
    fs::create_dir_all(&notes).unwrap();
    let file = notes.join("keep.txt");
    fs::write(&file, "Keep this note").unwrap();
    let d = Desktop::with_settings(
        app,
        SettingsFile {
            path: root.join("settings.json"),
        },
        Settings::new(notes.clone()),
        &notes,
    )
    .unwrap();
    let mut config = papertrail_sync::Config {
        server: "https://sync.example".into(),
        issuer: "https://issuer.example".into(),
        client_id: "native".into(),
        scopes: "openid offline_access".into(),
        folders: vec![notes.clone()],
        ..papertrail_sync::Config::default()
    };
    d.sync.configure(config.clone()).unwrap();
    d.window.present();
    settle();

    let footer = d.widget::<gtk::Widget>("sync_footer");
    let button = d.widget::<gtk::Widget>("sync_menu_button");
    assert_eq!(footer.halign(), gtk::Align::Fill);
    assert_eq!((footer.margin_top(), footer.margin_bottom()), (6, 10));
    assert_eq!(button.halign(), gtk::Align::Center);
    let bounds = button.compute_bounds(&footer).unwrap();
    assert!((bounds.x() + bounds.width() / 2.0 - footer.width() as f32 / 2.0).abs() <= 1.0);

    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    let original_row = d.widget::<adw::EntryRow>("sync_config_server");
    original_row.set_text("https://discard-this-edit.example");
    assert!(d
        .builder
        .object::<adw::EntryRow>("sync_config_server")
        .is_none());
    let first = d.window.visible_dialog().unwrap();
    first.close();
    // Reopen before the previous closing animation finishes. Its closed signal
    // must not clear this newer dialog's rows or retain the cancelled values.
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    let current_row = d.widget::<adw::EntryRow>("sync_config_server");
    assert_ne!(original_row, current_row);
    assert_eq!(current_row.text(), "https://sync.example");
    assert!(d.sync.saved_config().unwrap().unwrap() == config);
    d.window.visible_dialog().unwrap().close();
    wait_until(|| d.sync_config_widgets.borrow().is_none());

    config.scopes = "openid offline_access updated".into();
    d.sync.configure(config.clone()).unwrap();
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_scopes").text(),
        config.scopes
    );
    let dialog = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    dialog.emit_by_name::<()>("response", &[&"save"]);
    dialog.close();
    wait_until(|| d.sync_config_widgets.borrow().is_none());
    let saved = d.sync.saved_config().unwrap().unwrap();
    assert_eq!(saved.scopes, config.scopes);
    config = saved;

    // EntryRow needs the PreferencesGroup's native ListBox parent. Reopening
    // Edit Folder must keep that hierarchy and leave cancelled edits untouched.
    let edit = d.edit_folder(notes.clone());
    settle();
    let entry = descendant(edit.upcast_ref(), &|widget| widget.is::<adw::EntryRow>())
        .unwrap()
        .downcast::<adw::EntryRow>()
        .unwrap();
    assert!(entry.parent().unwrap().is::<gtk::ListBox>());
    entry.set_text("Cancelled name");
    let cancel = descendant(edit.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Cancel"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    cancel.emit_clicked();
    wait_until(|| !edit.is_visible());
    assert!(notes.is_dir());
    assert!(!root.join("Cancelled name").exists());

    let edit = d.edit_folder(notes.clone());
    settle();
    let entry = descendant(edit.upcast_ref(), &|widget| widget.is::<adw::EntryRow>())
        .unwrap()
        .downcast::<adw::EntryRow>()
        .unwrap();
    assert_eq!(entry.text(), "Original notes");
    assert!(entry.parent().unwrap().is::<gtk::ListBox>());
    let color_class = papertrail_core::colors::css_class("preset-4");
    descendant(edit.upcast_ref(), &|widget| {
        widget.is::<gtk::ToggleButton>() && widget.has_css_class(&color_class)
    })
    .unwrap()
    .downcast::<gtk::ToggleButton>()
    .unwrap()
    .set_active(true);
    descendant(edit.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Save"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap()
    .emit_clicked();
    wait_until(|| !edit.is_visible());
    assert_eq!(d.model.borrow().settings.folder_colors[&notes], "preset-4");
    assert_eq!(fs::read_to_string(&file).unwrap(), "Keep this note");
    assert!(d.sync.saved_config().unwrap().unwrap() == config);
    d.window.close();
    settle();
}

#[test]
fn desktop_workflows_use_embedded_assets_and_preserve_pending_edits() {
    let diagnostics = record_ui_diagnostics();
    adw::init().expect("GTK tests need a display (use xvfb-run)");
    gio::resources_register_include!("papertrail.gresource").unwrap();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes");
    let mut settings = Settings::new(notes);
    settings.theme_mode = "light".into();
    let default_notes_dir = settings.notes_dir.clone();
    let d = Desktop::with_settings(
        &app,
        SettingsFile {
            path: temp.path().join("config/settings.json"),
        },
        settings,
        &default_notes_dir,
    )
    .unwrap();
    for (name, _, _) in crate::actions::SHORTCUTS {
        assert!(d.window.lookup_action(name).is_some(), "missing {name}");
    }
    for name in [
        "sync-configure",
        "sync-login",
        "sync-create",
        "sync-unlock",
        "sync-recover",
        "sync-passphrase",
        "sync-now",
        "sync-lock",
        "sync-logout",
    ] {
        assert!(d.window.lookup_action(name).is_some(), "missing {name}");
    }
    assert!(!d.window.lookup_action("delete-note").unwrap().is_enabled());
    assert_eq!(
        d.widget::<gtk::Label>("editor_title_label").text(),
        "Get started"
    );
    d.window.present();
    settle();
    assert_no_application_diagnostics(&d, &diagnostics);
    capture(&d, "");
    d.new_note().unwrap();
    let first = d.current_path().unwrap();
    let cancel = d.confirm_delete(first.clone());
    settle();
    cancel.response("cancel");
    // response() emits the signal only; unlike a real response-button click it
    // does not dismiss this toplevel dialog.
    cancel.close();
    settle();
    assert!(first.exists());
    let anchor = d.widget::<gtk::Box>("note_list").first_child().unwrap();
    let rename = d.rename_popover(first.clone(), &anchor);
    settle();
    rename.popdown();
    settle();
    assert!(first.exists());
    d.buffer.set_text("Title\nPending edits");
    assert!(d.model.borrow().session.is_dirty());
    d.new_note().unwrap();
    assert_eq!(fs::read_to_string(&first).unwrap(), "Title\nPending edits");
    let second = d.current_path().unwrap();
    let languages = d.widget::<gtk::ListBox>("language_listbox");
    let mut child = languages.first_child();
    while let Some(row) = child {
        if row.widget_name() == "rust" {
            languages.emit_by_name::<()>(
                "row-activated",
                &[&row.downcast::<gtk::ListBoxRow>().unwrap()],
            );
            break;
        }
        child = row.next_sibling();
    }
    assert_eq!(d.buffer.language().unwrap().id(), "rust");
    assert_eq!(
        d.model.borrow().settings.note_language_overrides[&second],
        "rust"
    );
    d.buffer.set_text("Second\ntext");
    d.flush().unwrap();
    d.open(&first).unwrap();
    assert_eq!(
        d.buffer
            .text(&d.buffer.start_iter(), &d.buffer.end_iter(), true),
        "Title\nPending edits"
    );
    d.model.borrow_mut().settings.set_pinned(&first, true);
    d.rename_note(&first, "renamed.txt").unwrap();
    let renamed = d.current_path().unwrap();
    assert!(d.model.borrow().settings.pinned_notes.contains(&renamed));
    assert!(!first.exists());
    let other = temp.path().join("other");
    d.move_note(&renamed, &other).unwrap();
    let moved = d.current_path().unwrap();
    assert!(d.model.borrow().settings.pinned_notes.contains(&moved));
    d.choose_active(None).unwrap();
    capture(&d, "recent");
    assert!(d.visible_notes().unwrap().iter().any(|n| n.path == second));
    // Recent tiles and search results can open notes outside the active folder.
    d.open(&second).unwrap();
    assert_eq!(
        d.model.borrow().settings.notes_dir,
        second.parent().unwrap()
    );
    d.buffer.set_text("One one ONE");
    d.open_search(true);
    d.widget::<gtk::SearchEntry>("editor_search_entry")
        .set_text("one");
    settle();
    wait_until(|| d.search.occurrences_count() == 3);
    d.find(true);
    let forward = d.buffer.selection_bounds().unwrap().0.offset();
    d.find(false);
    assert_ne!(forward, d.buffer.selection_bounds().unwrap().0.offset());
    d.widget::<gtk::Entry>("editor_replace_entry")
        .set_text("two");
    d.replace(true).unwrap();
    assert_eq!(
        d.buffer
            .text(&d.buffer.start_iter(), &d.buffer.end_iter(), true),
        "two two two"
    );
    d.buffer.undo();
    assert_eq!(
        d.buffer
            .text(&d.buffer.start_iter(), &d.buffer.end_iter(), true),
        "One one ONE"
    );
    capture(&d, "editor");
    d.update_settings(|s| {
        s.theme_mode = "dark".into();
        s.show_line_numbers = false;
        s.wrap_text = false;
    })
    .unwrap();
    assert!(!d.view.shows_line_numbers());
    assert_eq!(d.view.wrap_mode(), gtk::WrapMode::None);
    assert_eq!(
        d.window
            .lookup_action("toggle-wrap")
            .unwrap()
            .state()
            .unwrap()
            .get::<bool>(),
        Some(false)
    );
    assert_eq!(d.buffer.style_scheme().unwrap().id(), "Adwaita-dark");
    d.window.present();
    settle();
    capture(&d, "dark");
    d.preferences();
    settle();
    capture(&d, "preferences");
    let pref_wrap = d
        .builder
        .object::<adw::SwitchRow>("preference_switch_1")
        .unwrap();
    assert!(!pref_wrap.is_active());
    d.update_settings(|s| s.wrap_text = true).unwrap();
    assert!(pref_wrap.is_active());
    pref_wrap.set_active(false);
    assert!(!d.model.borrow().settings.wrap_text);
    d.builder
        .object::<adw::PreferencesDialog>("preferences_dialog")
        .unwrap()
        .close();
    settle();
    d.update_settings(|s| s.theme_mode = "light".into())
        .unwrap();
    d.buffer.set_text("Autosaved by GTK timer");
    wait_until(|| !d.model.borrow().session.is_dirty());
    assert_eq!(
        fs::read_to_string(&second).unwrap(),
        "Autosaved by GTK timer"
    );
    d.flush().unwrap();
    d.remove_note(&second).unwrap();
    assert!(!second.exists());
    assert!(!d
        .widget::<gtk::Revealer>("editor_search_revealer")
        .reveals_child());
    assert!(!d
        .widget::<gtk::Revealer>("editor_replace_revealer")
        .reveals_child());
    d.window.present();
    settle();
    assert!(d.window.is_visible());
    editor_padding_and_filetype_regressions(&d, &temp.path().join("filetypes"));
    current_line_highlight_regression(&d);
    sidebar_and_shortcuts_regressions(&d);
    d.window.close();
    settle();
    startup_folder_recovery_regression(&app, &temp.path().join("startup-recovery"));
    background_sync_is_non_intrusive(&app, &temp.path().join("background-sync"));
    sync_configuration_prefills_and_preserves_multiple_selected_folders(
        &app,
        &temp.path().join("configure-folders"),
    );
    sync_discovery_trust_is_explicit(&app, &temp.path().join("discovery-trust"));
    transient_dialog_regressions(&app, &temp.path().join("transient-dialogs"));
    assert_no_application_diagnostics(&d, &diagnostics);
}
