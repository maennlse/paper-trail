// SPDX-License-Identifier: GPL-3.0-or-later
//! Portable filename detection for the deliberately small native iOS grammar set.
use std::path::Path;

pub const PLAIN_TEXT: &str = "plain";

pub struct Language {
    /// GtkSourceView-compatible IDs keep per-note overrides portable.
    pub id: &'static str,
    pub name: &'static str,
    extensions: &'static [&'static str],
}

pub const LANGUAGES: &[Language] = &[
    Language {
        id: PLAIN_TEXT,
        name: "Plain Text",
        extensions: &[],
    },
    Language {
        id: "markdown",
        name: "Markdown",
        extensions: &["md", "markdown", "mdown", "mkd"],
    },
    Language {
        id: "json",
        name: "JSON",
        extensions: &["json"],
    },
    Language {
        id: "python3",
        name: "Python",
        extensions: &["py", "pyw", "pyi"],
    },
    Language {
        id: "rust",
        name: "Rust",
        extensions: &["rs"],
    },
    Language {
        id: "js",
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
    },
    Language {
        id: "swift",
        name: "Swift",
        extensions: &["swift"],
    },
    Language {
        id: "toml",
        name: "TOML",
        extensions: &["toml"],
    },
    Language {
        id: "yaml",
        name: "YAML",
        extensions: &["yaml", "yml"],
    },
];

pub fn by_id(id: &str) -> Option<&'static Language> {
    // Older GTK overrides use "python"; both select the same native grammar.
    let id = if id == "python" { "python3" } else { id };
    LANGUAGES.iter().find(|language| language.id == id)
}

pub fn detect(path: &Path) -> &'static Language {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|language| language.extensions.contains(&extension.as_str()))
        .unwrap_or(&LANGUAGES[0])
}
