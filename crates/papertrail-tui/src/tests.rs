use super::*;
use crate::{
    app::{Focus, PromptKind},
    config::Paths,
    editor::Mode,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use std::{fs, path::Path};

fn app(root: &Path) -> App {
    App::new(Paths {
        home: root.to_owned(),
        settings: root.join("config/settings.json"),
        default_notes: root.join("notes"),
        folder: None,
        no_mouse: false,
    })
    .unwrap()
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

#[test]
fn sync_secret_prompts_are_masked_and_never_completed_or_written_to_settings() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    assert!(app
        .command("unlock never-put-secrets-in-arguments")
        .is_err());
    app.command("unlock").unwrap();
    type_text(&mut app, "private vault passphrase");
    key(&mut app, KeyCode::Tab);
    assert!(!app.prompt.as_ref().unwrap().completion.active);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!screen.contains("private vault passphrase"));
    assert!(screen.contains("Vault passphrase"));
    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none());
    let settings = fs::read_to_string(temp.path().join("config/sync/config.json")).unwrap();
    assert!(!settings.contains("private vault passphrase"));
}

#[test]
fn sync_folder_selection_is_explicit_and_account_edits_preserve_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://issuer.example native 'openid offline_access audience'").unwrap();
    let first = app.settings.notes_dir.clone();
    let other = temp.path().join("other folder");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("keep.txt"), "keep").unwrap();
    app.command(&format!("folder {}", other.display())).unwrap();
    app.command(&format!("sync-add-folder {}", other.display()))
        .unwrap();
    app.command(&format!("sync-remove-folder {}", first.display()))
        .unwrap();
    app.command("sync-configure https://sync.example https://issuer.example native 'openid offline_access updated'").unwrap();
    let config = app.sync.saved_config().unwrap().unwrap();
    assert_eq!(config.folders, vec![fs::canonicalize(&other).unwrap()]);
    assert!(config.download_dir.is_some());
    assert_eq!(config.scopes, "openid offline_access updated");
    app.command("sync-folders").unwrap();
    assert!(app.path_details.as_ref().unwrap().contains("other folder"));
    assert_eq!(fs::read_to_string(other.join("keep.txt")).unwrap(), "keep");
}

#[cfg(unix)]
#[test]
fn sync_selection_recognizes_directory_aliases_without_duplicate_choices() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let mut app = app(&alias);
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    let first = app.settings.notes_dir.clone();
    let canonical = fs::canonicalize(&first).unwrap();
    app.command(&format!("sync-add-folder {}", canonical.display()))
        .unwrap();
    assert_eq!(app.sync.saved_config().unwrap().unwrap().folders.len(), 1);
    let choices = app.sync.folder_choices(&app.settings).unwrap();
    assert_eq!(choices.len(), 1);
    assert!(choices[0].selected);
    app.command(&format!("sync-remove-folder {}", canonical.display()))
        .unwrap();
    assert!(app.sync.saved_config().unwrap().unwrap().folders.is_empty());
    let choices = app.sync.folder_choices(&app.settings).unwrap();
    assert_eq!(choices.len(), 1);
    assert!(!choices[0].selected);
    assert!(first.is_dir());
}

#[test]
fn login_defaults_to_device_flow_without_invoking_a_browser() {
    // No sync configuration: the worker exits before any network request. The
    // command must nevertheless select device flow, not the browser variant.
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("login").unwrap();
    assert!(app.device_cancel.is_some());
    assert!(app.device_notifications.is_some());
    assert!(app.status.contains("device login code"));
    app.command("login-cancel").unwrap();
    assert!(app.device_cancel.as_ref().unwrap().is_cancelled());
}

