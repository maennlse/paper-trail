// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{error, Api, Config};
use fs2::FileExt;
use papertrail_core::{Settings, Store};
use papertrail_crypto::{
    digest, opaque_id, Account, Change, Changes, FolderCatalog, NoteData, Record, Revision, Secret,
    Vault, FOLDER_FORMAT,
};
use papertrail_storage::{atomic_write, Repository};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Tracked {
    pub path: PathBuf,
    pub head: Option<String>,
    pub baseline: Option<NoteData>,
    pub deleted: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Applying {
    pub revision: String,
    pub path: PathBuf,
    pub original: Option<String>,
    pub source: Option<PathBuf>,
    pub source_body: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub account: Option<Account>,
    pub vault_id: String,
    pub generation: u64,
    pub cursor: u64,
    pub through: u64,
    #[serde(default)]
    pub highwater: u64,
    pub notes: BTreeMap<String, Tracked>,
    pub known: BTreeMap<String, String>,
    #[serde(default)]
    pub sequences: BTreeMap<String, u64>,
    pub outgoing: Vec<Revision>,
    pub incoming: Option<Changes>,
    #[serde(default)]
    pub applying: Option<Applying>,
    #[serde(default)]
    pub folders: BTreeMap<PathBuf, String>,
    #[serde(default)]
    pub deferred: Vec<Change>,
    #[serde(default)]
    pub relocations: BTreeSet<String>,
    #[serde(default)]
    pub catalog: Option<FolderCatalog>,
    #[serde(default)]
    pub catalog_head: Option<String>,
    #[serde(default)]
    pub catalog_draft: Option<FolderCatalog>,
    #[serde(default)]
    pub bindings: BTreeMap<String, crate::folders::Binding>,
    #[serde(default)]
    pub legacy_pending: Vec<crate::folders::LegacyNote>,
    #[serde(default)]
    pub folder_conflicts: BTreeSet<String>,
    #[serde(default)]
    pub remote_heads: BTreeMap<String, String>,
    #[serde(default)]
    pub remote_folders: BTreeMap<String, String>,
    #[serde(default)]
    pub bootstrap_pending: bool,
}
// This local journal contains plaintext baselines to detect offline changes. It is
// private local state, never sent to the server or included in encrypted exports.
pub struct Engine {
    pub state: State,
    pub state_path: PathBuf,
    _lock: fs::File,
    pub local: crate::local::LocalAccess,
    pub local_changed: bool,
}
pub trait Remote {
    fn changes(&self, cursor: u64, through: Option<u64>) -> io::Result<Changes>;
    fn upload(&self, revision: &Revision) -> io::Result<Option<u64>>;
}
impl Remote for Api {
    fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
        self.changes(c, t)
    }
    fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
        self.upload(r)
    }
}
impl Engine {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let root = path
            .parent()
            .ok_or_else(|| error("Missing sync state directory"))?;
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| error("Another client is already syncing"))?;
        let state = if path.exists() {
            let mut s: State = serde_json::from_slice(&fs::read(&path)?).map_err(|_| {
                error(
                    "Invalid sync journal; do not reset it without saving local notes/checkpoints",
                )
            })?;
            if !matches!(s.version, 1..=3) {
                return Err(error("Unsupported sync journal version"));
            }
            // V2 can advance a cursor with durable deferred editor revisions.
            // Old binaries must reject it rather than ignore/drop that queue.
            s.version = 3;
            if s.cursor == 0
                && s.notes.is_empty()
                && s.applying.is_none()
                && s.incoming.is_none()
                && s.deferred.is_empty()
            {
                s.bootstrap_pending = true;
            }
            s
        } else {
            State {
                version: 3,
                bootstrap_pending: true,
                ..State::default()
            }
        };
        Ok(Self {
            state,
            state_path: path,
            _lock: lock,
            local: crate::local::LocalAccess::default(),
            local_changed: false,
        })
    }
    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(
            self.state_path
                .parent()
                .ok_or_else(|| error("Missing sync state directory"))?,
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                self.state_path.parent().unwrap(),
                fs::Permissions::from_mode(0o700),
            )?;
        }
        atomic_write(
            &self.state_path,
            &serde_json::to_vec(&self.state).map_err(error)?,
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.state_path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    pub fn bind(&mut self, a: &Account, v: &Vault) -> io::Result<()> {
        if self.state.account.as_ref().is_some_and(|old| old != a)
            || (!self.state.vault_id.is_empty() && self.state.vault_id != v.id)
        {
            return Err(error("This local sync journal belongs to another account/vault. Use a separate local profile and note directory"));
        }
        if v.generation < self.state.generation {
            return Err(error("Server vault envelope moved backward"));
        }
        self.state.account = Some(a.clone());
        self.state.vault_id = v.id.clone();
        self.state.generation = v.generation;
        self.save()
    }
    pub fn run(
        &mut self,
        remote: &impl Remote,
        config: &Config,
        key: &Secret,
        settings: &Settings,
        protected: &[PathBuf],
    ) -> io::Result<String> {
        // Engine owns the cross-process journal lock throughout this operation.
        for folder in &config.folders {
            fs::read_dir(folder).map_err(|_| {
                error("Selected sync folder unavailable; no changes or deletions uploaded")
            })?;
        }
        // Check the remembered high-water mark before making any remote writes.
        let live = remote.changes(self.state.cursor, None)?;
        self.validate_page(&live, None)?;
        if live.through < self.state.highwater {
            return Err(error(
                "Server moved below a previously observed live checkpoint",
            ));
        }
        self.state.highwater = self.state.highwater.max(live.through);
        self.save()?;
        let mut initial = self.state.incoming.clone().unwrap_or_else(|| live.clone());
        self.validate_page(&initial, None)?;
        self.prepare_folders(remote, config, key, settings, &live)?;
        if initial.through < self.state.through && self.state.incoming.is_some() {
            let latest = remote.changes(self.state.cursor, None)?;
            for (old, new) in initial.changes.iter().zip(&latest.changes) {
                if old.sequence != new.sequence || old.revision != new.revision {
                    return Err(error(
                        "Server altered journaled history during folder migration",
                    ));
                }
            }
            initial = latest;
            self.state.incoming = Some(initial.clone());
            self.save()?;
        }
        let access = self.local.clone();
        {
            let activity = access
                .lock()
                .map_err(|_| error("Local note access unavailable"))?;
            self.relocate_pending()?;
            self.check_config(config)?;
            let protected: Vec<_> = protected
                .iter()
                .chain(&activity.protected)
                .cloned()
                .collect();
            self.scan(config, key, settings, &protected)?;
        }
        // Upload from the durable outbox before pulling. Retries use byte-identical
        // revisions and server-side idempotency, even if the first response was lost.
        let mut conflicts = 0;
        while let Some(index) = self
            .state
            .outgoing
            .iter()
            .position(|r| self.note_enabled(&r.note_id, config))
        {
            let r = self.state.outgoing[index].clone();
            // Finish a previously journaled page before acknowledging newer writes,
            // otherwise its stable boundary could fall below our upload checkpoint.
            if self.state.incoming.is_some() {
                break;
            }
            if let Some(sequence) = remote.upload(&r)? {
                self.state.through = self.state.through.max(sequence);
                self.remember(&r, sequence)?;
                let newer_remote = self
                    .state
                    .remote_heads
                    .get(&r.note_id)
                    .and_then(|id| self.state.sequences.get(id))
                    .is_some_and(|old| *old > sequence);
                if !newer_remote {
                    self.state.remote_heads.remove(&r.note_id);
                    self.state.remote_folders.remove(&r.note_id);
                }
                let newer_local = self
                    .state
                    .notes
                    .get(&r.note_id)
                    .and_then(|t| t.head.as_ref())
                    .and_then(|id| self.state.sequences.get(id))
                    .is_some_and(|old| *old > sequence);
                if !newer_local {
                    self.state.notes.get_mut(&r.note_id).unwrap().head = Some(r.id.clone());
                }
                self.state.outgoing.remove(index);
                self.save()?;
            } else {
                // Preserve the encrypted pending payload under a new identity. The
                // original note then takes the server version on the pull below.
                if let Some(mut data) = r.open(key, self.account()?).map_err(error)? {
                    self.bind_note_data(&r.note_id, &mut data)?;
                    let _activity = access
                        .lock()
                        .map_err(|_| error("Local note access unavailable"))?;
                    self.relocate_pending()?;
                    let id = opaque_id();
                    let mut data = data;
                    data.filename = conflict_name(&data.filename, &id);
                    let path = self.destination(config, &data, &id)?;
                    self.write_new(&path, &data.body)?;
                    self.local_changed = true;
                    let copy = Revision::seal(
                        key,
                        self.account()?,
                        &self.state.vault_id,
                        &id,
                        None,
                        Some(&data),
                    )
                    .map_err(error)?;
                    self.state.notes.insert(
                        id.clone(),
                        Tracked {
                            path,
                            head: None,
                            baseline: Some(data),
                            deleted: false,
                        },
                    );
                    self.state.outgoing.push(copy);
                }
                self.state.outgoing.remove(index);
                conflicts += 1;
                self.save()?;
            }
        }
        // Include the acknowledged upload checkpoints in the pull snapshot.
        let mut first = Some(if self.state.incoming.is_some() {
            initial
        } else {
            remote.changes(self.state.cursor, None)?
        });
        let mut boundary = None;
        self.apply_deferred(config, key, protected)?;
        loop {
            let page = match first.take() {
                Some(p) => p,
                None => remote.changes(self.state.cursor, boundary)?,
            };
            self.validate_page(&page, boundary)?;
            boundary = Some(page.through);
            self.state.through = self.state.through.max(page.through);
            self.state.incoming = Some(page.clone());
            self.save()?;
            for change in &page.changes {
                if !self.apply_local(change, config, key, protected)? {
                    if let Some(old) = self.state.deferred.iter().find(|old| {
                        old.sequence == change.sequence || old.revision.id == change.revision.id
                    }) {
                        if old.sequence != change.sequence || old.revision != change.revision {
                            return Err(error("Server altered a deferred revision"));
                        }
                    } else {
                        self.state.deferred.push(change.clone());
                    }
                }
            }
            self.state.cursor = page.next;
            self.state.incoming = None;
            self.save()?;
            if page.next == page.through {
                break;
            }
        }
        if self.state.bootstrap_pending {
            // A fresh replica must finish the history snapshot before publishing
            // files; otherwise a network interruption could resurrect a creation
            // whose tombstone is on the next page.
            self.state.bootstrap_pending = false;
            self.save()?;
            self.apply_deferred(config, key, protected)?;
        }
        Ok(format!(
            "Synced through {} · {conflicts} conflict copies · {} updates deferred (editor or folder selection)",
            self.state.cursor,
            self.state.deferred.len()
        ))
    }
    pub(crate) fn relocate_pending(&mut self) -> io::Result<()> {
        let root = self.state_path.parent().unwrap().to_owned();
        crate::local::relocations(&root, |entries| {
            if entries.is_empty() {
                return Ok(());
            }
            for entry in entries.iter() {
                if self.state.relocations.insert(entry.id.clone()) {
                    for binding in self.state.bindings.values_mut() {
                        if let Ok(suffix) = binding.path.strip_prefix(&entry.old) {
                            binding.path = if suffix.as_os_str().is_empty() {
                                entry.new.clone()
                            } else {
                                entry.new.join(suffix)
                            };
                        }
                    }
                    for tracked in self.state.notes.values_mut() {
                        if let Ok(suffix) = tracked.path.strip_prefix(&entry.old) {
                            tracked.path = if suffix.as_os_str().is_empty() {
                                entry.new.clone()
                            } else {
                                entry.new.join(suffix)
                            };
                        }
                    }
                }
            }
            self.save()?;
            entries.clear();
            Ok(())
        })
    }
    fn apply_local(
        &mut self,
        change: &Change,
        c: &Config,
        key: &Secret,
        protected: &[PathBuf],
    ) -> io::Result<bool> {
        let access = self.local.clone();
        let activity = access
            .lock()
            .map_err(|_| error("Local note access unavailable"))?;
        self.relocate_pending()?;
        self.check_config(c)?;
        let protected: Vec<_> = protected
            .iter()
            .chain(&activity.protected)
            .cloned()
            .collect();
        self.apply(&change.revision, change.sequence, c, key, &protected)
    }
    fn apply_deferred(
        &mut self,
        c: &Config,
        key: &Secret,
        protected: &[PathBuf],
    ) -> io::Result<()> {
        let pending = self.state.deferred.clone();
        for change in pending {
            if self.apply_local(&change, c, key, protected)? {
                self.state
                    .deferred
                    .retain(|old| old.revision.id != change.revision.id);
                self.save()?;
            }
        }
        Ok(())
    }
    pub(crate) fn account(&self) -> io::Result<&Account> {
        self.state
            .account
            .as_ref()
            .ok_or_else(|| error("Sync account not bound"))
    }
    pub(crate) fn remember(&mut self, r: &Revision, sequence: u64) -> io::Result<()> {
        let hash = digest(&serde_json::to_vec(r).map_err(error)?);
        if self.state.known.get(&r.id).is_some_and(|h| h != &hash) {
            return Err(error("Server altered a previously observed revision"));
        }
        if self
            .state
            .sequences
            .get(&r.id)
            .is_some_and(|s| *s != sequence)
        {
            return Err(error("Server replayed a revision at a different sequence"));
        }
        if self
            .state
            .sequences
            .iter()
            .any(|(id, s)| *s == sequence && id != &r.id)
        {
            return Err(error(
                "Server reused an observed sequence for another revision",
            ));
        }
        self.state.sequences.insert(r.id.clone(), sequence);
        self.state.known.insert(r.id.clone(), hash);
        Ok(())
    }
    fn validate_page(&self, p: &Changes, boundary: Option<u64>) -> io::Result<()> {
        if p.through < self.state.through
            || p.next < self.state.cursor
            || p.next > p.through
            || boundary.is_some_and(|t| t != p.through)
        {
            return Err(error(
                "Server history/checkpoint moved backward or changed during pagination",
            ));
        }
        let mut expected = self.state.cursor;
        for c in &p.changes {
            expected += 1;
            if c.sequence != expected {
                return Err(error("Server omitted or reordered a change"));
            }
        }
        if expected != p.next || (p.changes.is_empty() && p.next != p.through) {
            return Err(error("Invalid change cursor"));
        }
        Ok(())
    }
    fn scan(
        &mut self,
        c: &Config,
        key: &Secret,
        settings: &Settings,
        protected: &[PathBuf],
    ) -> io::Result<()> {
        if self.state.incoming.is_some() || self.state.applying.is_some() {
            return Ok(());
        }
        let mut present = BTreeSet::new();
        let folders: Vec<_> = self
            .state
            .bindings
            .iter()
            .filter(|(id, _)| self.folder_enabled(id, c))
            .map(|(id, b)| (id.clone(), b.path.clone()))
            .collect();
        for (folder_id, folder) in &folders {
            for entry in fs::read_dir(folder)? {
                let entry = entry?;
                // Sync only regular files, not symlinks to data outside the selection.
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let path = entry.path();
                present.insert(path.clone());
                let id = self
                    .state
                    .notes
                    .iter()
                    .find(|(_, t)| t.path == path)
                    .map(|(id, _)| id.clone())
                    .unwrap_or_else(opaque_id);
                if self.state.outgoing.iter().any(|r| r.note_id == id) {
                    continue;
                }
                let note = Repository.load(&path)?;
                let data = NoteData {
                    version: FOLDER_FORMAT,
                    filename: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or_else(|| error("Sync requires UTF-8 filenames"))?
                        .into(),
                    folder: self
                        .state
                        .catalog
                        .as_ref()
                        .and_then(|cat| cat.folders.get(folder_id))
                        .map(|data| data.name.clone())
                        .ok_or_else(|| error("Folder catalog missing a selected folder"))?,
                    folder_id: Some(folder_id.clone()),
                    body: note.body.unwrap_or_default(),
                    modified_ms: note
                        .modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .min(i64::MAX as u128) as i64,
                    pinned: settings.pinned_notes.contains(&path),
                    language: settings.note_language_overrides.get(&path).cloned(),
                };
                let previous = self.state.notes.get(&id);
                if self
                    .state
                    .deferred
                    .iter()
                    .any(|change| change.revision.note_id == id)
                {
                    continue;
                }
                // mtime alone is not a change; downloads may acquire local timestamps.
                let unchanged = previous
                    .and_then(|t| t.baseline.as_ref())
                    .is_some_and(|old| equal_note(old, &data));
                if unchanged {
                    continue;
                }
                let parent = previous.and_then(|t| t.head.clone());
                let r = Revision::seal(
                    key,
                    self.account()?,
                    &self.state.vault_id,
                    &id,
                    parent,
                    Some(&data),
                )
                .map_err(error)?;
                self.state.notes.insert(
                    id,
                    Tracked {
                        path,
                        head: r.parent.clone(),
                        baseline: Some(data),
                        deleted: false,
                    },
                );
                self.state.outgoing.push(r);
            }
        }
        let missing: Vec<_> = self
            .state
            .notes
            .iter()
            .filter(|(id, t)| {
                !t.deleted
                    && !present.contains(&t.path)
                    && !protected.contains(&t.path)
                    && self.note_enabled(id, c)
                    && !self.state.outgoing.iter().any(|r| r.note_id == **id)
            })
            .map(|(id, t)| (id.clone(), t.head.clone()))
            .collect();
        for (id, parent) in missing {
            let r = Revision::seal(
                key,
                self.account()?,
                &self.state.vault_id,
                &id,
                parent,
                None,
            )
            .map_err(error)?;
            self.state.notes.get_mut(&id).unwrap().deleted = true;
            self.state.outgoing.push(r);
        }
        self.save()
    }
    pub(crate) fn destination(
        &self,
        _c: &Config,
        data: &NoteData,
        id: &str,
    ) -> io::Result<PathBuf> {
        if !safe_name(&data.filename) {
            return Err(error("Unsafe encrypted filename"));
        }
        let folder = &self
            .state
            .bindings
            .get(
                data.folder_id
                    .as_ref()
                    .ok_or_else(|| error("Note has no stable folder ID"))?,
            )
            .ok_or_else(|| error("Map this remote folder before downloading notes"))?
            .path;
        let path = folder.join(&data.filename);
        if !path.exists() && !self.state.notes.values().any(|n| n.path == path) {
            return Ok(path);
        }
        let mut suffix = String::new();
        for ch in data.filename.chars() {
            if suffix.len() + ch.len_utf8() > 200 {
                break;
            }
            suffix.push(ch);
        }
        for index in 0..10000 {
            let name = if index == 0 {
                format!("{id}-{suffix}")
            } else {
                format!("{id}-{index}-{suffix}")
            };
            let path = folder.join(name);
            if !path.exists() && !self.state.notes.values().any(|n| n.path == path) {
                return Ok(path);
            }
        }
        Err(error(
            "Too many note filename collisions; no files overwritten",
        ))
    }
    pub(crate) fn write_new(&self, path: &Path, text: &str) -> io::Result<()> {
        use std::io::Write;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        fs::File::open(path.parent().ok_or_else(|| error("Missing note parent"))?)?.sync_all()?;
        Ok(())
    }
    fn apply(
        &mut self,
        r: &Revision,
        sequence: u64,
        c: &Config,
        key: &Secret,
        protected: &[PathBuf],
    ) -> io::Result<bool> {
        if r.vault_id != self.state.vault_id {
            return Err(error("Revision belongs to another vault"));
        }
        let record = r.open_record(key, self.account()?).map_err(error)?;
        let hash = digest(&serde_json::to_vec(r).map_err(error)?);
        let replay_pending = self.state.remote_heads.get(&r.note_id) == Some(&r.id)
            && self
                .state
                .deferred
                .iter()
                .any(|change| change.revision.id == r.id);
        if let Some(known) = self.state.known.get(&r.id) {
            if known != &hash {
                return Err(error("Server replayed altered known revision"));
            }
            if self.state.sequences.get(&r.id) != Some(&sequence) {
                return Err(error("Server replayed old ciphertext at a new sequence"));
            }
            if !replay_pending {
                return Ok(true);
            }
        }
        let data = match record {
            Record::FolderCatalog(catalog) => {
                if r.parent != self.state.catalog_head {
                    return Err(error("Unexpected folder catalog parent"));
                }
                if self.state.catalog.as_ref().is_some_and(|old| {
                    old.folders
                        .keys()
                        .any(|id| !catalog.folders.contains_key(id))
                        || old
                            .legacy_notes
                            .iter()
                            .any(|(id, folder)| catalog.legacy_notes.get(id) != Some(folder))
                }) {
                    return Err(error(
                        "Folder catalog removed an identity or rewrote confirmed legacy membership",
                    ));
                }
                self.state.catalog = Some(catalog);
                self.state.catalog_head = Some(r.id.clone());
                self.remember(r, sequence)?;
                self.save()?;
                return Ok(true);
            }
            Record::Note(mut data) => {
                self.bind_note_data(&r.note_id, &mut data)?;
                Some(data)
            }
            Record::DeletedNote => None,
        };
        let tracked = self.state.notes.get(&r.note_id).cloned();
        let folder_id = data
            .as_ref()
            .and_then(|n| n.folder_id.clone())
            .or_else(|| self.state.remote_folders.get(&r.note_id).cloned())
            .or_else(|| {
                tracked
                    .as_ref()
                    .and_then(|t| t.baseline.as_ref())
                    .and_then(|n| n.folder_id.clone())
            });
        if !replay_pending {
            let expected = self
                .state
                .remote_heads
                .get(&r.note_id)
                .cloned()
                .or_else(|| tracked.as_ref().and_then(|t| t.head.clone()));
            if r.parent != expected {
                return Err(error("Server supplied a non-descendant note revision"));
            }
        }
        if self.state.bootstrap_pending
            || folder_id
                .as_ref()
                .is_some_and(|id| !self.folder_enabled(id, c))
            || tracked
                .as_ref()
                .is_some_and(|t| protected.contains(&t.path))
        {
            // Validate each link now, but retain only the latest encrypted snapshot
            // for application. A move into a selected folder can then proceed even
            // if the note's older history belonged to an unselected collection.
            self.state
                .deferred
                .retain(|old| old.revision.note_id != r.note_id || old.sequence > sequence);
            if !self
                .state
                .deferred
                .iter()
                .any(|old| old.revision.id == r.id)
            {
                self.state.deferred.push(Change {
                    sequence,
                    revision: r.clone(),
                });
            }
            self.state
                .remote_heads
                .insert(r.note_id.clone(), r.id.clone());
            if let Some(id) = &folder_id {
                self.state
                    .remote_folders
                    .insert(r.note_id.clone(), id.clone());
            }
            self.remember(r, sequence)?;
            self.save()?;
            return Ok(false);
        }
        let mut original = None;
        if let Some(t) = &tracked {
            // A local deletion can happen while an upload/fetch is in flight.
            // Keep that intent at the original ID, and preserve the remote edit
            // as a conflict copy instead of recreating the just-deleted file.
            if !t.deleted
                && t.baseline.is_some()
                && !t.path.exists()
                && self.state.applying.is_none()
                && self.note_enabled(&r.note_id, c)
            {
                if let Some(data) = &data {
                    let mut copy = data.clone();
                    let id = opaque_id();
                    copy.filename = conflict_name(&copy.filename, &id);
                    let path = self.destination(c, &copy, &id)?;
                    self.write_new(&path, &copy.body)?;
                    let revision = Revision::seal(
                        key,
                        self.account()?,
                        &self.state.vault_id,
                        &id,
                        None,
                        Some(&copy),
                    )
                    .map_err(error)?;
                    self.state.notes.insert(
                        id,
                        Tracked {
                            path,
                            head: None,
                            baseline: Some(copy),
                            deleted: false,
                        },
                    );
                    self.state.outgoing.push(revision);
                    let deletion = Revision::seal(
                        key,
                        self.account()?,
                        &self.state.vault_id,
                        &r.note_id,
                        Some(r.id.clone()),
                        None,
                    )
                    .map_err(error)?;
                    let tracked = self.state.notes.get_mut(&r.note_id).unwrap();
                    tracked.head = Some(r.id.clone());
                    tracked.deleted = true;
                    tracked.baseline = Some(data.clone());
                    self.state.outgoing.push(deletion);
                    self.finish_note_revision(r, sequence, folder_id.as_deref())?;
                    self.local_changed = true;
                    self.save()?;
                    return Ok(true);
                }
            }
            if t.path.exists() {
                let current = Repository.load(&t.path)?;
                // A filesystem edit racing a network fetch must never be overwritten.
                let already_applied = self.state.applying.as_ref().is_some_and(|pending| {
                    pending.revision == r.id
                        && data
                            .as_ref()
                            .is_some_and(|d| current.body.as_deref() == Some(&d.body))
                });
                if !already_applied
                    && self.note_enabled(&r.note_id, c)
                    && t.baseline
                        .as_ref()
                        .is_some_and(|b| current.body.as_deref() != Some(&b.body))
                {
                    let mut copy = t
                        .baseline
                        .clone()
                        .ok_or_else(|| error("Missing local comparison baseline"))?;
                    copy.body = current.body.clone().unwrap_or_default();
                    let id = opaque_id();
                    copy.filename = conflict_name(&copy.filename, &id);
                    let path = self.destination(c, &copy, &id)?;
                    self.write_new(&path, &copy.body)?;
                    let revision = Revision::seal(
                        key,
                        self.account()?,
                        &self.state.vault_id,
                        &id,
                        None,
                        Some(&copy),
                    )
                    .map_err(error)?;
                    self.state.notes.insert(
                        id,
                        Tracked {
                            path,
                            head: None,
                            baseline: Some(copy),
                            deleted: false,
                        },
                    );
                    self.state.outgoing.push(revision);
                    self.state
                        .notes
                        .get_mut(&r.note_id)
                        .unwrap()
                        .baseline
                        .as_mut()
                        .unwrap()
                        .body = current.body.clone().unwrap_or_default();
                    self.save()?;
                }
                original = Some(current);
            }
        }
        match data {
            Some(data) => {
                if !safe_name(&data.filename) {
                    return Err(error("Unsafe encrypted filename"));
                }
                let path = match self.state.applying.as_ref().filter(|a| a.revision == r.id) {
                    Some(a) => a.path.clone(),
                    None => match &tracked {
                        Some(t)
                            if t.baseline.as_ref().is_some_and(|old| {
                                old.filename != data.filename || old.folder_id != data.folder_id
                            }) =>
                        {
                            self.destination(c, &data, &r.note_id)?
                        }
                        Some(t) => t.path.clone(),
                        None => self.destination(c, &data, &r.note_id)?,
                    },
                };
                if self.state.applying.is_none() {
                    let source = tracked
                        .as_ref()
                        .filter(|t| {
                            t.path != path
                                && t.baseline
                                    .as_ref()
                                    .and_then(|n| n.folder_id.as_deref())
                                    .is_none_or(|id| self.folder_enabled(id, c))
                        })
                        .map(|t| t.path.clone());
                    self.state.applying = Some(Applying {
                        revision: r.id.clone(),
                        path: path.clone(),
                        original: if tracked.as_ref().is_some_and(|t| t.path == path) {
                            original.as_ref().and_then(|n| n.body.clone())
                        } else {
                            None
                        },
                        source,
                        source_body: original.as_ref().and_then(|n| n.body.clone()),
                    });
                    self.save()?;
                }
                if path.exists() {
                    let current = Repository.load(&path)?;
                    if current.body.as_deref() != Some(&data.body) {
                        let pending = self.state.applying.as_ref().unwrap();
                        if current.body != pending.original {
                            return Err(error(
                                "Note changed during download recovery; refusing to overwrite it",
                            ));
                        }
                        Repository.save_if_unchanged(&path, &data.body, &current)?;
                    }
                } else {
                    self.write_new(&path, &data.body)?;
                }
                let modified = UNIX_EPOCH
                    .checked_add(std::time::Duration::from_millis(
                        u64::try_from(data.modified_ms)
                            .map_err(|_| error("Invalid encrypted modification time"))?,
                    ))
                    .ok_or_else(|| error("Invalid encrypted modification time"))?;
                let file = OpenOptions::new().write(true).open(&path)?;
                file.set_times(fs::FileTimes::new().set_modified(modified))?;
                file.sync_all()?;
                if let Some(pending) = &self.state.applying {
                    if let Some(source) = pending.source.as_ref().filter(|p| p.exists()) {
                        let original = Repository.load(source)?;
                        if original.body != pending.source_body {
                            return Err(error(
                                "Source note changed during remote rename; both files preserved",
                            ));
                        }
                        Repository.delete_if_unchanged(source, &original)?;
                    }
                }
                self.state.notes.insert(
                    r.note_id.clone(),
                    Tracked {
                        path,
                        head: Some(r.id.clone()),
                        baseline: Some(data),
                        deleted: false,
                    },
                );
            }
            None => {
                if let Some(mut t) = tracked {
                    if let Some(original) = original {
                        Repository.delete_if_unchanged(&t.path, &original)?;
                    }
                    t.deleted = true;
                    t.head = Some(r.id.clone());
                    self.state.notes.insert(r.note_id.clone(), t);
                }
            }
        }
        self.state.applying = None;
        self.finish_note_revision(r, sequence, folder_id.as_deref())?;
        self.local_changed = true;
        self.save()?;
        Ok(true)
    }
    fn finish_note_revision(
        &mut self,
        r: &Revision,
        sequence: u64,
        folder: Option<&str>,
    ) -> io::Result<()> {
        self.state
            .remote_heads
            .insert(r.note_id.clone(), r.id.clone());
        if let Some(folder) = folder {
            self.state
                .remote_folders
                .insert(r.note_id.clone(), folder.into());
        }
        self.state
            .deferred
            .retain(|old| old.revision.note_id != r.note_id || old.sequence > sequence);
        self.remember(r, sequence)
    }
}
fn equal_note(a: &NoteData, b: &NoteData) -> bool {
    a.body == b.body
        && a.filename == b.filename
        && a.folder_id == b.folder_id
        && a.pinned == b.pinned
        && a.language == b.language
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 240
        && !name.contains(['\\', '\0'])
        && Path::new(name).components().count() == 1
        && matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
}
fn conflict_name(name: &str, id: &str) -> String {
    let stem: String = Path::new(name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(40)
        .collect();
    format!("{stem} (conflict {}).txt", &id[..8])
}

#[cfg(test)]
mod tests;
