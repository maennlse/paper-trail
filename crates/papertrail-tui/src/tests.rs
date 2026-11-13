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
fn discovered_issuer_requires_confirmation_and_persists_only_on_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    let config = papertrail_sync::Config {
        server: "https://sync.example".into(),
        issuer: "https://issuer.example".into(),
        client_id: "native".into(),
        scopes: "openid offline_access".into(),
        discover_auth: true,
        folders: vec![app.settings.notes_dir.clone()],
        ..papertrail_sync::Config::default()
    };
    app.confirm_discovered(config.clone(), None).unwrap();
    assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::ConfirmIssuer);
    assert!(!app.sync.configured());
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("https://sync.example"));
    assert!(screen.contains("https://issuer.example"));
    key(&mut app, KeyCode::Esc);
    assert!(app.pending_auth.is_none());
    assert!(!app.sync.configured());
    app.confirm_discovered(config.clone(), None).unwrap();
    type_text(&mut app, "no");
    key(&mut app, KeyCode::Enter);
    assert!(!app.sync.configured());
    app.confirm_discovered(config, None).unwrap();
    type_text(&mut app, "yes");
    key(&mut app, KeyCode::Enter);
    let saved = app.sync.saved_config().unwrap().unwrap();
    assert!(saved.discover_auth);
    assert_eq!(saved.issuer, "https://issuer.example");
    app.confirm_discovered(saved.clone(), Some(saved.clone()))
        .unwrap();
    assert!(app.prompt.is_none()); // unchanged trusted issuer continues automatically
    let mut changed = saved.clone();
    changed.issuer = "https://new-issuer.example".into();
    app.confirm_discovered(changed.clone(), Some(saved.clone()))
        .unwrap();
    assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::ConfirmIssuer);
    type_text(&mut app, "no");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.sync.saved_config().unwrap().unwrap().issuer,
        saved.issuer
    );
    app.confirm_discovered(changed, Some(saved)).unwrap();
    type_text(&mut app, "yes");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.sync.saved_config().unwrap().unwrap().issuer,
        "https://new-issuer.example"
    );
}

#[test]
fn stale_discovery_confirmation_cannot_overwrite_manual_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://manual.example manual")
        .unwrap();
    let previous = app.sync.saved_config().unwrap().unwrap();
    assert!(!previous.discover_auth);
    let mut changed = previous.clone();
    changed.issuer = "https://discovered.example".into();
    changed.discover_auth = true;
    app.confirm_discovered(changed, Some(previous)).unwrap();
    // Another frontend changes this shared profile while the confirmation is open.
    let mut manual = app.sync.saved_config().unwrap().unwrap();
    manual.scopes = "openid manual:scope".into();
    app.sync.configure(manual.clone()).unwrap();
    type_text(&mut app, "yes");
    assert!(app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .is_err());
    assert!(app.sync.saved_config().unwrap().unwrap() == manual);
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
    assert!(app
        .sync_folders
        .as_ref()
        .unwrap()
        .choices
        .iter()
        .any(|choice| choice.name == "other folder" && choice.selected));
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

fn neovim_app(root: &Path, count: usize) -> Option<App> {
    if !crate::neovim::tests::available() {
        return None;
    }
    let mut app = browse_app(root, count);
    app.command("set nvim-config clean").unwrap();
    app.command("set editor neovim").unwrap();
    key(&mut app, KeyCode::Enter);
    Some(app)
}

fn wait_neovim(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.poll_neovim().unwrap();
        if predicate(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "Neovim did not reach the expected state"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn embedded_neovim_owns_colon_tab_and_motions_but_keeps_app_shortcuts() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    key(&mut app, KeyCode::Char(':'));
    assert!(app.prompt.is_none(), "Neovim owns its command line");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('i'));
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('X'));
    key(&mut app, KeyCode::Esc);
    wait_neovim(&mut app, |app| app.session.text.starts_with("\tX"));
    key(&mut app, KeyCode::Char('H'));
    key(&mut app, KeyCode::Char('L'));
    assert_eq!(app.focus, Focus::Editor);
    key(&mut app, KeyCode::F(6));
    assert_eq!(app.focus, Focus::Folders);
    key(&mut app, KeyCode::F(2));
    assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::Command);
    assert!(app.session.is_dirty());
}

