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
    // A newly attached paintable may not have received its first rendered frame,
    // especially on loaded CI runners. Wait for actual content, not a fixed delay.
    d.window.queue_draw();
    let deadline = Instant::now() + Duration::from_secs(5);
    let node = loop {
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, d.window.width() as f64, d.window.height() as f64);
        if let Some(node) = snapshot.to_node() {
            break node;
        }
        assert!(
            Instant::now() < deadline,
            "No GTK screenshot frame for {}",
            path.display()
        );
        settle();
    };
    d.window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(path)
        .unwrap();
}

fn remembered_password_dialogs(app: &adw::Application, root: &std::path::Path) {
    let notes = root.join("notes");
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
    for action in ["sync-create", "sync-unlock", "sync-passphrase"] {
        d.window.lookup_action(action).unwrap().activate(None);
        settle();
        let dialog = d
            .window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        let remember = descendant(dialog.upcast_ref(), &|widget| {
            widget
                .downcast_ref::<adw::SwitchRow>()
                .is_some_and(|row| row.title() == "Remember encryption password on this device")
        });
        let row = remember.unwrap().downcast::<adw::SwitchRow>().unwrap();
        assert!(!row.is_active(), "Remembering must be opt-in");
        row.set_active(true);
        if action == "sync-unlock" {
            let password = descendant(dialog.upcast_ref(), &|widget| {
                widget.is::<adw::PasswordEntryRow>()
            })
            .unwrap()
            .downcast::<adw::PasswordEntryRow>()
            .unwrap();
            password.set_text("discarded passphrase");
            dialog.emit_by_name::<()>("response", &[&"recover"]);
            settle();
            assert!(password.text().is_empty());
            let recovery = d
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap();
            assert_eq!(recovery.heading().as_deref(), Some("Recover Vault"));
            assert!(
                descendant(recovery.upcast_ref(), &|widget| widget
                    .is::<adw::SwitchRow>())
                .is_none(),
                "Recovery keys must never be remembered"
            );
            let key = descendant(recovery.upcast_ref(), &|widget| {
                widget.is::<adw::PasswordEntryRow>()
            })
            .unwrap()
            .downcast::<adw::PasswordEntryRow>()
            .unwrap();
            assert_eq!(key.title(), "Recovery key");
            key.set_text("discarded recovery key");
            recovery.emit_by_name::<()>("response", &[&"password"]);
            settle();
            assert!(key.text().is_empty());
            let unlock = d
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap();
            assert_eq!(unlock.heading().as_deref(), Some("Unlock Vault"));
            let remember = descendant(unlock.upcast_ref(), &|widget| widget.is::<adw::SwitchRow>())
                .unwrap()
                .downcast::<adw::SwitchRow>()
                .unwrap();
            assert!(!remember.is_active());
            unlock.emit_by_name::<()>("response", &[&"recover"]);
            settle();
            let recovery = d
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap();
            recovery.emit_by_name::<()>("response", &[&"cancel"]);
            recovery.close();
        } else {
            dialog.emit_by_name::<()>("response", &[&"cancel"]);
            dialog.close();
        }
        settle();
        assert!(d.sync_job.borrow().is_none());
    }
    assert!(!root.join("sync/config.json").exists());
    d.window.close();
    settle();
}

