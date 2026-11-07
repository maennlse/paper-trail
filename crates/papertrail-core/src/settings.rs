// SPDX-License-Identifier: GPL-3.0-or-later
use crate::colors;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub notes_dir: PathBuf,
    pub note_folders: Vec<PathBuf>,
    pub folder_colors: BTreeMap<PathBuf, String>,
    pub pinned_notes: Vec<PathBuf>,
    pub note_language_overrides: BTreeMap<PathBuf, String>,
    pub show_line_numbers: bool,
    pub wrap_text: bool,
    pub use_monospace: bool,
    #[serde(alias = "show_ruler")]
    pub highlight_current_line: bool,
    pub theme_mode: String,
    pub font_scale: f64,
    pub use_custom_font: bool,
    pub custom_font: String,
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
}

impl Settings {
    pub fn new(default_dir: PathBuf) -> Self {
        Self {
            notes_dir: default_dir.clone(),
            note_folders: vec![default_dir],
            folder_colors: BTreeMap::new(),
            pinned_notes: vec![],
            note_language_overrides: BTreeMap::new(),
            show_line_numbers: true,
            wrap_text: true,
            use_monospace: false,
            highlight_current_line: true,
            theme_mode: "system".into(),
            font_scale: 1.0,
            use_custom_font: false,
            custom_font: "Monospace 11".into(),
            width: 1320,
            height: 860,
            sidebar_width: 260,
        }
    }

    /// Normalize each field independently: malformed legacy values cannot poison valid fields.
    pub fn from_json(raw: &Value, default_dir: PathBuf, home: &Path) -> Self {
        let mut result = Self::new(default_dir);
        let path = |s: &str| {
            if s == "~" {
                home.to_owned()
            } else if let Some(s) = s.strip_prefix("~/") {
                home.join(s)
            } else {
                PathBuf::from(s)
            }
        };
        if let Some(s) = raw["notes_dir"].as_str().filter(|s| !s.trim().is_empty()) {
            result.notes_dir = path(s);
        }
        result.note_folders = raw["note_folders"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(path)
                    .collect()
            })
            .unwrap_or_else(|| vec![result.notes_dir.clone()]);
        let mut unique = vec![];
        for p in &result.note_folders {
            if !unique.contains(p) {
                unique.push(p.clone());
            }
        }
        if unique.is_empty() {
            unique.push(result.notes_dir.clone());
        }
        result.note_folders = unique;
        if !result.note_folders.contains(&result.notes_dir) {
            result.notes_dir = result.note_folders[0].clone();
        }
        for p in &result.note_folders {
            result.folder_colors.insert(
                p.clone(),
                colors::normalize(&raw["folder_colors"][p.to_string_lossy().as_ref()]),
            );
        }
        if let Some(a) = raw["pinned_notes"].as_array() {
            for p in a.iter().filter_map(Value::as_str).map(path) {
                if !result.pinned_notes.contains(&p) {
                    result.pinned_notes.push(p);
                }
            }
        }
        if let Some(a) = raw["note_language_overrides"].as_object() {
            for (k, v) in a {
                if let Some(v) = v.as_str().filter(|s| !s.is_empty()) {
                    result.note_language_overrides.insert(path(k), v.into());
                }
            }
        }
        macro_rules! boolean { ($($field:ident),*) => { $(if let Some(v) = raw[stringify!($field)].as_bool() { result.$field = v; })* }; }
        boolean!(
            show_line_numbers,
            wrap_text,
            use_monospace,
            highlight_current_line,
            use_custom_font
        );
        // Preserve the old preference while giving the clarified name priority.
        if raw["highlight_current_line"].as_bool().is_none() {
            if let Some(value) = raw["show_ruler"].as_bool() {
                result.highlight_current_line = value;
            }
        }
        if let Some(s) = raw["theme_mode"]
            .as_str()
            .filter(|s| ["system", "light", "dark"].contains(s))
        {
            result.theme_mode = s.into();
        }
        if let Some(s) = raw["custom_font"].as_str().filter(|s| !s.trim().is_empty()) {
            result.custom_font = s.into();
        }
        if let Some(n) = raw["font_scale"]
            .as_f64()
            .or_else(|| raw["font_scale"].as_str().and_then(|s| s.parse().ok()))
            .filter(|n| n.is_finite())
        {
            result.font_scale = n.clamp(0.7, 2.5);
        }
        for (key, target, min, max) in [
            ("width", &mut result.width, 480, 7680),
            ("height", &mut result.height, 320, 4320),
            ("sidebar_width", &mut result.sidebar_width, 180, 600),
        ] {
            if let Some(n) = raw[key]
                .as_i64()
                .or_else(|| raw[key].as_str().and_then(|s| s.parse().ok()))
            {
                *target = n.clamp(min, max) as i32;
            }
        }
        result
    }

    pub fn add_folder(&mut self, path: PathBuf, activate: bool) {
        if !self.note_folders.contains(&path) {
            self.note_folders.push(path.clone());
            self.folder_colors
                .entry(path.clone())
                .or_insert_with(|| format!("preset-{}", self.note_folders.len() % 12));
        }
        if activate {
            self.notes_dir = path;
        }
    }

    pub fn remove_folder(&mut self, path: &Path) {
        if self.note_folders.len() <= 1 {
            return;
        }
        self.note_folders.retain(|p| p != path);
        self.folder_colors.remove(path);
        if self.notes_dir == path {
            self.notes_dir = self.note_folders[0].clone();
        }
    }

    pub fn reorder(&mut self, folders: Vec<PathBuf>) -> bool {
        let mut sorted = folders.clone();
        sorted.sort();
        sorted.dedup();
        let mut current = self.note_folders.clone();
        current.sort();
        if sorted != current || folders.len() != current.len() {
            return false;
        }
        self.note_folders = folders;
        true
    }

    pub fn set_pinned(&mut self, path: &Path, pinned: bool) {
        self.pinned_notes.retain(|p| p != path);
        if pinned {
            self.pinned_notes.push(path.to_owned());
        }
    }

    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        for p in &mut self.pinned_notes {
            if p == old {
                *p = new.to_owned();
            }
        }
        if let Some(language) = self.note_language_overrides.remove(old) {
            self.note_language_overrides
                .insert(new.to_owned(), language);
        }
    }

    pub fn delete_note(&mut self, path: &Path) {
        self.set_pinned(path, false);
        self.note_language_overrides.remove(path);
    }

    pub fn rename_folder(&mut self, old: &Path, new: &Path) {
        let rewrite = |p: &Path| {
            p.strip_prefix(old)
                .map(|suffix| new.join(suffix))
                .unwrap_or_else(|_| p.to_owned())
        };
        self.notes_dir = rewrite(&self.notes_dir);
        self.note_folders = self.note_folders.iter().map(|p| rewrite(p)).collect();
        self.pinned_notes = self.pinned_notes.iter().map(|p| rewrite(p)).collect();
        self.folder_colors = self
            .folder_colors
            .iter()
            .map(|(p, v)| (rewrite(p), v.clone()))
            .collect();
        self.note_language_overrides = self
            .note_language_overrides
            .iter()
            .map(|(p, v)| (rewrite(p), v.clone()))
            .collect();
    }
}