#[test]
fn neovim_autosave_preserves_insert_mode_and_native_write_uses_the_host() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    let path = app.current_path().unwrap();
    app.neovim.as_mut().unwrap().input("A added").unwrap();
    wait_neovim(&mut app, |app| {
        app.session.text.contains("added") && app.neovim.as_ref().unwrap().grid.mode == "insert"
    });
    let saved = app.session.text.clone();
    app.session.edit(
        saved.clone(),
        std::time::Instant::now() - std::time::Duration::from_secs(1),
    );
    app.tick().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), saved);
    key(&mut app, KeyCode::Char('!'));
    wait_neovim(&mut app, |app| app.session.text.contains("added!"));
    app.neovim.as_mut().unwrap().input("<Esc>:w<CR>").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fs::read_to_string(&path).unwrap() != app.session.text {
        app.poll_neovim().unwrap();
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.session.is_dirty());
    assert!(app.neovim.is_some());
}

#[test]
fn neovim_write_cannot_override_external_edits_and_saveas_keeps_recovery_text() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    let path = app.current_path().unwrap();
    app.neovim
        .as_mut()
        .unwrap()
        .input("A locally changed<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| app.session.text.contains("locally changed"));
    let recovery = app.session.text.clone();
    fs::write(&path, "External change must survive").unwrap();
    app.neovim.as_mut().unwrap().input(":wq<CR>").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if app.poll_neovim().is_err() {
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.autosave_paused);
    assert!(
        app.neovim.is_some(),
        "A failed :wq must not close the editor"
    );
    assert!(app.session.is_dirty());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "External change must survive"
    );
    assert_eq!(app.session.text, recovery);
    let copy = root.path().join("recovered.txt");
    app.command(&format!("saveas {}", copy.display())).unwrap();
    assert_eq!(fs::read_to_string(&copy).unwrap(), recovery);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "External change must survive"
    );
    assert_eq!(app.neovim.as_ref().unwrap().path.as_ref(), Some(&copy));
}

#[test]
fn switching_notes_and_reloading_neovim_cannot_apply_old_buffer_events() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 2) else {
        return;
    };
    let first = app.current_path().unwrap();
    app.neovim
        .as_mut()
        .unwrap()
        .input("A saved before switch<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| {
        app.session.text.contains("saved before switch")
    });
    let edited = app.session.text.clone();
    app.select_note(1).unwrap();
    assert_ne!(app.current_path().unwrap(), first);
    assert_eq!(fs::read_to_string(first).unwrap(), edited);
    let second = app.current_path().unwrap();
    fs::write(&second, "Reloaded from disk").unwrap();
    app.command("reload").unwrap();
    for _ in 0..3 {
        app.poll_neovim().unwrap();
    }
    assert_eq!(app.session.text, "Reloaded from disk");
    app.command("set editor builtin").unwrap();
    assert!(app.neovim.is_none());
    assert_eq!(app.editor.text(), "Reloaded from disk");
}

#[test]
fn neovim_exit_retains_observed_unsaved_text_and_restarts_on_enter() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    app.neovim
        .as_mut()
        .unwrap()
        .input("A retain this<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| app.session.text.contains("retain this"));
    let text = app.session.text.clone();
    app.neovim.as_mut().unwrap().input(":qa!<CR>").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.neovim.is_some() {
        let _ = app.poll_neovim();
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.session.text, text);
    assert_eq!(app.editor.text(), text);
    assert!(app.autosave_paused);
    assert_eq!(app.focus, Focus::Notes);
    key(&mut app, KeyCode::Enter);
    assert!(app.neovim.is_some());
    assert_eq!(app.session.text, text);
}

