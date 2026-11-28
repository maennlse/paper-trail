// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    engine::{Engine, Remote},
    error, Config,
};
use papertrail_core::Settings;
use papertrail_crypto::{digest, ErasedRevision, Erasure, Record, Revision, Secret};
use std::{collections::BTreeMap, io};

impl Engine {
    pub fn delete_folder(
        &mut self,
        remote: &impl Remote,
        config: &Config,
        key: &Secret,
        _settings: &Settings,
        folder: &str,
    ) -> io::Result<()> {
        remote.check_cancelled()?;
        let first = remote.changes(self.state.cursor, None)?;
        if first.through < self.state.highwater.max(self.state.through) {
            return Err(error("Server history moved backward; deletion stopped"));
        }
        self.state.highwater = first.through;
        self.save()?;
        self.check_config(config)?;
        self.refresh_folders(remote, key, &first)?;
        let mut catalog = self
            .state
            .catalog
            .clone()
            .ok_or_else(|| error("No synced folders"))?;
        if !catalog.folders.contains_key(folder) {
            return Err(error("Unknown synced folder"));
        }
        if catalog.deleted_folders.contains(folder) {
            return Ok(());
        }
        // Read a complete immutable snapshot. The server compares its high-water
        // mark inside the deletion transaction, so concurrent edits force a retry.
        let mut cursor = 0;
        let mut boundary = None;
        let mut revisions = Vec::new();
        let mut membership = BTreeMap::new();
        let mut head = None;
        let mut heads = BTreeMap::new();
        loop {
            remote.check_cancelled()?;
            let page = remote.changes(cursor, boundary)?;
            let start = cursor;
            if boundary.is_some_and(|old| old != page.through)
                || page.next > page.through
                || page.through < self.state.highwater.max(self.state.through)
            {
                return Err(error("Invalid deletion snapshot"));
            }
            boundary = Some(page.through);
            for change in page.changes {
                cursor += 1;
                if change.sequence != cursor || change.revision.vault_id != self.state.vault_id {
                    return Err(error("Missing or invalid deletion history"));
                }
                let r = &change.revision;
                if heads.get(&r.note_id) != r.parent.as_ref() {
                    return Err(error("Non-descendant deletion history"));
                }
                heads.insert(r.note_id.clone(), r.id.clone());
                let record = r.open_record(key, self.account()?).map_err(error)?;
                let original_digest = match &record {
                    Record::Erased {
                        original_digest, ..
                    } => original_digest.clone(),
                    _ => digest(&serde_json::to_vec(r).map_err(error)?),
                };
                if self
                    .state
                    .known
                    .get(&r.id)
                    .is_some_and(|hash| *hash != original_digest)
                    || self
                        .state
                        .sequences
                        .get(&r.id)
                        .is_some_and(|old| *old != cursor)
                {
                    return Err(error("Server altered previously observed deletion history"));
                }
                match record {
                    Record::FolderCatalog(_) => head = Some(change.revision.id.clone()),
                    Record::Note(n) => {
                        let folder = n
                            .folder_id
                            .or_else(|| catalog.legacy_notes.get(&change.revision.note_id).cloned())
                            .ok_or_else(|| error("Unconfirmed legacy folder; deletion stopped"))?;
                        membership.insert(change.revision.note_id.clone(), folder);
                    }
                    Record::DeletedNote => {}
                    Record::Erased { .. } => {
                        membership.remove(&change.revision.note_id);
                    }
                }
                revisions.push(change.revision);
            }
            if cursor != page.next || (cursor != page.through && cursor == start) {
                return Err(error("Invalid deletion cursor"));
            }
            if cursor == page.through {
                break;
            }
        }
        if head != self.state.catalog_head {
            return Err(error("Folder catalog changed; sync and retry deletion"));
        }
        let mut replacements = Vec::new();
        for revision in revisions {
            if membership
                .get(&revision.note_id)
                .is_some_and(|id| id == folder)
            {
                replacements.push(ErasedRevision {
                    original_digest: digest(&serde_json::to_vec(&revision).map_err(error)?),
                    revision: revision
                        .erase(key, self.account()?, folder)
                        .map_err(error)?,
                });
            }
        }
        catalog.deleted_folders.insert(folder.into());
        let revision = Revision::seal_record(
            key,
            self.account()?,
            &self.state.vault_id,
            &self.state.vault_id,
            head,
            &Record::FolderCatalog(catalog.clone()),
        )
        .map_err(error)?;
        let request = Erasure {
            through: cursor,
            catalog: revision.clone(),
            replacements,
        };
        // Bound the atomic request to the server's existing request-size limit.
        if serde_json::to_vec(&request).map_err(error)?.len() > papertrail_crypto::MAX_BLOB * 2 {
            return Err(error(
                "Folder history is too large for one atomic deletion; nothing was deleted",
            ));
        }
        remote.erase_folder(&request)?;
        self.state.catalog = Some(catalog);
        self.state.catalog_head = Some(revision.id.clone());
        self.state.incoming = None;
        self.state.through = self.state.through.max(cursor + 1);
        self.state.highwater = self.state.highwater.max(cursor + 1);
        self.remember(&revision, cursor + 1)?;
        // Reconcile only journal state. A successful server deletion must not
        // become a failure because an unrelated local folder needs attention.
        self.reconcile_deleted_folders(key, &membership)?;
        self.save()
    }
}
