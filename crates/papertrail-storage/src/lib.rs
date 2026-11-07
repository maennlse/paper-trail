// SPDX-License-Identifier: GPL-3.0-or-later
//! Local filesystem adapter. No GTK, GLib, display, or global environment required.
pub mod shared;
pub mod startup;
use papertrail_core::{note::normalize_filename, Note, Settings, Store};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub const SUMMARY_CHAR_LIMIT: usize = 8 * 1024;

#[derive(Clone, Copy, Default)]
pub struct Repository;

impl Store for Repository {
    fn load(&self, path: &Path) -> io::Result<Note> {
        let bytes = fs::read(path)?;
        Ok(Note::from_text(
            path.to_owned(),
            decode(&bytes),
            fs::metadata(path)?.modified()?,
        ))
    }
    fn save(&self, path: &Path, text: &str) -> io::Result<Note> {
        // Match the old open/write behavior for symlinks: update the target, not the link.
        let target = fs::canonicalize(path)?;
        let _lock = shared::lock_note(&target)?;
        // Check write access without truncating before replacing the file.
        OpenOptions::new().write(true).open(&target)?;
        atomic_write(&target, text.as_bytes())?;
        self.load(path)
    }
    fn save_if_unchanged(&self, path: &Path, text: &str, original: &Note) -> io::Result<Note> {
        let target = fs::canonicalize(path)?;
        let _lock = shared::lock_note(&target)?;
        if self.load(path)?.body != original.body {
            return Err(io::Error::other(format!(
                "Note changed on disk: {}. Reload it or save your edits to another file.",
                path.display()
            )));
        }
        OpenOptions::new().write(true).open(&target)?;
        atomic_write(&target, text.as_bytes())?;
        self.load(path)
    }
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

impl Repository {
    pub fn summary(&self, path: &Path) -> io::Result<Note> {
        let mut bytes = vec![];
        // At most four bytes per Unicode scalar plus CRLF translation lookahead.
        File::open(path)?
            .take((SUMMARY_CHAR_LIMIT * 4 + 4) as u64)
            .read_to_end(&mut bytes)?;
        let body: String = decode(&bytes).chars().take(SUMMARY_CHAR_LIMIT).collect();
        let mut note = Note::from_text(path.to_owned(), body, fs::metadata(path)?.modified()?);
        note.body = None;
        Ok(note)
    }

    pub fn list(&self, folder: &Path) -> io::Result<Vec<Note>> {
        let mut notes = vec![];
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path.is_file() {
                notes.push(self.summary(&path)?);
            }
        }
        papertrail_core::note::sort_notes(&mut notes, &[]);
        Ok(notes)
    }

    pub fn create(&self, folder: &Path) -> io::Result<Note> {
        fs::create_dir_all(folder)?;
        let name = format!("{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let (path, file) = reserve(folder, &name)?;
        drop(file);
        self.load(&path)
    }

    pub fn rename(&self, path: &Path, name: &str) -> io::Result<Note> {
        let name = normalize_filename(name, "untitled-note");
        if path.file_name().is_some_and(|n| n == name.as_str()) {
            return self.load(path);
        }
        self.move_named(
            path,
            path.parent()
                .ok_or_else(|| io::Error::other("Missing parent"))?,
            &name,
        )
    }

    pub fn move_note(&self, path: &Path, folder: &Path) -> io::Result<Note> {
        if path.parent() == Some(folder) {
            return self.load(path);
        }
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::other("Missing filename"))?
            .to_str()
            .ok_or_else(|| io::Error::other("Filename is not UTF-8"))?;
        self.move_named(path, folder, name)
    }

    fn move_named(&self, path: &Path, folder: &Path, name: &str) -> io::Result<Note> {
        self.move_named_with(path, folder, name, |old, new| fs::hard_link(old, new))
    }

    fn move_named_with(
        &self,
        path: &Path,
        folder: &Path,
        name: &str,
        link: impl Fn(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<Note> {
        let _lock = shared::lock_note(&fs::canonicalize(path)?)?;
        fs::create_dir_all(folder)?;
        // Atomic no-clobber publication, including EXDEV. Source is removed only after success.
        for index in 0..10_000 {
            let target = numbered(folder, name, index);
            if target == path {
                return self.load(path);
            }
            match link(path, &target) {
                Ok(()) => {
                    if let Err(error) = fs::remove_file(path) {
                        let _ = fs::remove_file(&target);
                        return Err(error);
                    }
                    return self.load(&target);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    let mut temp = tempfile::NamedTempFile::new_in(folder)?;
                    io::copy(&mut File::open(path)?, temp.as_file_mut())?;
                    let metadata = fs::metadata(path)?;
                    temp.as_file().set_permissions(metadata.permissions())?;
                    temp.as_file()
                        .set_times(fs::FileTimes::new().set_modified(metadata.modified()?))?;
                    temp.as_file().sync_all()?;
                    match temp.persist_noclobber(&target) {
                        Ok(_) => {
                            if let Err(error) = fs::remove_file(path) {
                                let _ = fs::remove_file(&target);
                                return Err(error);
                            }
                            return self.load(&target);
                        }
                        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(e.error),
                    }
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Too many filename collisions",
        ))
    }

    pub fn delete(&self, path: &Path) -> io::Result<()> {
        let target = match fs::canonicalize(path) {
            Ok(target) => target,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Also remove a dangling symlink, without recreating its target.
                return match fs::remove_file(path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                    result => result,
                };
            }
            Err(error) => return Err(error),
        };
        let _lock = shared::lock_note(&target)?;
        match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

fn numbered(folder: &Path, name: &str, index: usize) -> PathBuf {
    if index == 0 {
        return folder.join(name);
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    folder.join(format!("{stem}-{index}{extension}"))
}

fn reserve(folder: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    for index in 0..10_000 {
        let path = numbered(folder, name, index);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Too many filename collisions",
    ))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing parent directory"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub struct SettingsFile {
    pub path: PathBuf,
}

impl SettingsFile {
    pub fn load(&self, default_dir: PathBuf, home: &Path) -> io::Result<Settings> {
        let raw = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::Value::Null,
            Err(e) => {
                return Err(startup::folder_error(
                    "Could not read settings",
                    &self.path,
                    e,
                ))
            }
        };
        Ok(Settings::from_json(&raw, default_dir, home))
    }
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        fs::create_dir_all(
            self.path
                .parent()
                .ok_or_else(|| io::Error::other("Missing config directory"))?,
        )?;
        let mut bytes = serde_json::to_vec_pretty(settings)?;
        bytes.push(b'\n');
        atomic_write(&self.path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_device_fallback_publishes_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, "contents").unwrap();
        let timestamp =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        let destination = dir.path().join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("source.txt"), "existing").unwrap();
        let result = Repository
            .move_named_with(&source, &destination, "source.txt", |_, _| {
                Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "simulated EXDEV",
                ))
            })
            .unwrap();
        assert_eq!(result.path, destination.join("source-1.txt"));
        assert_eq!(result.modified, timestamp);
        assert_eq!(fs::read_to_string(result.path).unwrap(), "contents");
        assert_eq!(
            fs::read_to_string(destination.join("source.txt")).unwrap(),
            "existing"
        );
        assert!(!source.exists());
    }
}