#[test]
fn managed_neovim_note_binding_follows_host_rename_and_move() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    app.command("rename renamed note.txt").unwrap();
    assert_eq!(
        app.neovim.as_ref().unwrap().path,
        app.session.current.as_ref().map(|note| note.path.clone())
    );
    app.neovim
        .as_mut()
        .unwrap()
        .input("A after rename<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| app.session.text.contains("after rename"));
    let folder = root.path().join("moved");
    fs::create_dir(&folder).unwrap();
    app.command(&format!("move {}", folder.display())).unwrap();
    assert_eq!(
        app.neovim.as_ref().unwrap().path,
        app.session.current.as_ref().map(|note| note.path.clone())
    );
    app.neovim
        .as_mut()
        .unwrap()
        .input("A after move<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| app.session.text.contains("after move"));
    app.command("w").unwrap();
    assert_eq!(
        fs::read_to_string(folder.join("renamed note.txt")).unwrap(),
        app.session.text
    );
}

#[test]
fn neovim_explicit_write_checks_external_changes_even_without_buffer_edits() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    let path = app.current_path().unwrap();
    fs::write(&path, "External text").unwrap();
    app.neovim.as_mut().unwrap().input(":w!<CR>").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !app.autosave_paused {
        let _ = app.poll_neovim();
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(fs::read_to_string(path).unwrap(), "External text");
    assert!(app.session.is_dirty());
}

#[test]
fn host_secret_prompts_do_not_forward_input_to_neovim() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    let text = app.session.text.clone();
    app.command("unlock").unwrap();
    type_text(&mut app, "private vault passphrase");
    key(&mut app, KeyCode::Tab);
    app.poll_neovim().unwrap();
    assert_eq!(app.session.text, text);
    assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::Unlock);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!screen.contains("private vault passphrase"));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.neovim.as_mut().unwrap().snapshot().unwrap(), text);
}

#[test]
fn stalled_neovim_falls_back_without_losing_observed_text_or_blocking_recovery() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    app.neovim
        .as_mut()
        .unwrap()
        .input("A retained<Esc>")
        .unwrap();
    wait_neovim(&mut app, |app| app.session.text.contains("retained"));
    let text = app.session.text.clone();
    app.neovim
        .as_mut()
        .unwrap()
        .input(":lua while true do end<CR>")
        .unwrap();
    let error = app.command("set editor builtin").unwrap_err();
    assert!(error.to_string().contains("retained"));
    assert!(app.neovim.is_none());
    assert_eq!(app.session.text, text);
    assert_eq!(app.editor.text(), text);
    assert!(app.autosave_paused);
    app.command("set editor builtin").unwrap();
    assert_eq!(
        fs::read_to_string(app.current_path().unwrap()).unwrap(),
        text
    );
}

#[test]
fn native_wq_saves_through_the_host_and_returns_to_notes() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    let path = app.current_path().unwrap();
    app.neovim
        .as_mut()
        .unwrap()
        .input("A saved<Esc>:wq<CR>")
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.neovim.is_some() {
        let _ = app.poll_neovim();
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(fs::read_to_string(path).unwrap().contains("saved"));
    assert!(!app.session.is_dirty());
    assert_eq!(app.focus, Focus::Notes);
    assert!(!app.quit);
}

#[test]
fn legacy_tui_preferences_keep_builtin_editor_until_explicitly_selected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tui.json");
    fs::write(&path, r#"{"hide_numbers":true,"folder_width":20}"#).unwrap();
    let preferences = crate::config::Preferences::load(&path).unwrap();
    assert!(preferences.editor == crate::config::EditorBackend::Builtin);
    assert!(!preferences.nvim_clean);
    assert!(preferences.hide_numbers);
    assert_eq!(preferences.folder_width, 20);
}

#[test]
fn neovim_mouse_motion_does_not_steal_sidebar_focus() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 1) else {
        return;
    };
    app.focus = Focus::Notes;
    render_app(&mut app, 100, 24);
    let editor = app.panes[2];
    app.mouse(mouse(MouseEventKind::Moved, editor.x + 2, editor.y + 2))
        .unwrap();
    assert_eq!(app.focus, Focus::Notes);
    app.mouse(mouse(
        MouseEventKind::ScrollDown,
        editor.x + 2,
        editor.y + 2,
    ))
    .unwrap();
    assert_eq!(app.focus, Focus::Notes);
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        editor.x + 2,
        editor.y + 2,
    ))
    .unwrap();
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn neovim_requires_a_managed_note_before_accepting_paste() {
    let root = tempfile::tempdir().unwrap();
    let Some(mut app) = neovim_app(root.path(), 0) else {
        return;
    };
    app.focus = Focus::Editor;
    app.paste("must not become an untracked buffer");
    assert!(app.session.current.is_none());
    assert!(!app.session.is_dirty());
    assert_eq!(app.neovim.as_mut().unwrap().snapshot().unwrap(), "");
    app.command("new managed.txt").unwrap();
    app.paste("managed text");
    wait_neovim(&mut app, |app| app.session.text == "managed text");
}

