use papertrail_core::{
    colors,
    note::{fold, normalize_filename, sort_notes, summarize},
    Note, Session, Settings, Store,
};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

fn note(path: &str, text: &str) -> Note {
    Note::from_text(path.into(), text.into(), SystemTime::UNIX_EPOCH)
}
fn settings(raw: serde_json::Value) -> Settings {
    Settings::from_json(&raw, "/default".into(), Path::new("/home/test"))
}

#[test]
fn empty_and_blank_first_line_metadata() {
    assert_eq!(
        summarize(Path::new("empty.txt"), ""),
        ("empty".into(), "Plain text note".into())
    );
    assert_eq!(
        summarize(Path::new("my-note.txt"), "\n\nsecond line\n"),
        ("my note".into(), "second line".into())
    );
}
#[test]
fn summaries_use_character_not_byte_limits() {
    let (title, preview) = summarize(
        Path::new("test"),
        &format!("{}\n{}", "🦀".repeat(121), "é".repeat(501)),
    );
    assert_eq!(title.chars().count(), 120);
    assert_eq!(preview.chars().count(), 501);
    assert!(preview.ends_with('…'));
}
#[test]
fn preview_uses_five_lines_and_trims_blanks() {
    assert_eq!(
        summarize(Path::new("test"), "Title\n \n a  \nb\nc\nd\ne\nf").1,
        "a\nb\nc\nd\ne"
    );
}
#[test]
fn unicode_line_boundaries_match_python() {
    assert_eq!(
        summarize(Path::new("test"), "Title\u{2028}\u{85}Preview").1,
        "Preview"
    );
}
#[test]
fn filename_sanitization_preserves_extensionless_names() {
    assert_eq!(
        normalize_filename(" bad:/\\name?.txt. ", "untitled-note"),
        "bad--name.txt"
    );
    assert_eq!(
        normalize_filename("   ...", "untitled-note"),
        "untitled-note"
    );
    assert_eq!(normalize_filename("README", "untitled-note"), "README");
    assert_eq!(normalize_filename("\0\n", "untitled-note"), "untitled-note");
}
#[test]
fn filename_sanitization_is_idempotent_and_cannot_escape_folder() {
    for input in [
        "../../escape",
        "a\\b",
        "<>:\"|?*",
        "..",
        "🦀.txt",
        "a\0b",
        " a ",
    ] {
        let output = normalize_filename(input, "untitled-note");
        assert_eq!(output, normalize_filename(&output, "untitled-note"));
        assert_eq!(Path::new(&output).components().count(), 1);
    }
}
#[test]
fn unicode_casefold_searches_body_title_and_filename() {
    let n = note("Straße.txt", "Title\nΣς σ\nStraße");
    assert!(n.matches("STRASSE"));
    assert!(n.matches("σσ Σ"));
    assert!(n.matches(" title "));
    assert_eq!(fold("ﬃ"), "ffi");
}
#[test]
fn pinned_then_modified_then_path_sorting() {
    let a = note("a", "a");
    let mut b = note("b", "b");
    b.modified += Duration::from_secs(10);
    let mut notes = vec![b.clone(), a.clone()];
    sort_notes(&mut notes, std::slice::from_ref(&a.path));
    assert_eq!(notes[0].path, a.path);
    sort_notes(&mut notes, &[]);
    assert_eq!(notes[0].path, b.path);
    b.modified = a.modified;
    let mut notes = vec![b, a];
    sort_notes(&mut notes, &[]);
    assert_eq!(notes[0].path, Path::new("a"));
}
#[test]
fn color_presets_clamp_and_invalid_values_fall_back() {
    assert_eq!(colors::normalize(&json!(-1)), "preset-0");
    assert_eq!(colors::normalize(&json!(999)), "preset-11");
    for value in [
        json!("preset-nope"),
        json!("invalid"),
        json!(null),
        json!([]),
    ] {
        assert_eq!(colors::normalize(&value), "preset-0");
    }
}
#[test]
fn custom_colors_normalize_and_css_is_stable() {
    assert_eq!(colors::normalize(&json!(" #AbC ")), "#aabbcc");
    assert_eq!(colors::hex("preset-3"), "#148085");
    assert_eq!(colors::css_class("#ABC"), colors::css_class("#aabbcc"));
    assert_eq!(colors::hex("#123456"), "#123456");
}
#[test]
fn folder_badges() {
    for (input, expected) in [
        ("Paper Trail", "PT"),
        ("project_notes", "PN"),
        ("42", "42"),
        ("---", "F"),
    ] {
        assert_eq!(colors::badge(input), expected);
    }
}
#[test]
fn invalid_settings_fields_do_not_reset_valid_fields() {
    let s = settings(
        json!({"notes_dir": 42, "note_folders": "wrong", "width": "bad", "height": 99999, "sidebar_width": false, "wrap_text": "false", "theme_mode": "dark"}),
    );
    assert_eq!((s.width, s.height, s.sidebar_width), (1320, 4320, 260));
    assert!(s.wrap_text);
    assert_eq!(s.theme_mode, "dark");
    assert_eq!(s.notes_dir, Path::new("/default"));
}
#[test]
fn folder_settings_expand_deduplicate_and_choose_valid_active() {
    let s = settings(
        json!({"notes_dir":"/missing", "note_folders":["~/Notes", 1, "~/Notes", "/other"]}),
    );
    assert_eq!(
        s.note_folders,
        [PathBuf::from("/home/test/Notes"), PathBuf::from("/other")]
    );
    assert_eq!(s.notes_dir, s.note_folders[0]);
}
#[test]
fn language_overrides_reject_invalid_values() {
    let s = settings(json!({"note_language_overrides": {"/a": 3, "/b":"rust", "/c":""}}));
    assert_eq!(s.note_language_overrides.len(), 1);
}
#[test]
fn font_settings_are_bounded_and_finite() {
    for (input, expected) in [
        (json!(99), 2.5),
        (json!(-2), 0.7),
        (json!("NaN"), 1.0),
        (json!("1.2"), 1.2),
    ] {
        assert_eq!(settings(json!({"font_scale":input})).font_scale, expected);
    }
}
#[test]
fn legacy_custom_themes_are_ignored() {
    let s = settings(
        json!({"editor_style_scheme":"peninsula", "theme_mode":"light", "show_ruler":false}),
    );
    assert_eq!(s.theme_mode, "light");
    assert!(!s.highlight_current_line);
    assert!(serde_json::to_value(s)
        .unwrap()
        .get("editor_style_scheme")
        .is_none());
}

