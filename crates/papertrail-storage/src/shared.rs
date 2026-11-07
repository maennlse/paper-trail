// SPDX-License-Identifier: GPL-3.0-or-later
//! Cooperative locks and three-way settings merging for multiple frontends.
use crate::{atomic_write, SettingsFile};
use fs2::FileExt;
use papertrail_core::Settings;
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

fn lock(path: &Path) -> io::Result<File> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing lock parent"))?,
    )?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}

pub(crate) fn lock_note(path: &Path) -> io::Result<File> {
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::other("Missing filename"))?
        .to_os_string();
    name.push(".lock");
    lock(&path.parent().unwrap().join(".papertrail-locks").join(name))
}

impl SettingsFile {
    /// Preserve unknown fields and concurrent unrelated changes; reject conflicts.
    pub fn save_merged(
        &self,
        settings: &Settings,
        baseline: &Settings,
        home: &Path,
    ) -> io::Result<Settings> {
        let _lock = lock(&self.path.with_extension("json.lock"))?;
        let mut latest: Value = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(e) => return Err(e),
        };
        if !latest.is_object() {
            return Err(io::Error::other("Settings must be a JSON object"));
        }
        let normalized = serde_json::to_value(Settings::from_json(
            &latest,
            baseline.notes_dir.clone(),
            home,
        ))
        .map_err(io::Error::other)?;
        let old = serde_json::to_value(baseline).map_err(io::Error::other)?;
        let new = serde_json::to_value(settings).map_err(io::Error::other)?;
        for (key, value) in new.as_object().unwrap() {
            if old[key] != *value {
                if latest.get(key).is_some()
                    && normalized[key] != old[key]
                    && normalized[key] != *value
                {
                    return Err(io::Error::other(format!("Settings changed in another client: {key}. Restart to reload before changing this preference.")));
                }
                latest[key] = value.clone();
            }
        }
        // First run also writes unchanged defaults; existing unknown fields survive.
        for (key, value) in new.as_object().unwrap() {
            if latest.get(key).is_none() {
                latest[key] = value.clone();
            }
        }
        let mut bytes = serde_json::to_vec_pretty(&latest).map_err(io::Error::other)?;
        bytes.push(b'\n');
        atomic_write(&self.path, &bytes)?;
        Ok(Settings::from_json(
            &latest,
            settings.notes_dir.clone(),
            home,
        ))
    }
}