#[test]
fn folder_colors_share_gtk_presets_custom_colors_and_readable_selection() {
    use ratatui::style::Color;
    for (index, hex) in papertrail_core::colors::PRESETS.iter().enumerate() {
        let rgb = u32::from_str_radix(&hex[1..], 16).unwrap();
        let color = Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        assert_eq!(ui::folder_color(&format!("preset-{index}")), color);
        assert_eq!(ui::selection_style(color).bg, Some(color));
    }
    assert_eq!(ui::folder_color("#abc"), Color::Rgb(170, 187, 204));
    assert_eq!(ui::folder_color("#aabbcc"), Color::Rgb(170, 187, 204));
    assert_eq!(ui::folder_color("invalid"), ui::folder_color("preset-0"));
    assert_eq!(
        ui::selection_style(Color::Rgb(0, 0, 0)).fg,
        Some(Color::Rgb(255, 255, 255))
    );
    assert_eq!(
        ui::selection_style(Color::Rgb(255, 255, 255)).fg,
        Some(Color::Rgb(0, 0, 0))
    );
}

#[test]
fn folder_badges_and_note_rows_use_the_same_rgb_colors_in_all_and_search_views() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    let first = app.settings.notes_dir.clone();
    let other = temp.path().join("Work notes");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("work.txt"), "Work note").unwrap();
    app.settings.add_folder(other.clone(), false);
    app.settings.folder_colors.insert(first, "preset-0".into());
    app.settings.folder_colors.insert(other, "#aabbcc".into());
    for command in ["all", "search note"] {
        app.command(command).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(110, 25)).unwrap();
        terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        for (index, note) in app.notes.iter().enumerate() {
            let color = ui::folder_color(&app.settings.folder_colors[note.path.parent().unwrap()]);
            let row = app.panes[1].y + 1 + index as u16 * 2;
            assert!((app.panes[1].x + 1..app.panes[1].right() - 1).any(|x| {
                let cell = buffer.cell((x, row)).unwrap();
                if index == app.note_index {
                    cell.bg == color
                } else {
                    cell.symbol() == "▎" && cell.fg == color
                }
            }));
        }
        app.focus = Focus::Folders;
        terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        for (index, note) in app.notes.iter().enumerate() {
            let color = ui::folder_color(&app.settings.folder_colors[note.path.parent().unwrap()]);
            let row = app.panes[1].y + 1 + index as u16 * 2;
            assert!((app.panes[1].x + 1..app.panes[1].right() - 1).any(|x| {
                let cell = buffer.cell((x, row)).unwrap();
                cell.symbol() == "▎" && cell.fg == color
            }));
        }
        let screen: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("[All]"));
        assert!(screen.contains("[WN]"));
        app.focus = Focus::Notes;
    }
}

#[test]
fn all_folder_row_supports_keyboard_mouse_and_unchanged_folder_numbers() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    let first = app.settings.notes_dir.clone();
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("other.txt"), "Other note").unwrap();
    app.settings.add_folder(other.clone(), false);
    app.focus = Focus::Folders;
    key(&mut app, KeyCode::Up);
    assert!(app.folder_all_selected);
    key(&mut app, KeyCode::Enter);
    assert!(app.all);
    assert_eq!(app.notes.len(), 2);
    assert!(app.command("folder-down").is_err());
    app.command("folder 2").unwrap();
    assert_eq!(app.settings.notes_dir, other);
    assert_eq!(app.folder_index, 1);
    assert!(!app.all);
    app.command("folder 1").unwrap();
    assert_eq!(app.settings.notes_dir, first);
    render_app(&mut app, 100, 25);
    let pane = app.panes[0];
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 2,
        pane.y + 1,
    ))
    .unwrap();
    assert!(app.all);
    assert!(app.folder_all_selected);
    assert_eq!(app.focus, Focus::Folders);
    app.command("path").unwrap();
    assert!(app.path_details.as_ref().unwrap().contains("All notes"));
}