#[test]
fn device_login_banner_cancellation_and_completion_preserve_edits() {
    use papertrail_sync::{
        auth::device::{Cancellation, Prompt},
        control::Outcome,
    };
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new ssh.txt").unwrap();
    type_text(&mut app, "before");
    app.flush().unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<std::io::Result<Outcome>>();
    let (prompt_tx, prompt_rx) = std::sync::mpsc::channel();
    let cancel = Cancellation::default();
    app.sync_job = Some(result_rx);
    app.device_cancel = Some(cancel.clone());
    app.device_notifications = Some(prompt_rx);
    let prompt = || Prompt {
        verification_uri: "https://issuer.example/activate".into(),
        user_code: zeroize::Zeroizing::new("ABCD-EFGH".into()),
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(300),
    };
    prompt_tx.send(prompt()).unwrap();
    app.tick().unwrap();
    assert!(app.device_prompt.is_some());
    type_text(&mut app, " during login");
    app.flush().unwrap();
    assert_eq!(app.session.text, "before during login");
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("ABCD-EFGH"));
    assert!(text.contains("https://issuer.example/activate"));
    assert!(!text.contains("PRIVATE_DEVICE_CODE"));
    app.command("login-show").unwrap();
    assert!(app.device_details);
    key(&mut app, KeyCode::Esc);
    assert!(!app.device_details);
    assert!(!cancel.is_cancelled());
    app.command("login-cancel").unwrap();
    assert!(cancel.is_cancelled());
    assert!(app.device_prompt.is_none());
    prompt_tx.send(prompt()).unwrap();
    app.tick().unwrap();
    assert!(
        app.device_prompt.is_none(),
        "a late progress notification revived a cancelled code"
    );
    type_text(&mut app, " still editing");
    result_tx
        .send(Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "cancelled",
        )))
        .unwrap();
    app.tick().unwrap();
    assert!(app.sync_job.is_none());
    assert!(app.device_cancel.is_none());
    assert_eq!(app.session.text, "before during login still editing");
    assert!(app.session.is_dirty());
    app.flush().unwrap();
    assert_eq!(
        fs::read_to_string(app.current_path().unwrap()).unwrap(),
        app.session.text
    );
    app.persist().unwrap();
    assert!(
        !fs::read_to_string(temp.path().join("config/settings.json"))
            .unwrap()
            .contains("ABCD-EFGH")
    );
}

#[test]
fn completing_device_login_does_not_reload_the_editor_or_unlock_the_vault() {
    use papertrail_sync::{
        auth::device::{Cancellation, Prompt},
        control::Outcome,
    };
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new ssh.txt").unwrap();
    type_text(&mut app, "unsaved local text");
    let (tx, rx) = std::sync::mpsc::channel();
    app.sync_job = Some(rx);
    app.device_cancel = Some(Cancellation::default());
    app.device_prompt = Some(Prompt {
        verification_uri: "https://issuer.example/activate".into(),
        user_code: zeroize::Zeroizing::new("DONE-CODE".into()),
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(300),
    });
    tx.send(Ok(Outcome {
        message: "Signed in for this session".into(),
        recovery: None,
        synced: false,
        unlocked: false,
        settings_update: None,
        local_changed: false,
    }))
    .unwrap();
    app.tick().unwrap();
    assert!(app.device_prompt.is_none());
    assert!(!app.sync_unlocked);
    assert_eq!(app.session.text, "unsaved local text");
    assert!(app.session.is_dirty());
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn lock_requested_during_device_login_survives_a_failed_local_save() {
    use papertrail_sync::{auth::device::Cancellation, control::Outcome};
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new missing.txt").unwrap();
    let path = app.current_path().unwrap();
    fs::remove_file(path).unwrap();
    type_text(&mut app, "keep this unsaved text");
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<Outcome>>();
    let cancel = Cancellation::default();
    app.sync_job = Some(rx);
    app.device_cancel = Some(cancel.clone());
    app.command("lock").unwrap();
    assert!(cancel.is_cancelled());
    tx.send(Err(std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "cancelled",
    )))
    .unwrap();
    app.tick().unwrap();
    assert!(
        app.sync_job.is_some(),
        "lock was not queued independently of the failed save"
    );
    assert_eq!(app.session.text, "keep this unsaved text");
    assert!(app.session.is_dirty());
}

#[test]
fn vim_edit_autosave_undo_visual_and_safe_quit() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new hello.txt").unwrap();
    assert_eq!(app.editor.mode, Mode::Insert);
    type_text(&mut app, "Hello λ");
    key(&mut app, KeyCode::Enter);
    type_text(&mut app, "World");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.session.text, "Hello λ\nWorld");
    app.flush().unwrap();
    let path = app.current_path().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), app.session.text);
    key(&mut app, KeyCode::Char('0'));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.session.text, "Hello λ\nWorld");
    key(&mut app, KeyCode::Char('0'));
    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(app.editor.mode, Mode::Normal);
    key(&mut app, KeyCode::Char('p'));
    assert!(app.session.is_dirty());
    app.command("wq").unwrap();
    assert!(app.quit);
    assert_eq!(fs::read_to_string(path).unwrap(), app.session.text);
}