#[test]
fn current_line_highlight_migrates_legacy_ruler_preference() {
    assert!(settings(json!({})).highlight_current_line);
    for value in [true, false] {
        let migrated = settings(json!({"show_ruler": value}));
        assert_eq!(migrated.highlight_current_line, value);
        let saved = serde_json::to_value(migrated).unwrap();
        assert_eq!(saved["highlight_current_line"], value);
        assert!(saved.get("show_ruler").is_none());
        assert_eq!(settings(saved).highlight_current_line, value);
    }
}

#[test]
fn current_line_highlight_prefers_valid_new_setting() {
    assert!(
        !settings(json!({"highlight_current_line": false, "show_ruler": true}))
            .highlight_current_line
    );
    assert!(
        settings(json!({"highlight_current_line": true, "show_ruler": false}))
            .highlight_current_line
    );
    assert!(
        !settings(json!({"highlight_current_line": "invalid", "show_ruler": false}))
            .highlight_current_line
    );
    assert!(
        settings(json!({"highlight_current_line": "false", "show_ruler": "false"}))
            .highlight_current_line
    );
}
#[test]
fn settings_roundtrip() {
    let s = settings(json!({"theme_mode":"dark", "font_scale":1.3, "pinned_notes":["/default/a"]}));
    let value = serde_json::to_value(&s).unwrap();
    let loaded = settings(value.clone());
    assert_eq!(serde_json::to_value(loaded).unwrap(), value);
}
#[test]
fn cannot_remove_last_folder() {
    let mut s = settings(json!({}));
    s.remove_folder(Path::new("/default"));
    assert_eq!(s.note_folders.len(), 1);
}
#[test]
fn folder_order_must_be_a_permutation() {
    let mut s = settings(json!({}));
    s.add_folder("/other".into(), false);
    assert!(!s.reorder(vec!["/other".into(), "/other".into()]));
    assert!(!s.reorder(vec!["/other".into()]));
    assert!(s.reorder(vec!["/other".into(), "/default".into()]));
}
#[test]
fn folder_rename_updates_pins_and_only_descendant_paths() {
    let mut s = settings(
        json!({"notes_dir":"/notes", "note_folders":["/notes", "/notes2"], "folder_colors":{"/notes":"#abc"}, "pinned_notes":["/notes/a", "/notes2/b"], "note_language_overrides":{"/notes/a":"rust", "/notes2/b":"python"}}),
    );
    s.rename_folder(Path::new("/notes"), Path::new("/renamed"));
    assert_eq!(s.notes_dir, Path::new("/renamed"));
    assert_eq!(
        s.pinned_notes,
        [PathBuf::from("/renamed/a"), PathBuf::from("/notes2/b")]
    );
    assert_eq!(s.folder_colors[Path::new("/renamed")], "#aabbcc");
    assert_eq!(s.note_language_overrides[Path::new("/renamed/a")], "rust");
}
#[test]
fn note_relocation_and_deletion_update_all_metadata() {
    let mut s =
        settings(json!({"pinned_notes":["/a", "/a"], "note_language_overrides":{"/a":"rust"}}));
    assert_eq!(s.pinned_notes.len(), 1);
    s.rename_note(Path::new("/a"), Path::new("/b"));
    assert_eq!(s.note_language_overrides[Path::new("/b")], "rust");
    s.delete_note(Path::new("/b"));
    assert!(s.pinned_notes.is_empty());
    assert!(s.note_language_overrides.is_empty());
}