#[test]
fn sync_folder_checklist_saves_or_cancels_without_touching_notes_or_editor() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    let first = app.settings.notes_dir.clone();
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("keep.txt"), "keep").unwrap();
    app.settings.add_folder(other.clone(), false);
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('i'));
    app.paste("Unsaved ");
    key(&mut app, KeyCode::Esc);
    let text = app.session.text.clone();
    let before = fs::read(temp.path().join("config/sync/config.json")).unwrap();
    app.command("sync-folders").unwrap();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    app.paste("must not reach editor or execute commands");
    assert_eq!(app.session.text, text);
    assert_eq!(
        fs::read(temp.path().join("config/sync/config.json")).unwrap(),
        before
    );
    key(&mut app, KeyCode::Esc);
    assert!(app.sync_folders.is_none());
    assert_eq!(
        fs::read(temp.path().join("config/sync/config.json")).unwrap(),
        before
    );
    app.command("sync-folders").unwrap();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    let config = app.sync.saved_config().unwrap().unwrap();
    assert_eq!(config.folders, vec![other.clone()]);
    assert!(config.download_dir.is_some());
    assert_eq!(app.session.text, text);
    assert!(app.session.is_dirty());
    assert_eq!(
        fs::read_to_string(first.join("00.txt")).unwrap(),
        "Note 0\nBody 0"
    );
    assert_eq!(fs::read_to_string(other.join("keep.txt")).unwrap(), "keep");
    app.command("sync-folders").unwrap();
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert!(app.sync.saved_config().unwrap().unwrap().folders.is_empty());
}

#[test]
fn sync_folder_checklist_rejects_stale_configuration_and_missing_new_directories() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    let missing = temp.path().join("missing");
    app.settings.add_folder(missing, false);
    app.command("sync-folders").unwrap();
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    let before = app.sync.saved_config().unwrap().unwrap();
    assert!(app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .is_err());
    assert!(app.sync_folders.is_some());
    assert!(app.sync.saved_config().unwrap().unwrap() == before);
    key(&mut app, KeyCode::Esc);
    app.command("sync-folders").unwrap();
    let mut changed = before;
    changed.scopes = "openid updated".into();
    app.sync.configure(changed.clone()).unwrap();
    let error = app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap_err();
    assert!(error.to_string().contains("configuration changed"));
    assert!(app.sync_folders.is_some());
    assert!(app.sync.saved_config().unwrap().unwrap() == changed);
}