#[test]
fn full_note_workflow_preserves_metadata_and_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new first.txt").unwrap();
    type_text(&mut app, "A searchable body");
    app.command("pin").unwrap();
    app.command("rename renamed.txt").unwrap();
    let old = app.current_path().unwrap();
    assert!(app.settings.pinned_notes.contains(&old));
    let other = temp.path().join("other folder");
    fs::create_dir(&other).unwrap();
    app.command(&format!("move {}", other.display())).unwrap();
    let moved = app.current_path().unwrap();
    assert!(!old.exists());
    assert!(app.settings.pinned_notes.contains(&moved));
    app.command("color #aabbcc").unwrap();
    assert_eq!(app.settings.folder_colors[&other], "#aabbcc");
    app.command("all").unwrap();
    app.command("search SEARCHABLE").unwrap();
    assert_eq!(app.notes.len(), 1);
    app.command("replace 'searchable body' 'changed text'")
        .unwrap();
    app.flush().unwrap();
    assert_eq!(fs::read_to_string(&moved).unwrap(), "A changed text");
    app.command("delete").unwrap();
    assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::Delete);
    key(&mut app, KeyCode::Esc);
    assert!(moved.exists());
    app.command("delete").unwrap();
    type_text(&mut app, "yes");
    key(&mut app, KeyCode::Enter);
    assert!(!moved.exists());
    assert!(!app.settings.pinned_notes.contains(&moved));
}

#[test]
fn external_changes_block_save_navigation_and_quit_without_losing_edits() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new conflict.txt").unwrap();
    type_text(&mut app, "Original");
    app.flush().unwrap();
    let path = app.current_path().unwrap();
    type_text(&mut app, " locally edited");
    fs::write(&path, "Changed by desktop").unwrap();
    assert!(app.command("w").is_err());
    assert!(app.command("q").is_err());
    assert!(app.command("new").is_err());
    assert!(!app.quit);
    assert_eq!(app.session.text, "Original locally edited");
    assert_eq!(fs::read_to_string(&path).unwrap(), "Changed by desktop");
    let copy = temp.path().join("recovered.txt");
    app.command(&format!("saveas {}", copy.display())).unwrap();
    assert_eq!(fs::read_to_string(copy).unwrap(), "Original locally edited");
    assert!(!app.session.is_dirty());
}

#[test]
fn missing_bookmark_recovers_and_external_editor_reload_is_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    let missing = temp.path().join("doc/missing/notes");
    app.settings.add_folder(missing.clone(), true);
    app.persist().unwrap();
    let mut app = self::app(temp.path());
    assert!(!missing.exists());
    assert!(app.settings.note_folders.contains(&missing));
    assert_eq!(app.settings.notes_dir, temp.path().join("notes"));
    assert!(app
        .command(&format!("folder {}", missing.display()))
        .is_err());
    app.command("new external.txt").unwrap();
    type_text(&mut app, "before");
    app.command("edit").unwrap();
    assert!(app.external);
    assert!(!app.session.is_dirty());
    fs::write(app.current_path().unwrap(), "after external editor").unwrap();
    app.reload(false).unwrap();
    assert_eq!(app.editor.text(), "after external editor");
}

#[test]
fn headless_rendering_and_prompts_work_at_various_sizes() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new render.txt").unwrap();
    type_text(&mut app, "Rendered note");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Folders);
    for (width, height) in [(100, 30), (55, 10), (20, 5)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!content.trim().is_empty());
    }
    key(&mut app, KeyCode::Char('?'));
    assert!(app.help);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.help_scroll, 1);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "all");
    key(&mut app, KeyCode::Enter);
    assert!(app.all);
}

