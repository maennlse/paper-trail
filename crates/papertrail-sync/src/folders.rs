// SPDX-License-Identifier: GPL-3.0-or-later
//! An encrypted, conditional folder catalog. The server never sees folder names,
//! folder membership or device-local paths. The vault ID reserves its catalog ID.
use crate::{
    engine::{Engine, Remote},
    error, Config,
};
use papertrail_core::Settings;
use papertrail_crypto::{
    opaque_id, Changes, FolderCatalog, FolderData, NoteData, Record, Revision, Secret,
    FOLDER_FORMAT,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Binding {
    pub path: PathBuf,
    pub managed: bool,
    pub observed_name: String,
    pub observed_color: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LegacyNote {
    pub id: String,
    pub filename: String,
    pub folder: String,
}
#[derive(Clone)]
pub struct FolderChoice {
    pub id: Option<String>,
    pub path: Option<PathBuf>,
    pub name: String,
    pub selected: bool,
    pub conflict: bool,
}

fn physical_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}
fn color(settings: &Settings, path: &Path) -> String {
    settings
        .folder_colors
        .get(path)
        .cloned()
        .unwrap_or_else(|| "preset-0".into())
}

impl Engine {
    pub(crate) fn prepare_folders(
        &mut self,
        remote: &impl Remote,
        c: &Config,
        key: &Secret,
        settings: &Settings,
        first: &Changes,
    ) -> io::Result<()> {
        let previous = self.state.catalog.clone();
        let mut catalog = previous.clone();
        let mut head = self.state.catalog_head.clone();
        let mut legacy: BTreeMap<String, LegacyNote> = self
            .state
            .notes
            .iter()
            .filter_map(|(id, t)| {
                t.baseline
                    .as_ref()
                    .filter(|n| n.folder_id.is_none())
                    .map(|n| {
                        (
                            id.clone(),
                            LegacyNote {
                                id: id.clone(),
                                filename: n.filename.clone(),
                                folder: n.folder.clone(),
                            },
                        )
                    })
            })
            .collect();
        // Inspect the complete bounded change snapshot before importing local
        // folders. New devices must adopt existing IDs, never infer them by name.
        let mut page = first.clone();
        let mut cursor = self.state.cursor;
        loop {
            if page.through != first.through || page.next < cursor || page.next > page.through {
                return Err(error("Invalid catalog change snapshot"));
            }
            let mut expected = cursor;
            for change in &page.changes {
                expected += 1;
                if change.sequence != expected {
                    return Err(error("Missing catalog history"));
                }
                match change
                    .revision
                    .open_record(key, self.account()?)
                    .map_err(error)?
                {
                    Record::FolderCatalog(value) => {
                        if self.state.known.contains_key(&change.revision.id) {
                            self.remember(&change.revision, change.sequence)?;
                            // A previous upload may have failed after journaling
                            // this digest but before saving the newer catalog head.
                            if change.revision.parent != head {
                                continue;
                            }
                        }
                        if change.revision.parent != head {
                            return Err(error(
                                "Folder catalog history is not a descendant of the known catalog",
                            ));
                        }
                        if catalog.as_ref().is_some_and(|old| {
                            old.folders.keys().any(|id| !value.folders.contains_key(id))
                                || old
                                    .legacy_notes
                                    .iter()
                                    .any(|(id, folder)| value.legacy_notes.get(id) != Some(folder))
                        }) {
                            return Err(error("Folder catalog removed an identity or rewrote a confirmed legacy assignment"));
                        }
                        head = Some(change.revision.id.clone());
                        catalog = Some(value);
                        self.remember(&change.revision, change.sequence)?;
                    }
                    Record::Note(n) if n.folder_id.is_none() => {
                        legacy.insert(
                            change.revision.note_id.clone(),
                            LegacyNote {
                                id: change.revision.note_id.clone(),
                                filename: n.filename,
                                folder: n.folder,
                            },
                        );
                    }
                    _ => (),
                }
            }
            if expected != page.next || (page.next == cursor && page.next != page.through) {
                return Err(error("Invalid catalog cursor"));
            }
            cursor = page.next;
            if cursor == page.through {
                break;
            }
            page = remote.changes(cursor, Some(first.through))?;
        }
        let access = self.local.clone();
        let activity = access
            .lock()
            .map_err(|_| error("Local note access unavailable"))?;
        self.relocate_pending()?;
        self.check_config(c)?;
        // If another device won the initial catalog CAS, adopt its confirmed
        // membership IDs rather than registering a second identity for the same
        // already-known legacy notes.
        if let Some(confirmed) = &catalog {
            let stale: Vec<_> = self
                .state
                .bindings
                .iter()
                .filter(|(id, _)| !confirmed.folders.contains_key(*id))
                .map(|(id, b)| (id.clone(), b.clone()))
                .collect();
            let mut updated_config = c.clone();
            for (old, binding) in stale {
                let ids: BTreeSet<_> = self
                    .state
                    .notes
                    .iter()
                    .filter(|(_, t)| t.path.parent() == Some(binding.path.as_path()))
                    .filter_map(|(id, _)| confirmed.legacy_notes.get(id).cloned())
                    .collect();
                if ids.len() > 1 {
                    return Err(error("Concurrent migration found several folder identities in one local directory; map them separately before syncing"));
                }
                if let Some(id) = ids.into_iter().next() {
                    if self
                        .state
                        .bindings
                        .get(&id)
                        .is_some_and(|b| b.path != binding.path)
                    {
                        return Err(error(
                            "Choose one local mapping for the confirmed legacy folder",
                        ));
                    }
                    self.state.bindings.remove(&old);
                    self.state.bindings.insert(id.clone(), binding);
                    if let Some(path) = updated_config.folder_mappings.remove(&old) {
                        updated_config.folder_mappings.insert(id.clone(), path);
                    }
                    if updated_config.excluded_folders.remove(&old) {
                        updated_config.excluded_folders.insert(id.clone());
                    }
                    if updated_config.selected_folders.remove(&old) {
                        updated_config.selected_folders.insert(id.clone());
                    }
                    for folder in updated_config.legacy_note_mappings.values_mut() {
                        if folder == &old {
                            *folder = id.clone();
                        }
                    }
                }
            }
            if updated_config != *c {
                self.state.catalog = catalog.clone();
                self.state.catalog_head = head.clone();
                self.save()?;
                let path = self.state_path.parent().unwrap().join("config.json");
                if path.exists() {
                    papertrail_storage::atomic_write(
                        &path,
                        &serde_json::to_vec_pretty(&updated_config).map_err(error)?,
                    )?;
                }
                return Err(io::Error::new(io::ErrorKind::Interrupted,"Adopted confirmed folder identities from another device; retry sync with the updated mappings"));
            }
        }
        let mut desired = catalog
            .clone()
            .or_else(|| self.state.catalog_draft.clone())
            .unwrap_or_else(|| FolderCatalog {
                version: FOLDER_FORMAT,
                ..FolderCatalog::default()
            });
        // Unpublished local folder creations survive a conditional-write failure.
        if catalog.is_some() {
            if let Some(draft) = &self.state.catalog_draft {
                for (id, data) in &draft.folders {
                    if self.state.bindings.contains_key(id) {
                        desired
                            .folders
                            .entry(id.clone())
                            .or_insert_with(|| data.clone());
                    }
                }
            }
        }
        for (id, path) in &c.folder_mappings {
            if c.excluded_folders.contains(id) {
                continue;
            }
            if !desired.folders.contains_key(id) {
                return Err(error("Unknown folder ID in local mapping"));
            }
            if !path.is_dir() {
                return Err(error(
                    "A mapped folder is unavailable; no deletions uploaded",
                ));
            }
            self.bind_path(id, path, false, c, settings, &activity.protected)?;
        }
        let mut paths: BTreeSet<_> = c.folders.iter().cloned().collect();
        if catalog.is_none() {
            paths.extend(
                self.state
                    .notes
                    .values()
                    .filter_map(|t| t.path.parent().map(Path::to_owned)),
            );
        }
        for path in paths {
            if self.state.bindings.values().any(|b| b.path == path) {
                continue;
            }
            let remote_ids: BTreeSet<_> = self
                .state
                .notes
                .iter()
                .filter(|(_, t)| t.path.parent() == Some(path.as_path()))
                .filter_map(|(id, _)| desired.legacy_notes.get(id).cloned())
                .collect();
            if remote_ids.len() > 1 {
                return Err(error("This local folder contains legacy notes assigned to several remote folders. Map them to distinct directories in Configure Sync"));
            }
            let id = remote_ids.into_iter().next().unwrap_or_else(opaque_id);
            if self.state.bindings.get(&id).is_some_and(|b| b.path != path) {
                return Err(error("A remote folder is mapped to several local directories; choose one explicit mapping"));
            }
            desired
                .folders
                .entry(id.clone())
                .or_insert_with(|| FolderData {
                    name: physical_name(&path),
                    color: color(settings, &path),
                });
            self.state.bindings.insert(
                id,
                Binding {
                    observed_name: physical_name(&path),
                    observed_color: settings.folder_colors.get(&path).cloned(),
                    path,
                    managed: false,
                },
            );
        }
        self.state.legacy_pending.clear();
        for note in legacy.values() {
            if desired.legacy_notes.contains_key(&note.id) {
                continue;
            }
            let id = c.legacy_note_mappings.get(&note.id).cloned().or_else(|| {
                self.state
                    .notes
                    .get(&note.id)
                    .and_then(|t| t.path.parent())
                    .and_then(|path| {
                        self.state
                            .bindings
                            .iter()
                            .find(|(_, b)| b.path == path)
                            .map(|(id, _)| id.clone())
                    })
            });
            if let Some(id) = id.filter(|id| desired.folders.contains_key(id)) {
                desired.legacy_notes.insert(note.id.clone(), id);
            } else {
                if !desired.folders.values().any(|f| f.name == note.folder) {
                    desired.folders.insert(
                        opaque_id(),
                        FolderData {
                            name: note.folder.clone(),
                            color: "preset-0".into(),
                        },
                    );
                }
                self.state.legacy_pending.push(note.clone());
            }
        }
        for (id, folder) in &c.legacy_note_mappings {
            if !desired.folders.contains_key(folder) {
                return Err(error("Unknown folder for a legacy-note assignment"));
            }
            if desired
                .legacy_notes
                .get(id)
                .is_some_and(|old| old != folder)
            {
                return Err(error("A confirmed legacy assignment cannot be changed; move the note normally instead"));
            }
            desired.legacy_notes.insert(id.clone(), folder.clone());
        }
        if catalog.is_none() && self.state.legacy_pending.is_empty() {
            desired.folders.retain(|id, _| {
                self.state.bindings.contains_key(id)
                    || desired.legacy_notes.values().any(|folder| folder == id)
            });
        }
        self.state.catalog_draft = Some(desired.clone());
        if !self.state.legacy_pending.is_empty() {
            self.save()?;
            return Err(error("Legacy folder assignments need confirmation. Open Configure Sync and assign the listed notes; no notes have been moved or uploaded"));
        }
        for (id, binding) in &self.state.bindings {
            if !self.folder_enabled(id, c) {
                continue;
            }
            if !binding.path.is_dir() {
                return Err(error(
                    "A selected folder is unavailable; no deletions uploaded",
                ));
            }
            if self.state.folder_conflicts.contains(id) {
                return Err(error("Folder name/color changed on both devices. Confirm the remote metadata in Configure Sync, then rename again if desired"));
            }
            let Some(data) = desired.folders.get_mut(id) else {
                continue;
            };
            let name = physical_name(&binding.path);
            let current_color = settings.folder_colors.get(&binding.path);
            let old = previous.as_ref().and_then(|old| old.folders.get(id));
            if name != binding.observed_name {
                if old.is_some_and(|old| old.name != data.name && name != data.name) {
                    self.state.folder_conflicts.insert(id.clone());
                } else {
                    data.name = name;
                }
            }
            if current_color != binding.observed_color.as_ref() {
                let new = color(settings, &binding.path);
                if old.is_some_and(|old| old.color != data.color && new != data.color) {
                    self.state.folder_conflicts.insert(id.clone());
                } else {
                    data.color = new;
                }
            }
        }
        if !self.state.folder_conflicts.is_empty() {
            self.state.catalog = catalog;
            self.state.catalog_head = head;
            self.save()?;
            return Err(error("Folder metadata conflict: review Configure Sync; local directories and notes were preserved"));
        }
        if catalog.as_ref() != Some(&desired) {
            self.state.catalog = catalog.clone();
            self.state.catalog_head = head.clone();
            self.state.catalog_draft = Some(desired.clone());
            self.save()?; // Folder IDs must survive a lost catalog-upload response.
        }
        drop(activity);
        if catalog.as_ref() != Some(&desired) {
            let revision = Revision::seal_record(
                key,
                self.account()?,
                &self.state.vault_id,
                &self.state.vault_id,
                head.clone(),
                &Record::FolderCatalog(desired.clone()),
            )
            .map_err(error)?;
            if let Some(sequence) = remote.upload(&revision)? {
                self.state.through = self.state.through.max(sequence);
                self.remember(&revision, sequence)?;
                head = Some(revision.id);
                self.state.catalog = Some(desired.clone());
                self.state.catalog_head = head.clone();
                self.save()?;
            } else {
                self.state.catalog = catalog;
                self.state.catalog_head = head;
                self.state.catalog_draft = Some(desired);
                self.save()?;
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Folder catalog changed on another device; retry sync to reconcile safely",
                ));
            }
        }
        self.state.catalog = Some(desired.clone());
        self.state.catalog_head = head;
        self.state.catalog_draft = None;
        self.state.legacy_pending.clear();
        let _activity = access
            .lock()
            .map_err(|_| error("Local note access unavailable"))?;
        self.check_config(c)?;
        for (id, data) in &desired.folders {
            if c.excluded_folders.contains(id)
                || (!c.auto_download
                    && !c.selected_folders.contains(id)
                    && !c.folder_mappings.contains_key(id))
            {
                continue;
            }
            if !self.state.bindings.contains_key(id) {
                let root = c.download_root()?;
                fs::create_dir_all(&root)?;
                let name = safe_folder_name(&data.name);
                let mut path = root.join(&name);
                if self.state.bindings.values().any(|b| b.path == path) {
                    path = root.join(format!("{name} ({})", &id[..8]));
                }
                if path.exists() {
                    return Err(error(format!("Folder '{}' already exists. Map folder {id} explicitly in Configure Sync; nothing was merged",data.name)));
                }
                fs::create_dir(&path)?;
                self.state.bindings.insert(
                    id.clone(),
                    Binding {
                        observed_name: physical_name(&path),
                        observed_color: None,
                        path,
                        managed: true,
                    },
                );
                self.local_changed = true;
            }
        }
        let enabled: BTreeSet<_> = self
            .state
            .bindings
            .keys()
            .filter(|id| self.folder_enabled(id, c))
            .cloned()
            .collect();
        for (id, b) in &mut self.state.bindings {
            if enabled.contains(id) {
                b.observed_name = physical_name(&b.path);
                b.observed_color = settings.folder_colors.get(&b.path).cloned();
            }
        }
        for (id, t) in &mut self.state.notes {
            if let Some(data) = &mut t.baseline {
                if data.folder_id.is_none() {
                    data.folder_id = desired.legacy_notes.get(id).cloned();
                    if data.folder_id.is_some() {
                        data.version = FOLDER_FORMAT;
                    }
                }
            }
        }
        self.save()
    }

    fn bind_path(
        &mut self,
        id: &str,
        path: &Path,
        managed: bool,
        c: &Config,
        settings: &Settings,
        protected: &[PathBuf],
    ) -> io::Result<()> {
        if self
            .state
            .bindings
            .iter()
            .any(|(other, b)| other != id && b.path == path)
        {
            return Err(error("Two remote folders cannot share a local directory"));
        }
        if let Some(old) = self
            .state
            .bindings
            .get(id)
            .filter(|b| b.path != path)
            .cloned()
        {
            if protected.iter().any(|p| p.starts_with(&old.path)) {
                return Err(error(
                    "Close notes in this folder before changing its local mapping",
                ));
            }
            let notes: Vec<_> = self
                .state
                .notes
                .iter()
                .filter(|(_, t)| {
                    !t.deleted
                        && t.baseline.as_ref().and_then(|n| n.folder_id.as_deref()) == Some(id)
                })
                .map(|(id, t)| (id.clone(), t.clone()))
                .collect();
            self.state.bindings.insert(
                id.into(),
                Binding {
                    path: path.into(),
                    managed,
                    observed_name: physical_name(path),
                    observed_color: settings.folder_colors.get(path).cloned(),
                },
            );
            for (note_id, t) in notes {
                let mut data = t
                    .baseline
                    .clone()
                    .ok_or_else(|| error("Missing remapping baseline"))?;
                if t.path.is_file() {
                    data.body = RepositoryLoad::body(&t.path)?;
                }
                let target = self.destination(c, &data, &note_id)?;
                self.write_new(&target, &data.body)?;
                self.state.notes.get_mut(&note_id).unwrap().path = target;
                self.local_changed = true;
            }
        } else {
            self.state
                .bindings
                .entry(id.into())
                .or_insert_with(|| Binding {
                    path: path.into(),
                    managed,
                    observed_name: physical_name(path),
                    observed_color: settings.folder_colors.get(path).cloned(),
                });
        }
        Ok(())
    }
    pub(crate) fn folder_enabled(&self, id: &str, c: &Config) -> bool {
        !c.excluded_folders.contains(id)
            && self.state.bindings.get(id).is_some_and(|b| {
                (b.managed && c.auto_download)
                    || c.selected_folders.contains(id)
                    || c.folders.contains(&b.path)
                    || c.folder_mappings.get(id) == Some(&b.path)
            })
    }
    pub(crate) fn note_enabled(&self, id: &str, c: &Config) -> bool {
        self.state
            .notes
            .get(id)
            .and_then(|t| t.baseline.as_ref())
            .and_then(|n| n.folder_id.as_deref())
            .or_else(|| {
                self.state
                    .catalog
                    .as_ref()
                    .and_then(|cat| cat.legacy_notes.get(id))
                    .map(String::as_str)
            })
            .is_some_and(|id| self.folder_enabled(id, c))
    }
    pub(crate) fn bind_note_data(&mut self, id: &str, data: &mut NoteData) -> io::Result<()> {
        let legacy = data.folder_id.is_none();
        if data.folder_id.is_none() {
            data.folder_id = self
                .state
                .catalog
                .as_ref()
                .and_then(|cat| cat.legacy_notes.get(id))
                .cloned();
        }
        if !data.folder_id.as_ref().is_some_and(|id| {
            self.state
                .catalog
                .as_ref()
                .is_some_and(|cat| cat.folders.contains_key(id))
        }) {
            if !legacy {
                return Err(error("Note refers to an unknown stable folder ID; its encrypted catalog is required before applying it"));
            }
            if !self.state.legacy_pending.iter().any(|n| n.id == id) {
                self.state.legacy_pending.push(LegacyNote {
                    id: id.into(),
                    filename: data.filename.clone(),
                    folder: data.folder.clone(),
                });
                self.save()?;
            }
            return Err(error("Unknown folder assignment; confirm the legacy note's destination in Configure Sync"));
        }
        data.version = FOLDER_FORMAT;
        Ok(())
    }
    pub(crate) fn folder_settings(&self, c: &Config, settings: &mut Settings) {
        if let Some(catalog) = &self.state.catalog {
            for (id, b) in &self.state.bindings {
                if self.folder_enabled(id, c) {
                    if let Some(data) = catalog.folders.get(id) {
                        settings.add_folder(b.path.clone(), false);
                        settings
                            .folder_labels
                            .insert(b.path.clone(), data.name.clone());
                        settings.folder_colors.insert(
                            b.path.clone(),
                            papertrail_core::colors::normalize(&serde_json::Value::String(
                                data.color.clone(),
                            )),
                        );
                    }
                }
            }
        }
    }
    pub(crate) fn check_config(&self, c: &Config) -> io::Result<()> {
        let path = self.state_path.parent().unwrap().join("config.json");
        if path.is_file() && crate::read_config(&fs::read(path)?)? != *c {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Sync settings changed; retrying with the new folder selection",
            ));
        }
        Ok(())
    }
}
struct RepositoryLoad;
impl RepositoryLoad {
    fn body(path: &Path) -> io::Result<String> {
        use papertrail_core::Store;
        Ok(papertrail_storage::Repository
            .load(path)?
            .body
            .unwrap_or_default())
    }
}
fn safe_folder_name(name: &str) -> String {
    let name = papertrail_core::note::normalize_filename(name, "Synced Folder");
    let mut short = String::new();
    for ch in name.chars() {
        if short.len() + ch.len_utf8() > 180 {
            break;
        }
        short.push(ch);
    }
    short
}
