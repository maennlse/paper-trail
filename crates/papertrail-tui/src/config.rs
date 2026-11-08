// SPDX-License-Identifier: GPL-3.0-or-later
use papertrail_storage::atomic_write;
use serde::{Deserialize, Serialize};
use std::{env, fs, io, path::PathBuf};

pub const HELP: &str = "Paper Trail TUI\n\nUsage: paper-trail-tui [--config FILE] [--notes-dir DIR] [--no-mouse]\n\n--config FILE     Share a specific desktop settings file (including Flatpak)\n--notes-dir DIR   Open an existing directory; never recreate a missing bookmark\n--no-mouse        Keep native terminal mouse/text selection\n--help            Show this help\n--version         Show version\n\nWithout --config, use existing native settings, then existing Linux Flatpak\nsettings, otherwise native defaults. TUI preferences are stored alongside the\nselected settings in tui.json. Press ? inside the app for keys and commands.\n\n:login uses device-code authentication: approve the URL/code on another device.\nIt needs no browser or forwarded port on an SSH host. :login-cancel cancels;\n:login-browser explicitly uses the local system browser instead.\nSSH/headless tokens are session-only; :unlock separately unlocks the vault.\n";

#[derive(Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub settings: PathBuf,
    pub default_notes: PathBuf,
    pub folder: Option<PathBuf>,
    pub no_mouse: bool,
}

impl Paths {
    pub fn discover(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let home =
            dirs::home_dir().ok_or_else(|| io::Error::other("Home directory unavailable"))?;
        let native = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("paper-trail/settings.json");
        let flatpak =
            home.join(".var/app/io.github.maennlse.paper-trail/config/paper-trail/settings.json");
        let mut settings = if cfg!(target_os = "linux") && !native.exists() && flatpak.exists() {
            flatpak
        } else {
            native
        };
        let mut folder = None;
        let mut no_mouse = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--no-mouse" => no_mouse = true,
                "--config" => {
                    settings = PathBuf::from(
                        args.next()
                            .ok_or_else(|| io::Error::other("--config needs a file"))?,
                    )
                }
                "--notes-dir" => {
                    folder =
                        Some(PathBuf::from(args.next().ok_or_else(|| {
                            io::Error::other("--notes-dir needs a directory")
                        })?))
                }
                _ => return Err(io::Error::other(format!("Unknown argument: {arg}\n{HELP}"))),
            }
        }
        if settings.is_relative() {
            settings = env::current_dir()?.join(settings);
        }
        // Flatpak's sandbox-private default is also accessible to a native TUI.
        let default_notes = if settings
            == home.join(".var/app/io.github.maennlse.paper-trail/config/paper-trail/settings.json")
        {
            home.join(".var/app/io.github.maennlse.paper-trail/data/notes")
        } else {
            dirs::document_dir()
                .unwrap_or_else(|| home.join("Documents"))
                .join("Paper Trail")
        };
        Ok(Self {
            home,
            settings,
            default_notes,
            folder,
            no_mouse,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub hide_numbers: bool,
    pub folder_width: u16,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            hide_numbers: false,
            folder_width: 16,
        }
    }
}

impl Preferences {
    pub fn load(path: &std::path::Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let mut preferences: Self =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                preferences.folder_width = preferences.folder_width.clamp(10, 30);
                Ok(preferences)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, path: &std::path::Path) -> io::Result<()> {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("Missing preferences parent"))?,
        )?;
        atomic_write(
            path,
            &serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )
    }
}

pub fn editor_command() -> io::Result<Vec<String>> {
    let value = env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });
    let parts = shell_words::split(&value).map_err(io::Error::other)?;
    if parts.is_empty() {
        return Err(io::Error::other("Empty external editor command"));
    }
    Ok(parts)
}