#[test]
fn preferences_do_not_change_desktop_layout_and_saveas_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new preferences.txt").unwrap();
    type_text(&mut app, "text");
    app.persist().unwrap();
    let file = temp.path().join("config/settings.json");
    let before = fs::read(&file).unwrap();
    app.command("set nonumber").unwrap();
    assert_eq!(fs::read(&file).unwrap(), before);
    assert!(temp.path().join("config/tui.json").exists());
    let original = app.current_path().unwrap();
    assert!(app
        .command(&format!("saveas {}", original.display()))
        .is_err());
    assert!(app.session.is_dirty());
}

#[test]
fn conflict_pauses_autosave_until_a_new_copy_is_saved() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new paused.txt").unwrap();
    type_text(&mut app, "local text");
    fs::write(app.current_path().unwrap(), "external text").unwrap();
    app.session.edit(
        "local text".into(),
        std::time::Instant::now() - std::time::Duration::from_secs(1),
    );
    assert!(app.tick().is_err());
    assert!(app.autosave_paused);
    assert!(app.tick().is_ok());
    let copy = temp.path().join("new-copy.txt");
    app.command(&format!("saveas {}", copy.display())).unwrap();
    assert!(!app.autosave_paused);
    assert_eq!(fs::read_to_string(copy).unwrap(), "local text");
}

#[test]
fn literal_find_replace_and_line_deletion_support_unicode_and_undo() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new unicode.txt").unwrap();
    type_text(&mut app, "a.b λ");
    key(&mut app, KeyCode::Enter);
    type_text(&mut app, "last");
    key(&mut app, KeyCode::Esc);
    assert!(app.editor.find("a.b", true));
    assert!(!app.editor.find("a*b", true));
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.session.text, "a.b λ");
    app.command("replace 'a.b' '✨'").unwrap();
    assert_eq!(app.session.text, "✨ λ");
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.session.text, "a.b λ");
}

#[test]
fn folder_commands_keep_last_accessible_folder_and_reorder_bookmarks() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    assert!(app.command("close-folder").is_err());
    assert!(app.command("folder 0").is_err());
    let other = temp.path().join("second");
    fs::create_dir(&other).unwrap();
    app.command(&format!("folder {}", other.display())).unwrap();
    app.command("folder-up").unwrap();
    assert_eq!(app.settings.note_folders[0], other.canonicalize().unwrap());
    app.command("close-folder").unwrap();
    assert!(other.is_dir());
    assert_eq!(app.settings.note_folders, [temp.path().join("notes")]);
}

#[test]
fn bracketed_paste_is_one_undo_step_and_never_executes_commands() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new paste.txt").unwrap();
    app.paste("first\nλ");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.session.text, "");
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.session.text, "first\nλ");
    key(&mut app, KeyCode::Char(':'));
    app.paste("q!\r\n");
    assert!(!app.quit);
    assert_eq!(app.prompt.as_ref().unwrap().text, "q!");
    key(&mut app, KeyCode::Esc);
}

fn render_app(app: &mut App, width: u16, height: u16) {
    Terminal::new(TestBackend::new(width, height))
        .unwrap()
        .draw(|frame| ui::render(frame, app))
        .unwrap();
}

fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn browse_app(root: &Path, count: usize) -> App {
    fs::create_dir(root.join("notes")).unwrap();
    for index in 0..count {
        fs::write(
            root.join(format!("notes/{index:02}.txt")),
            format!("Note {index}\nBody {index}"),
        )
        .unwrap();
    }
    app(root)
}

#[test]
fn highlighted_notes_preview_on_start_and_navigation_without_stealing_focus() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 3);
    assert_eq!(app.focus, Focus::Notes);
    assert_eq!(app.current_path().unwrap(), app.notes[0].path);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.current_path().unwrap(), app.notes[1].path);
    assert_eq!(app.focus, Focus::Notes);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('i'));
    type_text(&mut app, "HL");
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.session.text.starts_with("HL"));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('H'));
    assert_eq!(app.focus, Focus::Notes);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.current_path().unwrap(), app.notes[2].path);
    assert!(fs::read_to_string(&app.notes[1].path)
        .unwrap()
        .starts_with("HL"));
    key(&mut app, KeyCode::Char('L'));
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn preview_failure_keeps_keyboard_and_mouse_selection_and_unsaved_text() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 3);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('i'));
    type_text(&mut app, "LOCAL ");
    let text = app.session.text.clone();
    let path = app.current_path().unwrap();
    fs::write(&path, "external change").unwrap();
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('H'));
    assert!(app
        .key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .is_err());
    assert_eq!(app.note_index, 0);
    assert_eq!(app.current_path().unwrap(), path);
    assert_eq!(app.session.text, text);
    render_app(&mut app, 100, 25);
    let notes = app.panes[1];
    assert!(app
        .mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            notes.x + 2,
            notes.y + 3
        ))
        .is_err());
    assert_eq!(app.note_index, 0);
    assert_eq!(app.session.text, text);
    assert_eq!(fs::read_to_string(path).unwrap(), "external change");
}

