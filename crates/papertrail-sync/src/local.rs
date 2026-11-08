// SPDX-License-Identifier: GPL-3.0-or-later
//! Short local filesystem leases, never held over HTTP, browser login, or KDFs.
use crate::error;
use fs2::FileExt;
use papertrail_storage::atomic_write;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct Activity {
    pub protected: Vec<PathBuf>,
}
pub type LocalAccess = Arc<Mutex<Activity>>;

#[derive(Serialize, Deserialize)]
pub(crate) struct Relocation {
    pub id: String,
    pub old: PathBuf,
    pub new: PathBuf,
}
pub(crate) fn relocations<T>(
    root: &Path,
    operation: impl FnOnce(&mut Vec<Relocation>) -> io::Result<T>,
) -> io::Result<T> {
    fs::create_dir_all(root)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("relocations.lock"))?;
    lock.lock_exclusive()?;
    let path = root.join("relocations.json");
    let mut entries = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| error("Invalid pending local relocations"))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let previous = serde_json::to_vec(&entries).map_err(error)?;
    let result = operation(&mut entries)?;
    let updated = serde_json::to_vec(&entries).map_err(error)?;
    if previous != updated {
        atomic_write(&path, &updated)?;
    }
    Ok(result)
}
