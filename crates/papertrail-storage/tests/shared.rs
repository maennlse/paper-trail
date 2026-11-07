use papertrail_core::{Session, Settings, Store};
use papertrail_storage::{Repository, SettingsFile};
use std::{fs, time::Instant};

#[test]
fn concurrent_sessions_never_silently_overwrite_each_other() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shared.txt");
    fs::write(&path, "original").unwrap();
    let mut first = Session::default();
    let mut second = Session::default();
    first.open(&Repository, &path).unwrap();
    second.open(&Repository, &path).unwrap();
    first.edit("first client".into(), Instant::now());
    first.flush(&Repository).unwrap();
    second.edit("second client".into(), Instant::now());
    assert!(second
        .flush(&Repository)
        .unwrap_err()
        .to_string()
        .contains("changed on disk"));
    assert!(second.is_dirty());
    assert_eq!(second.text, "second client");
    assert_eq!(fs::read_to_string(path).unwrap(), "first client");
}

#[test]
fn merging_settings_preserves_desktop_and_unknown_fields_and_rejects_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let notes = root.path().join("notes");
    fs::create_dir(&notes).unwrap();
    let file = SettingsFile {
        path: root.path().join("settings.json"),
    };
    file.save(&Settings::new(notes.clone())).unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(&file.path).unwrap()).unwrap();
    raw["future_preference"] = "keep me".into();
    fs::write(&file.path, raw.to_string()).unwrap();
    let baseline = file.load(notes.clone(), root.path()).unwrap();
    let mut desktop = baseline.clone();
    desktop.width = 1500;
    file.save_merged(&desktop, &baseline, root.path()).unwrap();
    let mut tui = baseline.clone();
    tui.pinned_notes.push(notes.join("pin.txt"));
    let merged = file.save_merged(&tui, &baseline, root.path()).unwrap();
    assert_eq!(merged.width, 1500);
    assert_eq!(merged.pinned_notes, tui.pinned_notes);
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&file.path).unwrap()).unwrap();
    assert_eq!(raw["future_preference"], "keep me");
    tui.width = 1700;
    assert!(file.save_merged(&tui, &baseline, root.path()).is_err());
    assert_eq!(file.load(notes, root.path()).unwrap().width, 1500);
}

#[test]
fn conditional_save_preserves_symlink_and_unmodified_notes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    fs::write(&path, "original").unwrap();
    let note = Repository.load(&path).unwrap();
    Repository.save_if_unchanged(&path, "saved", &note).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "saved");
}

#[test]
fn merge_normalizes_legacy_home_paths_before_detecting_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let file = SettingsFile {
        path: root.path().join("settings.json"),
    };
    fs::write(
        &file.path,
        r#"{"notes_dir":"~/notes","note_folders":["~/notes"]}"#,
    )
    .unwrap();
    let baseline = file.load(root.path().join("notes"), root.path()).unwrap();
    let mut settings = baseline.clone();
    settings.add_folder(root.path().join("second"), true);
    let saved = file.save_merged(&settings, &baseline, root.path()).unwrap();
    assert_eq!(saved.notes_dir, root.path().join("second"));
    assert!(saved.note_folders.contains(&root.path().join("notes")));
}