#[test]
fn narrow_configurable_folder_pane_and_path_dialog_preserve_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    render_app(&mut app, 120, 25);
    assert_eq!(app.panes[0].width, 16);
    app.command("set folder-width 14").unwrap();
    render_app(&mut app, 120, 25);
    assert_eq!(app.panes[0].width, 14);
    assert!(app.command("set folder-width 2").is_err());
    app.focus = Focus::Folders;
    render_app(&mut app, 65, 25);
    assert_eq!(app.panes[0].width, 0);
    assert_eq!(app.focus, Focus::Notes);
    key(&mut app, KeyCode::Char('H'));
    assert_eq!(app.focus, Focus::Editor);
    key(&mut app, KeyCode::Char('L'));
    assert_eq!(app.focus, Focus::Notes);
    app.command("path").unwrap();
    assert!(app.help);
    assert!(app.path_details.as_ref().unwrap().contains(".txt"));
    key(&mut app, KeyCode::Esc);
    assert!(app.path_details.is_none());
    let reloaded = self::app(temp.path());
    assert_eq!(reloaded.preferences.folder_width, 14);
}

#[test]
fn mouse_clicks_and_scrolling_use_persistent_list_offsets_and_respect_no_mouse() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 30);
    render_app(&mut app, 100, 16);
    let notes = app.panes[1];
    for _ in 0..5 {
        app.mouse(mouse(MouseEventKind::ScrollDown, notes.x + 2, notes.y + 2))
            .unwrap();
        render_app(&mut app, 100, 16);
    }
    assert!(app.note_state.offset() > 0);
    let index = app.note_state.offset() + 1;
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        notes.x + 2,
        notes.y + 3,
    ))
    .unwrap();
    assert_eq!(app.note_index, index);
    assert_eq!(app.current_path().unwrap(), app.notes[index].path);
    assert_eq!(app.focus, Focus::Notes);
    app.mouse_enabled = false;
    app.mouse(mouse(MouseEventKind::ScrollDown, notes.x + 2, notes.y + 2))
        .unwrap();
    assert_eq!(app.note_index, index);
    let parsed = Paths::discover(["--no-mouse".to_string()]).unwrap();
    assert!(parsed.no_mouse);
}

#[test]
fn editor_mouse_positions_account_for_numbers_unicode_tabs_and_scrolling() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new mouse.txt").unwrap();
    app.editor.set_text(
        "abc界de\tz\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\neighth\nninth\ntenth\nlast",
    );
    render_app(&mut app, 110, 12);
    let pane = app.panes[2];
    // Eleven lines give a four-cell number gutter. Column four lies inside 界.
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1 + 4 + 4,
        pane.y + 1,
    ))
    .unwrap();
    assert_eq!(app.editor.area.cursor(), (0, 3));
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1 + 4 + 8,
        pane.y + 1,
    ))
    .unwrap();
    assert_eq!(app.editor.area.cursor(), (0, 7));
    app.mouse(mouse(MouseEventKind::ScrollDown, pane.x + 2, pane.y + 2))
        .unwrap();
    render_app(&mut app, 110, 12);
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1 + 4,
        pane.y + 1,
    ))
    .unwrap();
    assert_eq!(app.editor.area.cursor(), (3, 0));
    assert!(!app.session.is_dirty());
}

#[test]
fn mouse_editor_click_uses_horizontal_scroll_position() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("new wide.txt").unwrap();
    app.editor.set_text(&"0123456789".repeat(30));
    app.editor.area.move_cursor(tui_textarea::CursorMove::End);
    render_app(&mut app, 110, 20);
    let pane = app.panes[2];
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1,
        pane.y + 1,
    ))
    .unwrap();
    assert_eq!(
        app.editor.area.cursor(),
        (0, 301 - usize::from(pane.width - 2))
    );
}

