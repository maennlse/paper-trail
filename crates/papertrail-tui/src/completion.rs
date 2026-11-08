// SPDX-License-Identifier: GPL-3.0-or-later
//! Prompt completion is read-only: selecting a candidate never executes it.
use std::{fs, io, path::Path};

pub const COMMANDS: &[&str] = &[
    "all",
    "close-folder",
    "color ",
    "delete",
    "edit",
    "folder ",
    "folder-down",
    "folder-up",
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
    "sync",
    "sync-configure ",
    "sync-status",
    "login",
    "logout",
    "unlock",
    "lock",
    "create-vault",
    "recover-vault",
    "change-passphrase",
    "sync-folders",
    "sync-add-folder ",
    "sync-remove-folder ",
    "sync-download-dir ",
    "sync-map-folder ",
    "sync-map-legacy ",
    "sync-accept-remote ",
    "sync-auto-download ",
    "sync-select-folder ",
    "sync-unselect-folder ",
    "login-browser",
    "login-cancel",
    "login-show",
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
            if let Some(prefix) = arg.strip_prefix("folder-width ") {
                return Ok((10..=30)
                    .map(|n| n.to_string())
                    .filter(|n| n.starts_with(prefix))
                    .map(|n| format!("set folder-width {n}"))
                    .collect());
            }
            ["number", "nonumber", "folder-width "]
                .iter()
                .filter(|v| v.starts_with(arg))
                .map(|v| format!("set {v}"))
                .collect()
        }
        "color" => (0..12)
            .map(|n| format!("preset-{n}"))
            .filter(|v| v.starts_with(arg))
            .map(|v| format!("color {v}"))
            .collect(),
        "folder" | "move" | "saveas" => {
            let mut values = paths(command, arg, cwd, home, command != "saveas")?;
            if command != "saveas" {
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