fn successful_login_prompts_for_unlock_only_when_needed(
    app: &adw::Application,
    root: &std::path::Path,
) {
    use papertrail_sync::control::Outcome;
    let notes = root.join("notes");
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
    for (manual, unlocked, cancelled, failed) in [
        (true, false, false, false),
        (false, false, false, false), // Startup restore must stay non-interactive.
        (true, true, false, false),
        (true, false, true, false),
        (true, false, false, true),
    ] {
        let (tx, rx) = std::sync::mpsc::channel();
        *d.sync_job.borrow_mut() = Some(rx);
        d.sync_prompt_after_login.set(manual);
        d.sync_interactive.set(manual);
        d.sync_unlocked.set(unlocked);
        let cancel = papertrail_sync::control::Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        *d.sync_cancel.borrow_mut() = Some(cancel);
        tx.send(if failed {
            Err(std::io::Error::other("Test sign-in failure"))
        } else {
            Ok(Outcome {
                message: "Signed in".into(),
                warning: None,
                recovery: None,
                synced: false,
                unlocked,
                settings_update: None,
                local_changed: false,
            })
        })
        .unwrap();
        d.sync_poll();
        assert!(!d.sync_prompt_after_login.get());
        settle();
        if manual && !unlocked && !cancelled {
            let dialog = d
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap();
            assert_eq!(
                dialog.heading().as_deref(),
                Some(if failed {
                    "Could Not Complete Operation"
                } else {
                    "Unlock Vault"
                })
            );
            dialog.emit_by_name::<()>("response", &[&"cancel"]);
            dialog.close();
            settle();
        } else {
            assert!(d.window.visible_dialog().is_none());
        }
        d.sync_poll();
        assert!(
            d.window.visible_dialog().is_none(),
            "prompt must not repeat after dismissal"
        );
        assert!(d.sync_job.borrow().is_none());
    }
    d.window.close();
    settle();
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
    // Repeated status refreshes must not rebuild menus or re-notify hover
    // tooltips/status labels when no application state has changed.
    let changes = Rc::new(std::cell::Cell::new(0));
    for menu in [
        d.widget::<gio::Menu>("sync_operation_menu"),
        d.widget::<gio::Menu>("sync_actions_menu"),
    ] {
        let count = changes.clone();
        menu.connect_items_changed(move |_, _, _, _| count.set(count.get() + 1));
    }
    let count = changes.clone();
    d.widget::<gtk::MenuButton>("sync_menu_button")
        .connect_notify_local(Some("tooltip-text"), move |_, _| count.set(count.get() + 1));
    let count = changes.clone();
    d.widget::<gtk::Label>("sync_status_label")
        .connect_notify_local(Some("label"), move |_, _| count.set(count.get() + 1));
    for _ in 0..20 {
        d.sync_indicator();
    }
    assert_eq!(changes.get(), 0);
    let operation_menu = d.widget::<gio::Menu>("sync_operation_menu");
    let actions_menu = d.widget::<gio::Menu>("sync_actions_menu");
    let menu_item = |menu: &gio::Menu, index: i32| {
        let value = |attribute| {
            menu.item_attribute_value(index, attribute, None)
                .unwrap()
                .get::<String>()
                .unwrap()
        };
        (value("label"), value("action"))
    };
    assert_eq!(operation_menu.n_items(), 1);
    assert_eq!(actions_menu.n_items(), 5);
    assert!(d.window.lookup_action("sync-recover").is_none());
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Sync Now".into(), "win.sync-now".into())
    );
    assert_eq!(
        menu_item(&actions_menu, 1),
        ("Sign In".into(), "win.sync-login".into())
    );
    assert_eq!(
        menu_item(&actions_menu, 3),
        ("Unlock Vault".into(), "win.sync-unlock".into())
    );
    d.new_note().unwrap();
    let first = d.current_path().unwrap();
    d.buffer.set_text("Saved before the network request");
    d.flush().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<Outcome>>();
    *d.sync_job.borrow_mut() = Some(rx);
    d.sync_unlocked.set(true);
    d.sync_indicator();
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Operation Running…".into(), "win.sync-now".into())
    );
    assert!(!d.window.lookup_action("sync-now").unwrap().is_enabled());
    assert_eq!(
        menu_item(&actions_menu, 3),
        ("Lock Vault".into(), "win.sync-lock".into())
    );
    *d.sync_cancel.borrow_mut() = Some(papertrail_sync::control::Cancellation::default());
    d.sync_indicator();
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Cancel Operation".into(), "win.sync-cancel".into())
    );
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
    let before_hover = changes.get();
    let mut widgets = vec![sync.popover().unwrap().upcast::<gtk::Widget>()];
    let mut entries = 0;
    while let Some(widget) = widgets.pop() {
        let mut child = widget.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            widgets.push(item);
        }
        if widget.css_name() == "modelbutton" {
            entries += 1;
            widget.set_state_flags(gtk::StateFlags::PRELIGHT, false);
            settle();
            widget.unset_state_flags(gtk::StateFlags::PRELIGHT);
        }
    }
    assert!(entries > 0, "exercise native menu hover styling");
    for _ in 0..20 {
        d.sync_indicator();
    }
    assert_eq!(
        changes.get(),
        before_hover,
        "hover/unchanged refresh must not rewrite the menu or tooltip"
    );
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
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Sync Now".into(), "win.sync-now".into())
    );
    assert!(d.window.lookup_action("sync-now").unwrap().is_enabled());
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
        warning: None,
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
    // Cancellation keeps the worker attached until it acknowledges stopping,
    // does not show an error dialog, and pauses timer/edit-triggered retries.
    let (tx, rx) = std::sync::mpsc::channel();
    *d.sync_job.borrow_mut() = Some(rx);
    let cancel = papertrail_sync::control::Cancellation::default();
    *d.sync_cancel.borrow_mut() = Some(cancel.clone());
    d.sync_interactive.set(true);
    d.sync_indicator();
    assert!(d.window.lookup_action("sync-cancel").unwrap().is_enabled());
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Cancel Operation".into(), "win.sync-cancel".into())
    );
    let (events, receiver) = std::sync::mpsc::channel();
    *d.sync_browser_events.borrow_mut() = Some(receiver);
    events
        .send(papertrail_sync::auth::BrowserEvent::Progress(
            "Checking identity provider…",
        ))
        .unwrap();
    d.sync_poll();
    assert_eq!(&*d.sync_status.borrow(), "Checking identity provider…");
    d.window
        .lookup_action("sync-cancel")
        .unwrap()
        .activate(None);
    assert!(cancel.is_cancelled());
    assert!(d.sync_job.borrow().is_some());
    assert!(d.sync_status.borrow().contains("Cancelling"));
    assert!(!d.window.lookup_action("sync-cancel").unwrap().is_enabled());
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Cancelling…".into(), "win.sync-cancel".into())
    );
    // A launch request that was queued before Cancel must not open a browser.
    let (reply, result) = std::sync::mpsc::channel();
    events
        .send(papertrail_sync::auth::BrowserEvent::Open {
            url: "https://example.invalid/PRIVATE_AUTHORIZATION_URL".into(),
            reply,
        })
        .unwrap();
    d.sync_poll();
    assert_eq!(
        result.recv().unwrap().unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert!(d.sync_browser_launch.borrow().is_none());
    tx.send(Err(std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "Sign-in cancelled",
    )))
    .unwrap();
    d.sync_poll();
    assert!(d.sync_job.borrow().is_none());
    assert!(d.sync_cancel.borrow().is_none());
    assert!(!d.sync_error.get());
    assert!(d.sync_paused.get());
    assert_eq!(
        menu_item(&operation_menu, 0),
        ("Sync Now".into(), "win.sync-now".into())
    );
    assert_eq!(operation_menu.n_items(), 1);
    assert_eq!(actions_menu.n_items(), 5);
    d.schedule_sync();
    assert!(d.sync_requested.get().is_none());
    assert!(d.window.visible_dialog().is_none());
    assert_eq!(d.model.borrow().session.text, "Still typing during sync");
    d.sync_unlocked.set(false);
    d.sync_indicator();
    assert_eq!(
        menu_item(&actions_menu, 3),
        ("Unlock Vault".into(), "win.sync-unlock".into())
    );
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
    // Only published folders get a direct trash button, and deletion always
    // opens a native destructive confirmation rather than firing immediately.
    let folder_id = "AAAAAAAAAAAAAAAAAAAAAA".to_owned();
    let draft_id = "AQEBAQEBAQEBAQEBAQEBAQ".to_owned();
    {
        let mut engine =
            papertrail_sync::engine::Engine::open(root.join("sync/state.json")).unwrap();
        engine.state.vault_id = "AgICAgICAgICAgICAgICAg".into();
        engine.state.catalog = Some(
            serde_json::from_value(serde_json::json!({
                "version": 2,
                "folders": { (&folder_id): { "name": "Personal", "color": "preset-0" } },
                "legacy_notes": {}
            }))
            .unwrap(),
        );
        engine.state.catalog_draft = Some(
            serde_json::from_value(serde_json::json!({
                "version": 2,
                "folders": {
                    (&folder_id): { "name": "Personal", "color": "preset-0" },
                    (&draft_id): { "name": "Work", "color": "preset-0" }
                },
                "legacy_notes": {}
            }))
            .unwrap(),
        );
        for (id, path) in [(&folder_id, &a), (&draft_id, &b)] {
            engine.state.bindings.insert(
                id.clone(),
                papertrail_sync::folders::Binding {
                    path: path.clone(),
                    managed: false,
                    observed_name: path.file_name().unwrap().to_string_lossy().into_owned(),
                    observed_color: None,
                },
            );
        }
        engine.save().unwrap();
    }
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    let config_dialog = d.window.visible_dialog().unwrap();
    let trash = descendant(config_dialog.upcast_ref(), &|w| {
        w.is::<gtk::Button>()
            && w.clone()
                .downcast::<gtk::Button>()
                .unwrap()
                .icon_name()
                .as_deref()
                == Some("user-trash-symbolic")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    assert!(!trash.is_sensitive()); // Sign in and unlock first.
    assert!(!trash.has_css_class("destructive-action"));
    assert!(trash
        .tooltip_text()
        .unwrap()
        .starts_with("Delete from Server"));
    let work = descendant(config_dialog.upcast_ref(), &|widget| {
        widget
            .clone()
            .downcast::<adw::ActionRow>()
            .is_ok_and(|row| row.title() == "Work")
    })
    .unwrap();
    assert!(descendant(&work, &|widget| widget
        .clone()
        .downcast::<gtk::Button>()
        .is_ok_and(
            |button| button.icon_name().as_deref() == Some("user-trash-symbolic")
        ))
    .is_none());
    let mut ancestor = trash.parent();
    while let Some(widget) = ancestor {
        if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let adjustment = scroll.vadjustment();
            adjustment.set_value(adjustment.upper());
        }
        ancestor = widget.parent();
    }
    capture(&d, "sync-configure-trash-button");
    trash.emit_by_name::<()>("clicked", &[]);
    settle();
    let confirm = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(confirm.heading().unwrap().contains("Personal"));
    assert!(confirm
        .body()
        .contains("Local files on all devices are kept"));
    assert!(confirm
        .body()
        .contains("won’t be uploaded again automatically"));
    assert_eq!(
        confirm.response_appearance("delete"),
        adw::ResponseAppearance::Destructive
    );
    capture(&d, "sync-delete-folder-confirmation");
    confirm.emit_by_name::<()>("response", &[&"cancel"]);
    confirm.close();
    settle();
    assert!(d.sync_job.borrow().is_none());
    assert!(d
        .sync
        .snapshot()
        .unwrap()
        .unwrap()
        .catalog
        .unwrap()
        .deleted_folders
        .is_empty());
    // Completing deletion updates this dialog, not a rebuilt replacement, so
    // unrelated edits remain in the same widgets on both failure and success.
    d.widget::<adw::EntryRow>("sync_config_scopes")
        .set_text("openid unsaved-scope");
    let row = trash
        .ancestor(adw::ActionRow::static_type())
        .unwrap()
        .downcast::<adw::ActionRow>()
        .unwrap();
    let folders = row
        .ancestor(adw::PreferencesGroup::static_type())
        .unwrap()
        .downcast::<adw::PreferencesGroup>()
        .unwrap();
    assert!(descendant(config_dialog.upcast_ref(), &|widget| {
        widget
            .clone()
            .downcast::<adw::PreferencesGroup>()
            .is_ok_and(|group| group.title() == "Removed from Sync")
    })
    .is_none());
    let selected = descendant(row.upcast_ref(), &|widget| widget.is::<gtk::CheckButton>())
        .unwrap()
        .downcast::<gtk::CheckButton>()
        .unwrap();
    let alert = config_dialog
        .clone()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    let deletion_view =
        crate::sync::FolderDeletionView::new(&folder_id, &alert, &folders, &row, &selected, None);
    deletion_view.set_busy(true);
    d.sync_folder_deletion.replace(Some(deletion_view.clone()));
    let (tx, rx) = std::sync::mpsc::channel();
    *d.sync_job.borrow_mut() = Some(rx);
    assert!(!row.is_sensitive());
    assert!(!alert.is_response_enabled("save"));
    assert_eq!(d.window.visible_dialog().unwrap(), config_dialog);
    d.sync_interactive.set(true);
    tx.send(Err(std::io::Error::other("Deletion unavailable")))
        .unwrap();
    d.sync_poll();
    settle();
    let error = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(error.body().contains("Deletion unavailable"));
    error.close();
    settle();
    assert_eq!(d.window.visible_dialog().unwrap(), config_dialog);
    assert!(row.is_sensitive());
    assert!(alert.is_response_enabled("save"));
    assert!(selected.is_active());
    assert!(descendant(config_dialog.upcast_ref(), &|widget| widget
        .clone()
        .downcast::<gtk::Button>()
        .is_ok_and(
            |button| button.label().as_deref() == Some("Sync as New Folder…")
        ))
    .is_none());
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_scopes").text(),
        "openid unsaved-scope"
    );
    {
        let mut engine =
            papertrail_sync::engine::Engine::open(root.join("sync/state.json")).unwrap();
        engine.state.catalog = engine
            .state
            .catalog_draft
            .take()
            .or(engine.state.catalog.take());
        engine
            .state
            .catalog
            .as_mut()
            .unwrap()
            .deleted_folders
            .insert(folder_id.clone());
        engine.state.catalog_draft = None;
        engine.save().unwrap();
    }
    deletion_view.set_busy(true);
    d.sync_folder_deletion.replace(Some(deletion_view));
    let (tx, rx) = std::sync::mpsc::channel();
    *d.sync_job.borrow_mut() = Some(rx);
    tx.send(Ok(papertrail_sync::control::Outcome {
        message: "Folder deleted from server".into(),
        warning: None,
        recovery: None,
        synced: false,
        unlocked: false,
        settings_update: None,
        local_changed: false,
    }))
    .unwrap();
    d.sync_poll();
    settle();
    assert_eq!(d.window.visible_dialog().unwrap(), config_dialog);
    assert!(alert.is_response_enabled("save"));
    assert!(!selected.is_active());
    assert!(row.parent().is_none());
    assert_eq!(
        d.widget::<adw::EntryRow>("sync_config_scopes").text(),
        "openid unsaved-scope"
    );
    assert_eq!(
        d.sync.saved_config().unwrap().unwrap().scopes,
        "openid offline_access changed-scope"
    );
    let again = descendant(config_dialog.upcast_ref(), &|widget| {
        widget
            .clone()
            .downcast::<gtk::Button>()
            .is_ok_and(|button| button.label().as_deref() == Some("Sync as New Folder…"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    let local_row = again
        .ancestor(adw::ActionRow::static_type())
        .unwrap()
        .downcast::<adw::ActionRow>()
        .unwrap();
    assert_eq!(
        local_row
            .ancestor(adw::PreferencesGroup::static_type())
            .unwrap(),
        folders.clone().upcast::<gtk::Widget>()
    );
    assert!(local_row.subtitle().unwrap().starts_with("Local only"));
    assert!(descendant(local_row.upcast_ref(), &|widget| widget
        .is::<gtk::CheckButton>())
    .is_none());
    let mut ancestor = again.parent();
    while let Some(widget) = ancestor {
        if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let adjustment = scroll.vadjustment();
            adjustment.set_value(adjustment.upper());
        }
        ancestor = widget.parent();
    }
    capture(&d, "sync-local-only-folder");
    again.emit_clicked();
    settle();
    let confirm = d
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(confirm.body().contains("new note identities"));
    confirm.emit_by_name::<()>("response", &[&"cancel"]);
    confirm.close();
    settle();
    assert!(d
        .sync
        .snapshot()
        .unwrap()
        .unwrap()
        .bindings
        .contains_key(&folder_id));
    alert.emit_by_name::<()>("response", &[&"save"]);
    config_dialog.close();
    settle();
    let saved = d.sync.saved_config().unwrap().unwrap();
    assert_eq!(saved.scopes, "openid unsaved-scope");
    assert!(saved.excluded_folders.contains(&folder_id));
    assert!(!saved.folders.contains(&a));
    assert!(saved.folders.contains(&b));
    // Closing a non-deleted synced folder pauses this device, even if an older
    // worker result tries to add its bookmark again.
    fs::write(b.join("keep.txt"), "local files remain").unwrap();
    let journal = fs::read(root.join("sync/state.json")).unwrap();
    d.close_folder(&b).unwrap();
    let paused = d.sync.saved_config().unwrap().unwrap();
    assert!(paused.excluded_folders.contains(&draft_id));
    assert!(!paused.selected_folders.contains(&draft_id));
    assert!(!paused.folders.contains(&b));
    assert!(!d.model.borrow().settings.note_folders.contains(&b));
    let baseline = d.model.borrow().settings.clone();
    let mut stale = baseline.clone();
    stale.add_folder(b.clone(), false);
    stale
        .folder_labels
        .insert(b.clone(), "Late remote label".into());
    let (tx, rx) = std::sync::mpsc::channel();
    *d.sync_job.borrow_mut() = Some(rx);
    tx.send(Ok(papertrail_sync::control::Outcome {
        message: "Late sync completion".into(),
        warning: None,
        recovery: None,
        synced: true,
        unlocked: false,
        settings_update: Some((Box::new(baseline), Box::new(stale))),
        local_changed: false,
    }))
    .unwrap();
    d.sync_poll();
    assert!(!d.model.borrow().settings.note_folders.contains(&b));
    assert!(!d.model.borrow().settings.folder_labels.contains_key(&b));
    assert_eq!(fs::read(root.join("sync/state.json")).unwrap(), journal);
    assert_eq!(
        fs::read_to_string(b.join("keep.txt")).unwrap(),
        "local files remain"
    );
    d.choose_active(Some(&b)).unwrap();
    assert!(d.model.borrow().settings.note_folders.contains(&b));
    assert!(
        !d.sync
            .folder_choices(&d.model.borrow().settings)
            .unwrap()
            .iter()
            .find(|choice| choice.id.as_ref() == Some(&draft_id))
            .unwrap()
            .selected
    );
    // Server-only markers and stale mappings are internal bookkeeping, not rows.
    {
        let mut engine =
            papertrail_sync::engine::Engine::open(root.join("sync/state.json")).unwrap();
        let catalog = engine.state.catalog.as_mut().unwrap();
        let mut data = catalog.folders[&folder_id].clone();
        data.name = "Server-only deleted folder".into();
        let remote_id = "BAQEBAQEBAQEBAQEBAQEBA".to_owned();
        catalog.folders.insert(remote_id.clone(), data.clone());
        catalog.deleted_folders.insert(remote_id);
        let missing_id = "BQUFBQUFBQUFBQUFBQUFBQ".to_owned();
        data.name = "Unavailable deleted folder".into();
        catalog.folders.insert(missing_id.clone(), data);
        catalog.deleted_folders.insert(missing_id.clone());
        engine.state.bindings.insert(
            missing_id,
            papertrail_sync::folders::Binding {
                path: root.join("unavailable"),
                managed: true,
                observed_name: "unavailable".into(),
                observed_color: None,
            },
        );
        engine.save().unwrap();
    }
    let journal = fs::read(root.join("sync/state.json")).unwrap();
    d.window
        .lookup_action("sync-configure")
        .unwrap()
        .activate(None);
    settle();
    let config_dialog = d.window.visible_dialog().unwrap();
    for title in [
        "Server-only deleted folder",
        "Unavailable deleted folder",
        "Removed from Sync",
    ] {
        assert!(descendant(config_dialog.upcast_ref(), &|widget| {
            widget
                .clone()
                .downcast::<adw::ActionRow>()
                .is_ok_and(|row| row.title() == title)
                || widget
                    .clone()
                    .downcast::<adw::PreferencesGroup>()
                    .is_ok_and(|group| group.title() == title)
        })
        .is_none());
    }
    config_dialog.close();
    settle();
    assert_eq!(fs::read(root.join("sync/state.json")).unwrap(), journal);
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
    // Keep GTK initialization on one test thread, but allow sync regressions
    // to run without compositor-dependent editor focus/input checks.
    if std::env::var("PAPERTRAIL_TEST_SCOPE").as_deref() == Ok("sync") {
        remembered_password_dialogs(&app, &temp.path().join("remembered-password"));
        successful_login_prompts_for_unlock_only_when_needed(
            &app,
            &temp.path().join("login-unlock"),
        );
        background_sync_is_non_intrusive(&app, &temp.path().join("background-sync"));
        sync_configuration_prefills_and_preserves_multiple_selected_folders(
            &app,
            &temp.path().join("configure-folders"),
        );
        sync_discovery_trust_is_explicit(&app, &temp.path().join("discovery-trust"));
        assert_no_application_diagnostics(&d, &diagnostics);
        return;
    }
    for (name, _, _) in crate::actions::SHORTCUTS {
        assert!(d.window.lookup_action(name).is_some(), "missing {name}");
    }
    for name in [
        "sync-configure",
        "sync-login",
        "sync-create",
        "sync-unlock",
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
    remembered_password_dialogs(&app, &temp.path().join("remembered-password"));
    successful_login_prompts_for_unlock_only_when_needed(&app, &temp.path().join("login-unlock"));
    background_sync_is_non_intrusive(&app, &temp.path().join("background-sync"));
    sync_configuration_prefills_and_preserves_multiple_selected_folders(
        &app,
        &temp.path().join("configure-folders"),
    );
    sync_discovery_trust_is_explicit(&app, &temp.path().join("discovery-trust"));
    transient_dialog_regressions(&app, &temp.path().join("transient-dialogs"));
    assert_no_application_diagnostics(&d, &diagnostics);
}