#[test]
fn mouse_folder_activation_keeps_folder_focus_but_enter_moves_to_notes() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    let other = temp.path().join("other folder");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("other.txt"), "Other folder preview").unwrap();
    app.settings.add_folder(other.clone(), false);
    render_app(&mut app, 110, 25);
    let folders = app.panes[0];
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        folders.x + 2,
        folders.y + 2,
    ))
    .unwrap();
    assert_eq!(app.focus, Focus::Folders);
    assert_eq!(app.folder_index, 1);
    assert_eq!(app.settings.notes_dir, other);
    assert_eq!(app.session.text, "Other folder preview");
    key(&mut app, KeyCode::Up);
    assert_eq!(app.focus, Focus::Folders);
    assert_eq!(app.folder_index, 0);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Notes);
    assert_eq!(app.settings.notes_dir, temp.path().join("notes"));
}

#[test]
fn mouse_folder_activation_does_not_switch_on_save_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    app.settings.add_folder(other, false);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('i'));
    app.paste("local text");
    let text = app.session.text.clone();
    fs::write(app.current_path().unwrap(), "external change").unwrap();
    render_app(&mut app, 110, 25);
    let folders = app.panes[0];
    assert!(app
        .mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x + 2,
            folders.y + 2
        ))
        .is_err());
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.folder_index, 0);
    assert_eq!(app.session.text, text);
    assert_eq!(app.settings.notes_dir, temp.path().join("notes"));
}

#[test]
fn command_completion_cycles_both_directions_and_never_executes() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    key(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "re");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.prompt.as_ref().unwrap().text, "refresh");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.prompt.as_ref().unwrap().text, "reload");
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.prompt.as_ref().unwrap().text, "refresh");
    assert_eq!(app.focus, Focus::Notes);
    render_app(&mut app, 70, 15);
    key(&mut app, KeyCode::Backspace);
    assert!(!app.prompt.as_ref().unwrap().completion.active);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "q");
    key(&mut app, KeyCode::Tab);
    assert!(!app.quit);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn completion_handles_options_colors_numbers_and_no_matches() {
    use crate::completion::candidates;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert_eq!(
        candidates("set non", 2, root, root).unwrap(),
        ["set nonumber"]
    );
    assert_eq!(
        candidates("set folder-width 3", 2, root, root).unwrap(),
        ["set folder-width 30"]
    );
    assert_eq!(
        candidates("color preset-11", 2, root, root).unwrap(),
        ["color preset-11"]
    );
    assert_eq!(candidates("move 2", 3, root, root).unwrap(), ["move 2"]);
    assert!(candidates("not-a-command", 0, root, root)
        .unwrap()
        .is_empty());
    let mut app = app(root);
    key(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "not-a-command");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.prompt.as_ref().unwrap().text, "not-a-command");
    render_app(&mut app, 80, 15);
    app.paste("x");
    assert!(!app.prompt.as_ref().unwrap().completion.active);
}

#[test]
fn path_completion_supports_spaces_unicode_home_and_unique_directory_descent() {
    use crate::completion::{candidates, Completion};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("folder λ with spaces")).unwrap();
    fs::create_dir(root.join("folder λ with spaces/child")).unwrap();
    fs::write(root.join("file.txt"), "keep").unwrap();
    assert_eq!(
        candidates("folder fol", 0, root, root).unwrap(),
        ["folder folder λ with spaces/"]
    );
    assert!(candidates("move file", 0, root, root).unwrap().is_empty());
    assert_eq!(
        candidates("saveas file", 0, root, root).unwrap(),
        ["saveas file.txt"]
    );
    assert_eq!(
        candidates("folder ~/fol", 0, root, root).unwrap(),
        ["folder ~/folder λ with spaces/"]
    );
    let mut state = Completion::default();
    let mut text = "folder fol".to_owned();
    state.cycle(&mut text, false, 0, root, root).unwrap();
    state.cycle(&mut text, false, 0, root, root).unwrap();
    assert_eq!(text, "folder folder λ with spaces/child/");
    assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "keep");
}
