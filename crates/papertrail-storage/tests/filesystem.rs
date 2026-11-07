use papertrail_core::{Session, Store};
use papertrail_storage::{atomic_write, Repository, SettingsFile, SUMMARY_CHAR_LIMIT};

#[test]
fn newest_first_with_deterministic_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let older = dir.path().join("older");
    let newer = dir.path().join("newer");
    fs::write(&older, "old").unwrap();
    fs::write(&newer, "new").unwrap();
    let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&older)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(base))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(base + std::time::Duration::from_secs(60)))
        .unwrap();
    let notes = Repository.list(dir.path()).unwrap();
    assert_eq!(notes[0].path, newer);
    assert_eq!(notes[1].path, older);
}
use std::{fs, path::Path};

#[test]
fn startup_creates_only_the_default_folder_on_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let default = dir.path().join("app-data/notes");
    let mut settings = papertrail_core::Settings::new(default.clone());
    assert!(
        papertrail_storage::startup::prepare_startup_folder(&mut settings, &default)
            .unwrap()
            .is_none()
    );
    assert!(default.is_dir());
}

#[test]
fn valid_saved_folder_does_not_create_a_default_or_move_notes() {
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("saved");
    fs::create_dir(&saved).unwrap();
    fs::write(saved.join("keep.txt"), "original").unwrap();
    let default = dir.path().join("app-data/notes");
    let mut settings = papertrail_core::Settings::new(saved.clone());
    assert!(
        papertrail_storage::startup::prepare_startup_folder(&mut settings, &default)
            .unwrap()
            .is_none()
    );
    assert_eq!(settings.notes_dir, saved);
    assert!(!default.exists());
    assert_eq!(
        fs::read_to_string(saved.join("keep.txt")).unwrap(),
        "original"
    );
}

#[test]
fn stale_portal_folder_recovers_and_preserves_all_saved_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let portal = dir.path().join("doc/bookmark/notes");
    fs::create_dir_all(portal.parent().unwrap()).unwrap();
    let default = dir.path().join("app-data/notes");
    let mut settings = papertrail_core::Settings::new(portal.clone());
    settings
        .folder_colors
        .insert(portal.clone(), "#aabbcc".into());
    settings.pinned_notes.push(portal.join("keep.txt"));
    settings
        .note_language_overrides
        .insert(portal.join("keep.txt"), "rust".into());
    let colors = settings.folder_colors.clone();
    let pins = settings.pinned_notes.clone();
    let languages = settings.note_language_overrides.clone();
    let recovery = papertrail_storage::startup::prepare_startup_folder(&mut settings, &default)
        .unwrap()
        .unwrap();
    assert_eq!(recovery.unavailable, portal);
    assert_eq!(recovery.fallback, default);
    assert!(!portal.exists(), "Do not recreate a missing portal export");
    assert_eq!(settings.notes_dir, default);
    assert_eq!(settings.note_folders, [portal.clone(), default.clone()]);
    assert_eq!(settings.folder_colors[&portal], colors[&portal]);
    assert_eq!(settings.pinned_notes, pins);
    assert_eq!(settings.note_language_overrides, languages);
    let file = SettingsFile {
        path: dir.path().join("settings.json"),
    };
    file.save(&settings).unwrap();
    let reloaded = file.load(default.clone(), dir.path()).unwrap();
    assert_eq!(reloaded.note_folders, settings.note_folders);
    assert_eq!(reloaded.pinned_notes, settings.pinned_notes);
}

#[test]
fn saved_path_that_is_a_file_recovers_to_default() {
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("not-a-directory");
    fs::write(&saved, "keep").unwrap();
    let default = dir.path().join("app-data/notes");
    let mut settings = papertrail_core::Settings::new(saved.clone());
    assert!(
        papertrail_storage::startup::prepare_startup_folder(&mut settings, &default)
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read_to_string(saved).unwrap(), "keep");
    assert_eq!(settings.notes_dir, default);
}

#[test]
fn unavailable_default_reports_path_without_changing_settings() {
    let dir = tempfile::tempdir().unwrap();
    let default = dir.path().join("blocked");
    fs::write(&default, "keep").unwrap();
    let saved = dir.path().join("missing-portal");
    let mut settings = papertrail_core::Settings::new(saved.clone());
    let error =
        papertrail_storage::startup::prepare_startup_folder(&mut settings, &default).unwrap_err();
    assert!(error.to_string().contains(&default.display().to_string()));
    assert_eq!(settings.notes_dir, saved);
    assert_eq!(settings.note_folders, [saved]);
    assert_eq!(fs::read_to_string(default).unwrap(), "keep");
}