#[test]
fn sync_folder_checklist_preserves_remote_ids_mappings_and_account_binding() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = browse_app(temp.path(), 1);
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    // Opaque IDs for sixteen zero/one bytes; no cryptographic material is needed.
    let mapped = "AAAAAAAAAAAAAAAAAAAAAA".to_owned();
    let remote = "AQEBAQEBAQEBAQEBAQEBAQ".to_owned();
    let mut config = app.sync.saved_config().unwrap().unwrap();
    config.auto_download = false;
    config
        .folder_mappings
        .insert(mapped.clone(), app.settings.notes_dir.clone());
    app.sync.configure(config.clone()).unwrap();
    let state = papertrail_sync::engine::State {
        version: 3,
        account: Some(
            serde_json::from_value(serde_json::json!({"iss": config.issuer, "sub": "alice"}))
                .unwrap(),
        ),
        vault_id: mapped.clone(),
        catalog: Some(
            serde_json::from_value(serde_json::json!({"version": 2, "folders": {
            (mapped.clone()): {"name": "Mapped notes", "color": "preset-0"},
            (remote.clone()): {"name": "Remote only", "color": "preset-2"}
        }, "legacy_notes": {}}))
            .unwrap(),
        ),
        ..papertrail_sync::engine::State::default()
    };
    let path = temp.path().join("config/sync/state.json");
    let original = serde_json::to_vec(&state).unwrap();
    fs::write(&path, &original).unwrap();
    app.command("sync-folders").unwrap();
    let picker = app.sync_folders.as_mut().unwrap();
    assert_eq!(picker.choices.len(), 2);
    let index = picker
        .choices
        .iter()
        .position(|choice| choice.id.as_ref() == Some(&remote))
        .unwrap();
    assert_eq!(picker.color(index), "preset-2");
    for choice in &mut picker.choices {
        choice.selected = choice.id.as_ref() == Some(&remote);
    }
    key(&mut app, KeyCode::Enter);
    let saved = app.sync.saved_config().unwrap().unwrap();
    assert!(saved.folders.is_empty());
    assert!(saved.selected_folders.contains(&remote));
    assert!(saved.excluded_folders.contains(&mapped));
    assert_eq!(saved.folder_mappings, config.folder_mappings);
    assert_eq!(saved.issuer, config.issuer);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(app.settings.notes_dir.join("00.txt").exists());
    app.command("sync-folders").unwrap();
    let picker = app.sync_folders.as_mut().unwrap();
    for choice in &mut picker.choices {
        assert_eq!(choice.selected, choice.id.as_ref() == Some(&remote));
        choice.selected = choice.id.as_ref() == Some(&mapped);
    }
    key(&mut app, KeyCode::Enter);
    let saved = app.sync.saved_config().unwrap().unwrap();
    assert_eq!(saved.folders, vec![app.settings.notes_dir.clone()]);
    assert!(saved.selected_folders.contains(&mapped));
    assert!(saved.excluded_folders.contains(&remote));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn sync_folder_checklist_mouse_scrolling_resize_and_no_mouse_are_safe() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    for index in 0..30 {
        let path = temp.path().join(format!("Long folder 界 {index}"));
        fs::create_dir(&path).unwrap();
        app.settings.add_folder(path, false);
    }
    app.command("sync-folders").unwrap();
    render_app(&mut app, 80, 14);
    let list = app.sync_folders.as_ref().unwrap().area;
    for _ in 0..5 {
        app.mouse(mouse(MouseEventKind::ScrollDown, list.x + 1, list.y))
            .unwrap();
        render_app(&mut app, 80, 14);
    }
    let picker = app.sync_folders.as_ref().unwrap();
    let index = picker.state.offset();
    assert!(index > 0);
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 1,
        list.y,
    ))
    .unwrap();
    assert!(app.sync_folders.as_ref().unwrap().choices[index].selected);
    app.mouse_enabled = false;
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 1,
        list.y,
    ))
    .unwrap();
    assert!(app.sync_folders.as_ref().unwrap().choices[index].selected);
    for (width, height) in [(55, 10), (20, 5), (120, 30)] {
        render_app(&mut app, width, height);
    }
    key(&mut app, KeyCode::Char('i'));
    assert!(app.sync_folders.as_ref().unwrap().details);
    key(&mut app, KeyCode::PageDown);
    render_app(&mut app, 55, 10);
    assert!(app.sync_folders.as_ref().unwrap().details_scroll > 0);
    key(&mut app, KeyCode::Esc);
    assert!(app.sync_folders.is_some());
    assert!(!app.sync_folders.as_ref().unwrap().details);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.sync.saved_config().unwrap().unwrap().folders.len(), 1);
}

#[test]
fn sync_folder_checklist_pauses_automatic_sync_until_closed() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    app.command("sync-folders").unwrap();
    app.sync_unlocked = true;
    app.sync_last = std::time::Instant::now() - std::time::Duration::from_secs(60);
    app.tick().unwrap();
    assert!(app.sync_job.is_none());
    assert!(app
        .command("sync")
        .unwrap_err()
        .to_string()
        .contains(":sync-folders"));
    app.sync_unlocked = false;
    key(&mut app, KeyCode::Esc);
}

#[cfg(unix)]
#[test]
fn sync_folder_checklist_deduplicates_directory_aliases_without_moving_notes() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let mut app = app(&alias);
    app.command("sync-configure https://sync.example https://issuer.example native")
        .unwrap();
    let original = app.settings.notes_dir.clone();
    let canonical = fs::canonicalize(&original).unwrap();
    fs::write(original.join("keep.txt"), "keep").unwrap();
    app.settings.add_folder(canonical.clone(), false);
    app.settings
        .folder_colors
        .insert(original.clone(), "#aabbcc".into());
    app.command("sync-folders").unwrap();
    assert_eq!(app.sync_folders.as_ref().unwrap().choices.len(), 1);
    assert_eq!(app.sync_folders.as_ref().unwrap().color(0), "#aabbcc");
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert!(app.sync.saved_config().unwrap().unwrap().folders.is_empty());
    app.command("sync-folders").unwrap();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.sync.saved_config().unwrap().unwrap().folders,
        vec![canonical]
    );
    assert_eq!(app.settings.notes_dir, original);
    assert_eq!(
        fs::read_to_string(original.join("keep.txt")).unwrap(),
        "keep"
    );
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
        folders.y + 3,
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
            folders.y + 3
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
