// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};
use unicode_casefold::UnicodeCaseFold;

#[derive(Clone, Debug)]
pub struct Note {
    pub path: PathBuf,
    pub title: String,
    pub preview: String,
    pub body: Option<String>,
    pub modified: SystemTime,
}

impl Note {
    pub fn from_text(path: PathBuf, body: String, modified: SystemTime) -> Self {
        let (title, preview) = summarize(&path, &body);
        Self {
            path,
            title,
            preview,
            body: Some(body),
            modified,
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = fold(query.trim());
        [
            self.title.as_str(),
            self.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
            self.body.as_deref().unwrap_or(""),
        ]
        .iter()
        .any(|text| fold(text).contains(&query))
    }
}

pub fn fold(text: &str) -> String {
    text.case_fold().collect()
}

pub fn normalize_filename(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .filter_map(|c| match c {
            '/' | '\\' => Some('-'),
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.');
    if cleaned.is_empty() {
        fallback.into()
    } else {
        cleaned.into()
    }
}

pub fn summarize(path: &Path, body: &str) -> (String, String) {
    // Python splitlines also recognizes these Unicode line boundaries.
    let text = body.replace("\r\n", "\n");
    let mut lines: Vec<_> = text
        .split([
            '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
            '\u{2029}',
        ])
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let title = lines
        .first()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .replace('-', " ")
        });
    let preview = lines
        .iter()
        .skip(1)
        .skip_while(|s| s.trim().is_empty())
        .take(5)
        .map(|s| s.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    let preview = preview.trim();
    let mut preview: String = if preview.is_empty() {
        "Plain text note".into()
    } else {
        preview
            .chars()
            .take(500)
            .collect::<String>()
            .trim_end()
            .into()
    };
    if preview.chars().count() == 500 {
        preview.push('…');
    }
    (title.chars().take(120).collect(), preview)
}

pub fn sort_notes(notes: &mut [Note], pinned: &[PathBuf]) {
    notes.sort_by(|a, b| {
        pinned
            .contains(&b.path)
            .cmp(&pinned.contains(&a.path))
            .then_with(|| b.modified.cmp(&a.modified))
            .then_with(|| a.path.cmp(&b.path))
    });
}