#[test]
fn create_save_reopen_rename_move_delete() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository;
    let note = repo.create(dir.path()).unwrap();
    assert!(note.path.extension().is_some_and(|e| e == "txt"));
    repo.save(&note.path, "Title\nBody 🦀").unwrap();
    assert_eq!(
        repo.load(&note.path).unwrap().body.unwrap(),
        "Title\nBody 🦀"
    );
    let renamed = repo.rename(&note.path, "README").unwrap();
    assert_eq!(renamed.path.file_name().unwrap(), "README");
    assert!(!note.path.exists());
    let moved = repo
        .move_note(&renamed.path, &dir.path().join("other"))
        .unwrap();
    assert!(!renamed.path.exists());
    assert!(moved.path.exists());
    repo.delete(&moved.path).unwrap();
    repo.delete(&moved.path).unwrap();
    assert!(!moved.path.exists());
}
#[test]
fn create_never_overwrites_an_existing_note() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository;
    let first = repo.create(dir.path()).unwrap();
    repo.save(&first.path, "keep").unwrap();
    for _ in 0..10 {
        assert_ne!(repo.create(dir.path()).unwrap().path, first.path);
    }
    assert_eq!(fs::read_to_string(first.path).unwrap(), "keep");
}
#[test]
fn rename_sanitizes_and_resolves_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository;
    let first = repo.create(dir.path()).unwrap();
    fs::write(dir.path().join("bad--name.txt"), "keep").unwrap();
    let renamed = repo.rename(&first.path, " bad:/\\name?.txt. ").unwrap();
    assert_eq!(renamed.path.file_name().unwrap(), "bad--name-1.txt");
    assert_eq!(
        fs::read_to_string(dir.path().join("bad--name.txt")).unwrap(),
        "keep"
    );
}
#[test]
fn moving_to_same_folder_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let note = Repository.create(dir.path()).unwrap();
    assert_eq!(
        Repository.move_note(&note.path, dir.path()).unwrap().path,
        note.path
    );
}
#[test]
fn moving_preserves_destination_on_collision() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("note.txt");
    fs::write(&source, "source").unwrap();
    let folder = dir.path().join("target");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("note.txt"), "existing").unwrap();
    let moved = Repository.move_note(&source, &folder).unwrap();
    assert_eq!(moved.path, folder.join("note-1.txt"));
    assert_eq!(
        fs::read_to_string(folder.join("note.txt")).unwrap(),
        "existing"
    );
    assert_eq!(fs::read_to_string(moved.path).unwrap(), "source");
}
#[test]
fn listing_keeps_bodies_out_of_cache_and_accepts_all_regular_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensionless");
    fs::write(
        &path,
        format!("Title\n{}", "🦀".repeat(SUMMARY_CHAR_LIMIT * 2)),
    )
    .unwrap();
    fs::create_dir(dir.path().join("not-a-note")).unwrap();
    let notes = Repository.list(dir.path()).unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].body.is_none());
    assert!(notes[0].preview.chars().count() <= 501);
    assert!(
        Repository
            .load(&path)
            .unwrap()
            .body
            .unwrap()
            .chars()
            .count()
            > SUMMARY_CHAR_LIMIT
    );
}
#[test]
fn invalid_utf8_and_newlines_match_python_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad");
    fs::write(&path, b"Title\r\n\xffbody\rnext").unwrap();
    assert_eq!(
        Repository.load(&path).unwrap().body.unwrap(),
        "Title\n�body\nnext"
    );
}
#[test]
fn failed_move_retains_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("note");
    fs::write(&source, "keep").unwrap();
    let not_folder = dir.path().join("file");
    fs::write(&not_folder, "file").unwrap();
    assert!(Repository.move_note(&source, &not_folder).is_err());
    assert_eq!(fs::read_to_string(source).unwrap(), "keep");
}
#[test]
fn failed_save_retains_dirty_session() {
    let dir = tempfile::tempdir().unwrap();
    let note = Repository.create(dir.path()).unwrap();
    let mut session = Session::default();
    session.open(&Repository, &note.path).unwrap();
    session.edit("keep unsaved".into(), std::time::Instant::now());
    fs::remove_file(&note.path).unwrap();
    assert!(session.flush(&Repository).is_err());
    assert!(session.is_dirty());
    assert_eq!(session.text, "keep unsaved");
    assert!(!note.path.exists());
}
#[test]
fn pending_edits_survive_new_note_creation() {
    let dir = tempfile::tempdir().unwrap();
    let first = Repository.create(dir.path()).unwrap();
    let mut session = Session::default();
    session.open(&Repository, &first.path).unwrap();
    session.edit("pending".into(), std::time::Instant::now());
    session.flush(&Repository).unwrap();
    let next = Repository.create(dir.path()).unwrap();
    session.open(&Repository, &next.path).unwrap();
    assert_eq!(fs::read_to_string(first.path).unwrap(), "pending");
}
#[test]
fn malformed_settings_recover_and_roundtrip_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let file = SettingsFile {
        path: dir.path().join("config/settings.json"),
    };
    fs::create_dir_all(file.path.parent().unwrap()).unwrap();
    fs::write(&file.path, "{broken").unwrap();
    let mut settings = file.load(dir.path().join("notes"), dir.path()).unwrap();
    settings.theme_mode = "dark".into();
    file.save(&settings).unwrap();
    let loaded = file.load(dir.path().join("notes"), dir.path()).unwrap();
    assert_eq!(loaded.theme_mode, "dark");
    assert_eq!(
        fs::read_dir(file.path.parent().unwrap()).unwrap().count(),
        1
    );
}
#[test]
fn loads_legacy_settings_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let file = SettingsFile {
        path: dir.path().join("settings.json"),
    };
    fs::write(
        &file.path,
        include_str!("../../../tests/fixtures/legacy-settings.json"),
    )
    .unwrap();
    let s = file
        .load("/fallback".into(), Path::new("/home/test"))
        .unwrap();
    assert_eq!(s.theme_mode, "dark");
    assert_eq!(s.pinned_notes.len(), 1);
    assert_eq!(
        s.note_language_overrides[Path::new("/notes/first.txt")],
        "rust"
    );
}
#[test]
fn failed_atomic_write_does_not_modify_an_unrelated_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep");
    fs::write(&path, "keep").unwrap();
    assert!(atomic_write(&path.join("child"), b"bad").is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "keep");
}
#[cfg(unix)]
#[test]
fn save_preserves_permissions_and_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    fs::write(&target, "old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, &link).unwrap();
    Repository.save(&link, "new").unwrap();
    assert!(link.is_symlink());
    assert_eq!(fs::read_to_string(target).unwrap(), "new");
    assert_eq!(
        fs::metadata(link).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