#[derive(Default)]
struct MemoryStore {
    saved: RefCell<Vec<String>>,
    fail: Cell<bool>,
}
impl Store for MemoryStore {
    fn load(&self, path: &Path) -> io::Result<Note> {
        Ok(note(path.to_str().unwrap(), "initial"))
    }
    fn save(&self, path: &Path, text: &str) -> io::Result<Note> {
        if self.fail.get() {
            return Err(io::Error::other("simulated failure"));
        }
        self.saved.borrow_mut().push(text.into());
        Ok(note(path.to_str().unwrap(), text))
    }
}
fn editing() -> (MemoryStore, Session, Instant) {
    let store = MemoryStore::default();
    let mut session = Session::default();
    session.open(&store, Path::new("first")).unwrap();
    let now = Instant::now();
    session.edit("unsaved".into(), now);
    (store, session, now)
}
#[test]
fn debounce_resets_without_sleeping() {
    let (_, mut s, now) = editing();
    assert!(!s.save_due(now + Duration::from_millis(349)));
    assert!(s.save_due(now + Duration::from_millis(350)));
    s.edit("latest".into(), now + Duration::from_millis(300));
    assert!(!s.save_due(now + Duration::from_millis(600)));
}
#[test]
fn switching_notes_flushes_original_before_loading_next() {
    let (store, mut s, _) = editing();
    s.open(&store, Path::new("new")).unwrap();
    assert_eq!(*store.saved.borrow(), ["unsaved"]);
    assert_eq!(s.current.unwrap().path, Path::new("new"));
}
#[test]
fn closing_flushes_pending_edits() {
    let (store, mut s, _) = editing();
    s.close(&store).unwrap();
    assert_eq!(*store.saved.borrow(), ["unsaved"]);
    assert!(s.current.is_none());
}
#[test]
fn failed_save_prevents_navigation_and_retains_dirty_text() {
    let (store, mut s, _) = editing();
    store.fail.set(true);
    assert!(s.open(&store, Path::new("next")).is_err());
    assert!(s.close(&store).is_err());
    assert_eq!(s.current.as_ref().unwrap().path, Path::new("first"));
    assert_eq!(s.text, "unsaved");
    assert!(s.is_dirty());
    store.fail.set(false);
    s.flush(&store).unwrap();
    assert!(!s.is_dirty());
    assert_eq!(*store.saved.borrow(), ["unsaved"]);
}
#[test]
fn deleting_current_cancels_dirty_state_without_recreating_file() {
    let (store, mut s, _) = editing();
    s.discard_deleted(Path::new("first"));
    s.flush(&store).unwrap();
    assert!(store.saved.borrow().is_empty());
    assert!(s.current.is_none());
}
