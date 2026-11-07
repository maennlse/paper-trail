// SPDX-License-Identifier: GPL-3.0-or-later
//! Recover unavailable saved folders without recreating user-selected paths.
use papertrail_core::Settings;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct FolderRecovery {
    pub unavailable: PathBuf,
    pub fallback: PathBuf,
    pub reason: String,
}

pub fn check_notes_folder(path: &Path) -> io::Result<()> {
    fs::read_dir(path)
        .map(|_| ())
        .map_err(|error| folder_error("Could not open notes folder", path, error))
}

pub fn folder_error(operation: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{operation} ‘{}’: {error}", path.display()),
    )
}

pub fn prepare_startup_folder(
    settings: &mut Settings,
    default: &Path,
) -> io::Result<Option<FolderRecovery>> {
    prepare_with(settings, default, check_notes_folder)
}

fn prepare_with(
    settings: &mut Settings,
    default: &Path,
    check: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<Option<FolderRecovery>> {
    let ensure_default = || match check(default) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(default).map_err(|error| {
                folder_error("Could not create default notes folder", default, error)
            })?;
            check(default)
        }
        Err(error) => Err(error),
    };
    if settings.notes_dir == default {
        ensure_default()?;
        return Ok(None);
    }
    match check(&settings.notes_dir) {
        Ok(()) => Ok(None),
        Err(error) => {
            ensure_default()?;
            let recovery = FolderRecovery {
                unavailable: settings.notes_dir.clone(),
                fallback: default.to_owned(),
                reason: error.to_string(),
            };
            // Keep bookmarks, colors, pins, language overrides, and folder order.
            settings.add_folder(default.to_owned(), true);
            Ok(Some(recovery))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_denied_portal_recovers_without_creating_selected_folder() {
        let temp = tempfile::tempdir().unwrap();
        let portal = temp.path().join("portal/notes");
        let default = temp.path().join("app-data/notes");
        let mut settings = Settings::new(portal.clone());
        let recovery = prepare_with(&mut settings, &default, |path| {
            if path == portal {
                Err(folder_error(
                    "Could not open notes folder",
                    path,
                    io::Error::new(io::ErrorKind::PermissionDenied, "Operation not permitted"),
                ))
            } else {
                check_notes_folder(path)
            }
        })
        .unwrap()
        .unwrap();
        assert_eq!(settings.notes_dir, default);
        assert!(default.is_dir());
        assert!(!portal.parent().unwrap().exists());
        assert!(settings.note_folders.contains(&portal));
        assert!(recovery.reason.contains("Operation not permitted"));
        assert!(recovery.reason.contains(&portal.display().to_string()));
    }
}
