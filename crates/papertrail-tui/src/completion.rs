// SPDX-License-Identifier: GPL-3.0-or-later
//! Prompt completion is read-only: selecting a candidate never executes it.
use std::{fs, io, path::Path};

pub const COMMANDS: &[&str] = &[
    // Notes, editing, and conventional Vim commands.
    "delete",
    "edit",
    "help",
    "move ",
    "new ",
    "path",
    "pin",
    "q",
    "q!",
    "quit",
    "refresh",
    "reload",
    "reload!",
    "rename ",
    "replace ",
    "saveas ",
    "search ",
    "set ",
    "w",
    "wq",
    "x",
    // Local folders.
    "folder-all",
    "folder-close",
    "folder-color ",
    "folder-down",
    "folder-open ",
    "folder-up",
    // Account authentication.
    "login-start",
    "login-browser",
    "login-cancel",
    "login-show",
    "login-sign-out",
    // Vault key lifecycle.
    "vault-unlock",
    "vault-lock",
    "vault-create",
    "vault-recover",
    "vault-change-passphrase",
    // Sync and mapping/migration operations not covered by the folder picker.
    "sync",
    "sync-configure ",
    "sync-discovery-cancel",
    "sync-status",
    "sync-folders",
    "sync-download-dir ",
    "sync-map-folder ",
    "sync-map-legacy ",
    "sync-accept-remote ",
    "sync-auto-download ",
];

/// Retired spellings are errors, not hidden executable aliases. Never echo the
/// user's arguments: they may contain a passphrase entered by mistake.
pub const RETIRED: &[(&str, &str)] = &[
    ("folder", ":folder-open PATH or NUMBER"),
    ("close-folder", ":folder-close"),
    ("all", ":folder-all"),
    ("color", ":folder-color COLOR"),
    ("login", ":login-start"),
    ("logout", ":login-sign-out"),
    ("unlock", ":vault-unlock"),
    ("lock", ":vault-lock"),
    ("create-vault", ":vault-create"),
    ("recover-vault", ":vault-recover"),
    ("change-passphrase", ":vault-change-passphrase"),
    ("sync-add-folder", ":sync-folders to select folders"),
    ("sync-remove-folder", ":sync-folders to uncheck folders"),
    ("sync-select-folder", ":sync-folders to select folders"),
    ("sync-unselect-folder", ":sync-folders to uncheck folders"),
];

#[derive(Default)]
pub struct Completion {
    pub active: bool,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub message: Option<String>,
}

impl Completion {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn cycle(
        &mut self,
        text: &mut String,
        backwards: bool,
        folder_count: usize,
        cwd: &Path,
        home: &Path,
    ) -> io::Result<()> {
        // A unique directory or command prefix can be completed further on the
        // next Tab. Ambiguous candidates retain their original cycle until edited.
        let descend = self.items.len() == 1
            && !backwards
            && (text.ends_with(' ')
                || text.ends_with('/')
                || (cfg!(windows) && text.ends_with('\\')));
        if !self.active || descend {
            let items = match candidates(text, folder_count, cwd, home) {
                Ok(items) => items,
                Err(error) => {
                    self.reset();
                    self.active = true;
                    self.message = Some(error.to_string());
                    return Err(error);
                }
            };
            self.items = items;
            self.selected = None;
            self.active = true;
            self.message = None;
        }
        if self.items.is_empty() {
            return Ok(());
        }
        let index = match self.selected {
            None if backwards => self.items.len() - 1,
            None => 0,
            Some(index) if backwards => (index + self.items.len() - 1) % self.items.len(),
            Some(index) => (index + 1) % self.items.len(),
        };
        *text = self.items[index].clone();
        self.selected = Some(index);
        Ok(())
    }
}

pub fn candidates(
    text: &str,
    folder_count: usize,
    cwd: &Path,
    home: &Path,
) -> io::Result<Vec<String>> {
    let Some((command, arg)) = text.split_once(' ') else {
        return Ok(COMMANDS
            .iter()
            .filter(|c| c.starts_with(text))
            .map(|c| (*c).to_owned())
            .collect());
    };
    let mut values = match command {
        "set" => {
            for (prefix, options) in [
                ("editor ", &["builtin", "neovim"][..]),
                ("nvim-config ", &["clean", "user"][..]),
            ] {
                if let Some(value) = arg.strip_prefix(prefix) {
                    return Ok(options
                        .iter()
                        .filter(|option| option.starts_with(value))
                        .map(|option| format!("set {prefix}{option}"))
                        .collect());
                }
            }
            if let Some(prefix) = arg.strip_prefix("folder-width ") {
                return Ok((7..=30)
                    .map(|n| n.to_string())
                    .filter(|n| n.starts_with(prefix))
                    .map(|n| format!("set folder-width {n}"))
                    .collect());
            }
            if let Some(prefix) = arg.strip_prefix("note-width ") {
                return Ok(std::iter::once("auto".to_owned())
                    .chain((10..=80).map(|n| n.to_string()))
                    .filter(|value| value.starts_with(prefix))
                    .map(|value| format!("set note-width {value}"))
                    .collect());
            }
            [
                "number",
                "nonumber",
                "folder-width ",
                "note-width ",
                "editor ",
                "nvim-config ",
            ]
            .iter()
            .filter(|v| v.starts_with(arg))
            .map(|v| format!("set {v}"))
            .collect()
        }
        "folder-color" => (0..12)
            .map(|n| format!("preset-{n}"))
            .filter(|v| v.starts_with(arg))
            .map(|v| format!("folder-color {v}"))
            .collect(),
        "sync-auto-download" => ["on", "off"]
            .iter()
            .filter(|value| value.starts_with(arg))
            .map(|value| format!("{command} {value}"))
            .collect(),
        "folder-open" | "move" | "saveas" | "sync-download-dir" => {
            let mut values = paths(command, arg, cwd, home, command != "saveas")?;
            if matches!(command, "folder-open" | "move") {
                values.extend(
                    (1..=folder_count)
                        .map(|n| n.to_string())
                        .filter(|v| v.starts_with(arg))
                        .map(|v| format!("{command} {v}")),
                );
            }
            values
        }
        _ => vec![],
    };
    values.sort();
    values.dedup();
    Ok(values)
}

fn paths(
    command: &str,
    arg: &str,
    cwd: &Path,
    home: &Path,
    directories_only: bool,
) -> io::Result<Vec<String>> {
    if arg == "~" {
        return Ok(vec![format!("{command} ~/")]);
    }
    let split = arg.rfind(|c| c == '/' || (cfg!(windows) && c == '\\'));
    let (prefix, leaf) = split.map(|i| arg.split_at(i + 1)).unwrap_or(("", arg));
    let directory = if let Some(suffix) = prefix.strip_prefix("~/") {
        home.join(suffix)
    } else {
        cwd.join(prefix)
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(vec![])
        }
        Err(error) => return Err(error),
    };
    let mut values = vec![];
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(leaf) || name.chars().any(char::is_control) {
            continue;
        }
        let is_directory = entry.path().is_dir();
        if directories_only && !is_directory {
            continue;
        }
        values.push(format!(
            "{command} {prefix}{name}{}",
            if is_directory { "/" } else { "" }
        ));
    }
    Ok(values)
}
