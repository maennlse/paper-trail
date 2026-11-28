use super::*;
use std::cell::RefCell;
#[derive(Default)]
struct Memory {
    revisions: RefCell<Vec<Revision>>,
    erased: RefCell<BTreeSet<String>>,
}
impl Remote for Memory {
    fn upload_checked(&self, r: &Revision, head: Option<&str>) -> io::Result<Option<u64>> {
        if !self.erased.borrow().is_empty() {
            if r.note_id != r.vault_id && self.erased.borrow().contains(&r.note_id) {
                return Err(error("erased note"));
            }
            let all = self.revisions.borrow();
            let current = all
                .iter()
                .rev()
                .find(|old| old.note_id == r.vault_id)
                .map(|r| r.id.as_str());
            if current != head {
                return Err(error("stale catalog"));
            }
        }
        self.upload(r)
    }
    fn erase_folder(&self, request: &papertrail_crypto::Erasure) -> io::Result<()> {
        let mut all = self.revisions.borrow_mut();
        if all.len() as u64 != request.through {
            return Err(error("concurrent change"));
        }
        for replacement in &request.replacements {
            let index = all
                .iter()
                .position(|r| r.id == replacement.revision.id)
                .unwrap();
            assert_eq!(
                digest(&serde_json::to_vec(&all[index]).unwrap()),
                replacement.original_digest
            );
            all[index] = replacement.revision.clone();
            self.erased
                .borrow_mut()
                .insert(replacement.revision.note_id.clone());
        }
        self.erased
            .borrow_mut()
            .insert(request.catalog.vault_id.clone());
        all.push(request.catalog.clone());
        Ok(())
    }
    fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
        let mut all = self.revisions.borrow_mut();
        if let Some(index) = all.iter().position(|old| old == r) {
            return Ok(Some(index as u64 + 1));
        }
        if all
            .iter()
            .rev()
            .find(|old| old.note_id == r.note_id)
            .map(|old| old.id.clone())
            != r.parent
        {
            return Ok(None);
        }
        all.push(r.clone());
        Ok(Some(all.len() as u64))
    }
    fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
        let all = self.revisions.borrow();
        let through = t.unwrap_or(all.len() as u64);
        if through > all.len() as u64 || c > through {
            return Err(error("rollback"));
        }
        Ok(Changes {
            through,
            next: through,
            changes: all
                .iter()
                .enumerate()
                .skip(c as usize)
                .take((through - c) as usize)
                .map(|(i, r)| papertrail_crypto::Change {
                    sequence: i as u64 + 1,
                    revision: r.clone(),
                })
                .collect(),
        })
    }
}
#[test]
fn deletion_is_independent_of_local_preparation_and_preserves_unpublished_work() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, mut config, settings) = setup(temp.path(), "device", &a, &vault);
    fs::write(config.folders[0].join("private"), "keep locally").unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let target = engine.state.bindings.keys().next().unwrap().clone();

    let mut catalog = engine.state.catalog.clone().unwrap();
    let other = papertrail_crypto::opaque_id();
    catalog.folders.insert(
        other.clone(),
        papertrail_crypto::FolderData {
            name: "Collision".into(),
            color: "preset-0".into(),
        },
    );
    let update = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &vault.id,
        engine.state.catalog_head.clone(),
        &Record::FolderCatalog(catalog),
    )
    .unwrap();
    remote.upload(&update).unwrap().unwrap();
    let downloads = temp.path().join("downloads");
    fs::create_dir_all(downloads.join("Collision")).unwrap();
    fs::write(downloads.join("Collision/untouched"), "unmanaged").unwrap();
    config.download_dir = Some(downloads.clone());

    // Discovery survives a failure in local folder creation and is visible even
    // when an older unpublished draft exists.
    let failure = engine
        .run(&remote, &config, &key, &settings, &[])
        .unwrap_err();
    assert!(failure.to_string().contains("Collision"));
    drop(engine);
    let mut engine = Engine::open(temp.path().join("device/sync/state.json")).unwrap();
    assert!(engine
        .state
        .catalog
        .as_ref()
        .unwrap()
        .folders
        .contains_key(&other));
    let unpublished = papertrail_crypto::opaque_id();
    let mut draft = engine.state.catalog.clone().unwrap();
    draft.folders.remove(&other);
    draft.folders.insert(
        unpublished.clone(),
        papertrail_crypto::FolderData {
            name: "Unpublished".into(),
            color: "preset-0".into(),
        },
    );
    engine.state.catalog_draft = Some(draft);
    engine.save().unwrap();
    let service =
        crate::control::Service::new_session_only(&temp.path().join("device/settings.json"));
    let choices = service.folder_choices(&settings).unwrap();
    assert!(choices.iter().any(|c| c.id.as_ref() == Some(&other)));
    assert!(!choices.iter().any(|c| c.id.as_ref() == Some(&unpublished)));

    // All of these would fail normal local preparation.
    config
        .folder_mappings
        .insert(other.clone(), temp.path().join("missing"));
    engine.state.folder_conflicts.insert(other.clone());
    let mut pending_note = engine
        .state
        .notes
        .values()
        .next()
        .unwrap()
        .baseline
        .clone()
        .unwrap();
    pending_note.folder_id = Some(other.clone());
    let pending_id = papertrail_crypto::opaque_id();
    let pending = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &pending_id,
        None,
        &Record::Note(pending_note),
    )
    .unwrap();
    engine.state.outgoing.push(pending.clone());
    let before = remote.revisions.borrow().len();
    engine
        .delete_folder(&remote, &config, &key, &settings, &target)
        .unwrap();
    assert_eq!(remote.revisions.borrow().len(), before + 1);
    assert_eq!(
        fs::read_to_string(config.folders[0].join("private")).unwrap(),
        "keep locally"
    );
    assert_eq!(
        fs::read_to_string(downloads.join("Collision/untouched")).unwrap(),
        "unmanaged"
    );
    assert!(!temp.path().join("missing").exists());
    assert!(engine
        .state
        .catalog_draft
        .as_ref()
        .unwrap()
        .folders
        .contains_key(&unpublished));
    assert!(engine
        .state
        .catalog_draft
        .as_ref()
        .unwrap()
        .deleted_folders
        .contains(&target));
    assert!(engine.state.folder_conflicts.contains(&other));
    assert!(engine.state.outgoing.contains(&pending));
}

#[test]
fn concurrent_server_changes_abort_deletion_without_marking_the_folder_deleted() {
    struct Concurrent<'a> {
        remote: &'a Memory,
        update: Revision,
    }
    impl Remote for Concurrent<'_> {
        fn changes(&self, cursor: u64, through: Option<u64>) -> io::Result<Changes> {
            self.remote.changes(cursor, through)
        }
        fn upload(&self, _: &Revision) -> io::Result<Option<u64>> {
            panic!("Deletion must not publish unrelated uploads");
        }
        fn erase_folder(&self, request: &papertrail_crypto::Erasure) -> io::Result<()> {
            self.remote.upload(&self.update)?.unwrap();
            self.remote.erase_folder(request)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, config, settings) = setup(temp.path(), "device", &a, &vault);
    fs::write(config.folders[0].join("note"), "keep").unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let folder = engine.state.bindings.keys().next().unwrap().clone();
    let update = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &vault.id,
        engine.state.catalog_head.clone(),
        &Record::FolderCatalog(engine.state.catalog.clone().unwrap()),
    )
    .unwrap();
    let racing = Concurrent {
        remote: &remote,
        update,
    };
    assert!(engine
        .delete_folder(&racing, &config, &key, &settings, &folder)
        .is_err());
    assert!(!engine
        .state
        .catalog
        .as_ref()
        .unwrap()
        .deleted_folders
        .contains(&folder));
    assert!(remote.erased.borrow().is_empty());
    assert_eq!(
        fs::read_to_string(config.folders[0].join("note")).unwrap(),
        "keep"
    );
    engine
        .delete_folder(&remote, &config, &key, &settings, &folder)
        .unwrap();
}

#[test]
fn folder_choices_only_include_live_remote_ids_or_open_local_directories() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, mut config, mut settings) = setup(temp.path(), "device", &a, &vault);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let published = engine.state.bindings.keys().next().unwrap().clone();
    let closed_draft = temp.path().join("closed-draft");
    let config_only = temp.path().join("config-only");
    fs::create_dir(&closed_draft).unwrap();
    fs::create_dir(&config_only).unwrap();
    let draft_id = papertrail_crypto::opaque_id();
    let mut draft = engine.state.catalog.clone().unwrap();
    draft
        .folders
        .insert(draft_id.clone(), draft.folders[&published].clone());
    engine.state.catalog_draft = Some(draft);
    engine.state.bindings.insert(
        draft_id.clone(),
        crate::folders::Binding {
            path: closed_draft.clone(),
            managed: false,
            observed_name: "closed-draft".into(),
            observed_label: None,
            observed_color: None,
        },
    );
    config.folders.push(config_only.clone());
    engine.save().unwrap();
    fs::write(
        temp.path().join("device/sync/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let journal = fs::read(&engine.state_path).unwrap();
    settings.note_folders.clear(); // A live remote collection remains visible.
    let service =
        crate::control::Service::new_session_only(&temp.path().join("device/settings.json"));
    let choices = service.folder_choices(&settings).unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].id.as_ref(), Some(&published));
    settings.note_folders.push(closed_draft);
    let choices = service.folder_choices(&settings).unwrap();
    assert!(choices.iter().any(|c| c.id.as_ref() == Some(&draft_id)));
    assert!(!choices
        .iter()
        .any(|c| c.path.as_ref() == Some(&config_only)));
    assert_eq!(fs::read(&engine.state_path).unwrap(), journal);
}

#[test]
fn mapping_preflight_reports_retained_owners_without_partial_changes() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, mut config, settings) = setup(temp.path(), "device", &a, &vault);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let owner = engine.state.bindings.keys().next().unwrap().clone();
    let first = "BBBBBBBBBBBBBBBBBBBBBB".to_owned();
    let second = "CCCCCCCCCCCCCCCCCCCCCC".to_owned();
    let data = engine.state.catalog.as_ref().unwrap().folders[&owner].clone();
    let catalog = engine.state.catalog.as_mut().unwrap();
    catalog.folders.insert(first.clone(), data.clone());
    catalog.folders.insert(second.clone(), data);
    // Unchecking an owner does not give another folder permission to adopt it.
    config.excluded_folders.insert(owner.clone());
    let separate = temp.path().join("separate");
    fs::create_dir(&separate).unwrap();
    config.folder_mappings.insert(first.clone(), separate);
    config
        .folder_mappings
        .insert(second.clone(), config.folders[0].clone());
    let message = engine
        .run(&remote, &config, &key, &settings, &[])
        .unwrap_err()
        .to_string();
    assert!(message.contains(&owner));
    assert!(message.contains(&second));
    assert!(message.contains(&config.folders[0].display().to_string()));
    assert!(!engine.state.bindings.contains_key(&first));
    assert!(!engine.state.bindings.contains_key(&second));
}

#[test]
fn folder_erasure_keeps_local_edits_and_prevents_offline_reupload() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut one, c1, s1) = setup(temp.path(), "one", &a, &vault);
    let (mut two, c2, s2) = setup(temp.path(), "two", &a, &vault);
    let path1 = c1.folders[0].join("secret");
    let path2 = c2.folders[0].join("secret");
    fs::write(&path1, "first revision").unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    fs::write(&path1, "second revision").unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    fs::write(&path2, "offline edit that must stay").unwrap();
    fs::write(c2.folders[0].join("offline-new"), "must not upload").unwrap();
    two.scan(&c2, &key, &s2, &[]).unwrap();
    assert!(!two.state.outgoing.is_empty());
    two.state.incoming = Some(remote.changes(two.state.cursor, None).unwrap());
    let folder = one.state.bindings.keys().next().unwrap().clone();
    one.delete_folder(&remote, &c1, &key, &s1, &folder).unwrap();
    assert_eq!(fs::read_to_string(&path1).unwrap(), "second revision");
    assert!(one
        .state
        .catalog
        .as_ref()
        .unwrap()
        .deleted_folders
        .contains(&folder));
    let after = remote.revisions.borrow().len();
    for r in remote
        .revisions
        .borrow()
        .iter()
        .filter(|r| r.note_id != vault.id)
    {
        assert!(matches!(
            r.open_record(&key, &a).unwrap(),
            Record::Erased { .. }
        ));
    }
    // Simulate a crash after persisting the catalog marker but before replacing
    // a journaled page which still contains the original encrypted payload.
    let live = remote.changes(two.state.cursor, None).unwrap();
    two.prepare_folders(&remote, &c2, &key, &s2, &live).unwrap();
    assert!(two.state.incoming.is_some());
    drop(two);
    let mut two = Engine::open(temp.path().join("two/sync/state.json")).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(&path2).unwrap(),
        "offline edit that must stay"
    );
    assert_eq!(remote.revisions.borrow().len(), after);
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(remote.revisions.borrow().len(), after);
    assert!(two.state.outgoing.is_empty());
    assert!(two.state.deferred.is_empty());
    // Fresh replicas authenticate the erased history but never create the folder.
    let mut fresh = Engine::open(temp.path().join("fresh/sync/state.json")).unwrap();
    fresh.bind(&a, &vault).unwrap();
    let config = Config {
        folders: vec![],
        download_dir: Some(temp.path().join("downloads")),
        ..c1.clone()
    };
    fresh.run(&remote, &config, &key, &s1, &[]).unwrap();
    assert!(fresh.state.bindings.is_empty());
    // Re-enabling is an explicit operation and assigns fresh folder/note IDs.
    let old_ids: BTreeSet<_> = one.state.notes.keys().cloned().collect();
    drop(one);
    fs::write(
        temp.path().join("one/sync/config.json"),
        serde_json::to_vec(&c1).unwrap(),
    )
    .unwrap();
    let service = crate::control::Service::new_session_only(&temp.path().join("one/settings.json"));
    let choices = service.folder_choices(&s1).unwrap();
    assert!(choices.is_empty());
    service.sync_as_new_folder(&folder).unwrap();
    assert!(service.sync_as_new_folder(&folder).is_err());
    let new_config = service.saved_config().unwrap().unwrap();
    let mut one = Engine::open(temp.path().join("one/sync/state.json")).unwrap();
    one.run(&remote, &new_config, &key, &s1, &[]).unwrap();
    one.run(&remote, &new_config, &key, &s1, &[]).unwrap();
    let new_folder = one.state.bindings.keys().next().unwrap();
    assert_ne!(new_folder, &folder);
    assert!(one.state.notes.keys().all(|id| !old_ids.contains(id)));
    assert_eq!(fs::read_to_string(&path1).unwrap(), "second revision");
}

#[test]
fn erasing_an_empty_folder_does_not_block_other_folders_or_require_its_directory() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, mut config, settings) = setup(temp.path(), "one", &a, &vault);
    let other = temp.path().join("one/Other");
    fs::create_dir(&other).unwrap();
    config.folders.push(other.clone());
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let folder = engine
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == config.folders[0])
        .unwrap()
        .0
        .clone();
    engine
        .delete_folder(&remote, &config, &key, &settings, &folder)
        .unwrap();
    fs::remove_dir(&config.folders[0]).unwrap();
    fs::write(other.join("still-syncing"), "kept syncing").unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(remote.revisions.borrow().iter().any(
        |r| matches!(r.open_record(&key, &a).unwrap(), Record::Note(n) if n.body == "kept syncing")
    ));
}

#[test]
fn lost_erasure_response_is_reconciled_without_deleting_local_files_or_reuploading() {
    struct LostErasure {
        memory: Memory,
        lost: std::cell::Cell<bool>,
    }
    impl Remote for LostErasure {
        fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
            self.memory.upload(r)
        }
        fn upload_checked(&self, r: &Revision, head: Option<&str>) -> io::Result<Option<u64>> {
            self.memory.upload_checked(r, head)
        }
        fn changes(&self, cursor: u64, through: Option<u64>) -> io::Result<Changes> {
            self.memory.changes(cursor, through)
        }
        fn erase_folder(&self, request: &papertrail_crypto::Erasure) -> io::Result<()> {
            self.memory.erase_folder(request)?;
            if !self.lost.replace(true) {
                return Err(error("lost response"));
            }
            Ok(())
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = LostErasure {
        memory: Memory::default(),
        lost: Default::default(),
    };
    let (mut engine, config, settings) = setup(temp.path(), "one", &a, &vault);
    let path = config.folders[0].join("kept");
    fs::write(&path, "retained locally").unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let folder = engine.state.bindings.keys().next().unwrap().clone();
    assert!(engine
        .delete_folder(&remote, &config, &key, &settings, &folder)
        .is_err());
    let count = remote.memory.revisions.borrow().len();
    drop(engine);
    let mut engine = Engine::open(temp.path().join("one/sync/state.json")).unwrap();
    engine
        .delete_folder(&remote, &config, &key, &settings, &folder)
        .unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "retained locally");
    assert_eq!(remote.memory.revisions.borrow().len(), count);
}

#[test]
fn deleting_a_folder_preserves_notes_and_deferred_moves_already_moved_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut one, mut c1, s1) = setup(temp.path(), "one", &a, &vault);
    let (mut two, c2, s2) = setup(temp.path(), "two", &a, &vault);
    let other = temp.path().join("one/Other");
    fs::create_dir(&other).unwrap();
    c1.folders.push(other.clone());
    let original = c1.folders[0].join("moved");
    fs::write(&original, "moved note").unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    let note_id = one.state.notes.keys().next().unwrap().clone();
    let protected = two.state.notes[&note_id].path.clone();
    let moved = other.join("moved");
    fs::rename(&original, &moved).unwrap();
    one.state.notes.get_mut(&note_id).unwrap().path = moved;
    one.save().unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[protected]).unwrap();
    assert!(!two.state.deferred.is_empty());
    let old_folder = one
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == c1.folders[0])
        .unwrap()
        .0
        .clone();
    one.delete_folder(&remote, &c1, &key, &s1, &old_folder)
        .unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert!(two.state.deferred.is_empty());
    assert_eq!(
        fs::read_to_string(&two.state.notes[&note_id].path).unwrap(),
        "moved note"
    );
    let mut fresh = Engine::open(temp.path().join("fresh/sync/state.json")).unwrap();
    fresh.bind(&a, &vault).unwrap();
    let config = Config {
        folders: vec![],
        download_dir: Some(temp.path().join("fresh/downloads")),
        ..c1
    };
    fresh.run(&remote, &config, &key, &s1, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(&fresh.state.notes[&note_id].path).unwrap(),
        "moved note"
    );
}

fn setup(root: &Path, name: &str, a: &Account, v: &Vault) -> (Engine, Config, Settings) {
    let notes = root.join(name).join("Notes");
    fs::create_dir_all(&notes).unwrap();
    let mut e = Engine::open(root.join(name).join("sync/state.json")).unwrap();
    e.bind(a, v).unwrap();
    // Explicitly map the same test folder identity to each device's own path.
    let folder_id = "AAAAAAAAAAAAAAAAAAAAAA".to_owned();
    e.state.catalog_draft = Some(FolderCatalog {
        version: FOLDER_FORMAT,
        folders: BTreeMap::from([(
            folder_id.clone(),
            papertrail_crypto::FolderData {
                name: "Notes".into(),
                color: "preset-0".into(),
            },
        )]),
        legacy_notes: BTreeMap::new(),
        deleted_folders: BTreeSet::new(),
    });
    e.state.bindings.insert(
        folder_id,
        crate::folders::Binding {
            path: notes.clone(),
            managed: false,
            observed_name: "Notes".into(),
            observed_label: None,
            observed_color: None,
        },
    );
    e.save().unwrap();
    let c = Config {
        version: 1,
        server: "https://sync.example".into(),
        issuer: a.iss.clone(),
        client_id: "native".into(),
        scopes: "openid".into(),
        folders: vec![notes.clone()],
        ..Config::default()
    };
    (e, c, Settings::new(notes))
}
#[test]
fn cancellation_journals_inflight_upload_and_resumes_without_losing_pending_notes() {
    struct CancelAfterUpload<'a> {
        remote: &'a Memory,
        cancel: crate::control::Cancellation,
    }
    impl Remote for CancelAfterUpload<'_> {
        fn check_cancelled(&self) -> io::Result<()> {
            if self.cancel.is_cancelled() {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        }
        fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
            self.check_cancelled()?;
            self.remote.changes(c, t)
        }
        fn upload(&self, revision: &Revision) -> io::Result<Option<u64>> {
            self.check_cancelled()?;
            let receipt = self.remote.upload(revision)?;
            self.cancel.cancel();
            Ok(receipt)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let account = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&account, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, config, settings) = setup(temp.path(), "one", &account, &vault);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let before = remote.revisions.borrow().len();
    for name in ["first.txt", "second.txt"] {
        fs::write(config.folders[0].join(name), name).unwrap();
    }
    let cancelled = CancelAfterUpload {
        remote: &remote,
        cancel: crate::control::Cancellation::default(),
    };
    let error = engine
        .run(&cancelled, &config, &key, &settings, &[])
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(remote.revisions.borrow().len(), before + 1);
    assert_eq!(engine.state.outgoing.len(), 1);
    let receipt = remote.revisions.borrow().last().unwrap().id.clone();
    assert!(engine.state.known.contains_key(&receipt));
    let path = engine.state_path.clone();
    drop(engine);
    let mut engine = Engine::open(path).unwrap();
    assert!(engine.state.known.contains_key(&receipt));
    assert_eq!(engine.state.outgoing.len(), 1);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(engine.state.outgoing.is_empty());
    assert_eq!(remote.revisions.borrow().len(), before + 2);
    for name in ["first.txt", "second.txt"] {
        assert_eq!(
            fs::read_to_string(config.folders[0].join(name)).unwrap(),
            name
        );
    }
}

#[test]
fn two_devices_conflicts_tombstones_and_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    let p1 = c1.folders[0].join("note.txt");
    let p2 = c2.folders[0].join("note.txt");
    fs::write(&p1, "original").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(&p2).unwrap(), "original");
    fs::write(&p1, "edit one").unwrap();
    fs::write(&p2, "edit two").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let outcome = e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert!(outcome.contains("1 conflict"));
    let bodies: Vec<_> = fs::read_dir(&c2.folders[0])
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert!(bodies.contains(&"edit one".into()));
    assert!(bodies.contains(&"edit two".into()));
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    fs::remove_file(&p1).unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert!(!p2.exists());
    remote.revisions.borrow_mut().pop();
    assert!(e2.run(&remote, &c2, &key, &s2, &[]).is_err());
}
#[test]
fn unavailable_folder_never_becomes_deletion_and_account_binding_is_strict() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e, c, s) = setup(temp.path(), "one", &a, &v);
    fs::write(c.folders[0].join("n.txt"), "note").unwrap();
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    fs::rename(&c.folders[0], c.folders[0].with_file_name("unmounted")).unwrap();
    assert!(e.run(&remote, &c, &key, &s, &[]).is_err());
    assert_eq!(
        remote
            .revisions
            .borrow()
            .iter()
            .filter(|r| r.note_id != v.id)
            .count(),
        1
    );
    let mut b = a;
    b.sub = "bob".into();
    assert!(e.bind(&b, &v).is_err());
    assert!(Engine::open(e.state_path.clone()).is_err());
}

#[test]
fn legacy_journals_upgrade_without_resetting_checkpoints_and_unknown_versions_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sync/state.json");
    let mut e = Engine::open(path.clone()).unwrap();
    e.state.cursor = 17;
    e.state.highwater = 19;
    e.save().unwrap();
    drop(e);
    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy["version"] = serde_json::json!(1);
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let e = Engine::open(path.clone()).unwrap();
    assert_eq!(e.state.version, 3);
    assert_eq!(e.state.cursor, 17);
    assert_eq!(e.state.highwater, 19);
    e.save().unwrap();
    drop(e);
    legacy["version"] = serde_json::json!(99);
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(Engine::open(path.clone()).is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn interrupted_download_replays_idempotently_and_rejects_unsafe_names() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e, mut c, s) = setup(temp.path(), "device", &a, &v);
    let data = NoteData {
        version: 1,
        filename: "note.txt".into(),
        folder: "Notes".into(),
        folder_id: None,
        body: "downloaded".into(),
        modified_ms: 1,
        pinned: false,
        language: None,
    };
    let r = Revision::seal(&key, &a, &v.id, &opaque_id(), None, Some(&data)).unwrap();
    remote.upload(&r).unwrap();
    c.legacy_note_mappings
        .insert(r.note_id.clone(), "AAAAAAAAAAAAAAAAAAAAAA".into());
    // Simulate termination after publishing a downloaded file but before updating
    // its tracked head/cursor. The durable applying record fixes the target path.
    let path = c.folders[0].join("note.txt");
    fs::write(&path, &data.body).unwrap();
    e.state.incoming = Some(remote.changes(0, None).unwrap());
    e.state.applying = Some(Applying {
        revision: r.id.clone(),
        path: path.clone(),
        original: None,
        source: None,
        source_body: None,
    });
    e.state.bootstrap_pending = false; // Model an interrupted legacy file publication.
    e.save().unwrap();
    let journal = e.state_path.clone();
    drop(e);
    let mut e = Engine::open(journal).unwrap();
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    assert_eq!(e.state.notes.len(), 1);
    assert_eq!(e.state.notes[&r.note_id].path, path);
    let count = fs::read_dir(&c.folders[0])
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().is_file())
        .count();
    assert_eq!(count, 1);
    let mut bad = data;
    bad.filename = "../outside.txt".into();
    let r = Revision::seal(&key, &a, &v.id, &opaque_id(), None, Some(&bad)).unwrap();
    remote.upload(&r).unwrap();
    assert!(e.run(&remote, &c, &key, &s, &[]).is_err());
    assert!(!c.folders[0].parent().unwrap().join("outside.txt").exists());
}

#[test]
fn edit_delete_conflict_keeps_edit_as_new_identity_and_replays_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    let p1 = c1.folders[0].join("n.txt");
    let p2 = c2.folders[0].join("n.txt");
    fs::write(&p1, "original").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    fs::write(&p2, "offline edit").unwrap();
    fs::remove_file(&p1).unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert!(!p2.exists());
    assert!(fs::read_dir(&c2.folders[0])
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|body| body == "offline edit"));
    let old = remote.revisions.borrow()[0].clone();
    remote.revisions.borrow_mut().push(old);
    assert!(e2.run(&remote, &c2, &key, &s2, &[]).is_err());
}

#[test]
fn protected_editor_delays_apply_without_losing_cursor_or_local_edits() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    let p1 = c1.folders[0].join("n.txt");
    let p2 = c2.folders[0].join("n.txt");
    fs::write(&p1, "original").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    fs::write(&p1, "remote edit").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, std::slice::from_ref(&p2))
        .unwrap();
    assert_eq!(e2.state.cursor, remote.revisions.borrow().len() as u64);
    assert_eq!(e2.state.deferred.len(), 1);
    assert_eq!(fs::read_to_string(&p2).unwrap(), "original");
    // A cached page must not hide a server rollback while application is deferred.
    let latest = remote.revisions.borrow_mut().pop().unwrap();
    assert!(e2.run(&remote, &c2, &key, &s2, &[]).is_err());
    remote.revisions.borrow_mut().push(latest);
    fs::write(&p2, "edit while deferred").unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(p2).unwrap(), "remote edit");
    assert!(fs::read_dir(&c2.folders[0])
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|body| body == "edit while deferred"));
}

struct DuringFetch<'a> {
    remote: &'a Memory,
    calls: std::cell::Cell<usize>,
    action: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
}

#[test]
fn multiple_folders_empty_collections_same_names_and_moves_keep_stable_ids() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, mut c1, mut s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, mut c2, s2) = setup(temp.path(), "two", &a, &v);
    let work = temp.path().join("one/work/Shared");
    let other = temp.path().join("one/other/Shared");
    let empty = temp.path().join("one/Empty");
    for path in [&work, &other, &empty] {
        fs::create_dir_all(path).unwrap();
        c1.folders.push(path.clone());
        s1.add_folder(path.clone(), false);
    }
    s1.folder_colors.insert(work.clone(), "#aabbcc".into());
    fs::write(work.join("same.txt"), "work body").unwrap();
    fs::write(other.join("same.txt"), "other body").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let folder_id = e1
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == work)
        .unwrap()
        .0
        .clone();
    let other_id = e1
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == other)
        .unwrap()
        .0
        .clone();
    let empty_id = e1
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == empty)
        .unwrap()
        .0
        .clone();
    let note_id = e1
        .state
        .notes
        .iter()
        .find(|(_, t)| t.path == work.join("same.txt"))
        .unwrap()
        .0
        .clone();
    c2.download_dir = Some(temp.path().join("two/downloaded"));
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    let downloaded = e2.state.bindings[&folder_id].path.clone();
    let downloaded_other = e2.state.bindings[&other_id].path.clone();
    assert_ne!(downloaded, downloaded_other);
    assert_eq!(
        fs::read_to_string(downloaded.join("same.txt")).unwrap(),
        "work body"
    );
    assert_eq!(
        fs::read_to_string(downloaded_other.join("same.txt")).unwrap(),
        "other body"
    );
    assert!(e2.state.bindings[&empty_id].path.is_dir());
    let mut view = s2.clone();
    e2.folder_settings(&c2, &mut view);
    assert_eq!(view.folder_labels[&downloaded], "Shared");
    assert_eq!(view.folder_colors[&downloaded], "#aabbcc");
    let renamed = work.with_file_name("Projects");
    fs::rename(&work, &renamed).unwrap();
    let service = crate::control::Service::new(&temp.path().join("one/settings.json"));
    service.relocate(&work, &renamed).unwrap();
    c1.folders.retain(|p| p != &work);
    c1.folders.push(renamed.clone());
    s1.rename_folder(&work, &renamed);
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(
        e1.state.catalog.as_ref().unwrap().folders[&folder_id].name,
        "Projects"
    );
    assert_eq!(e2.state.bindings[&folder_id].path, downloaded);
    e2.folder_settings(&c2, &mut view);
    assert_eq!(view.folder_labels[&downloaded], "Projects");
    let old = renamed.join("same.txt");
    let moved = Repository.move_note(&old, &c1.folders[0]).unwrap();
    service.relocate(&old, &moved.path).unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(
        e1.state.notes[&note_id]
            .baseline
            .as_ref()
            .unwrap()
            .folder_id
            .as_deref(),
        Some("AAAAAAAAAAAAAAAAAAAAAA")
    );
    assert_eq!(
        e2.state.notes[&note_id].path.parent(),
        Some(c2.folders[0].as_path())
    );
    assert!(!downloaded.join("same.txt").exists());
}

#[test]
fn unselecting_preserves_pending_edits_and_does_not_block_other_folders() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, mut c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, mut c2, s2) = setup(temp.path(), "two", &a, &v);
    let extra = temp.path().join("one/Extra");
    fs::create_dir_all(&extra).unwrap();
    c1.folders.push(extra.clone());
    fs::write(c1.folders[0].join("base.txt"), "base").unwrap();
    fs::write(extra.join("extra.txt"), "extra").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    let folder = e1
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == extra)
        .unwrap()
        .0
        .clone();
    let path = e2.state.bindings[&folder].path.join("extra.txt");
    fs::write(&path, "pending local edit").unwrap();
    let access = e2.local.clone();
    {
        let _guard = access.lock().unwrap();
        e2.scan(&c2, &key, &s2, &[]).unwrap();
    }
    assert_eq!(e2.state.outgoing.len(), 1);
    c2.excluded_folders.insert(folder.clone());
    fs::write(c2.folders[0].join("base.txt"), "base local edit").unwrap();
    fs::write(extra.join("extra.txt"), "remote extra edit").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "pending local edit");
    assert_eq!(e2.state.outgoing.len(), 1);
    assert!(!e2.state.deferred.is_empty());
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(c1.folders[0].join("base.txt")).unwrap(),
        "base local edit"
    );
    c2.excluded_folders.remove(&folder);
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "remote extra edit");
    assert!(fs::read_dir(&e2.state.bindings[&folder].path)
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|body| body == "pending local edit"));
}

#[test]
fn an_old_idempotent_upload_receipt_cannot_erase_a_newer_deferred_head() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, mut c2, s2) = setup(temp.path(), "two", &a, &v);
    let first = c1.folders[0].join("n.txt");
    let second = c2.folders[0].join("n.txt");
    fs::write(&first, "base").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    fs::write(&second, "accepted but response lost").unwrap();
    e2.scan(&c2, &key, &s2, &[]).unwrap();
    remote.upload(&e2.state.outgoing[0]).unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    fs::write(&first, "newer remote edit").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    c2.excluded_folders.insert("AAAAAAAAAAAAAAAAAAAAAA".into());
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(e2.state.outgoing.len(), 1);
    assert_eq!(e2.state.deferred.len(), 1);
    c2.excluded_folders.clear();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(second).unwrap(), "newer remote edit");
    assert!(e2.state.deferred.is_empty());
}

#[test]
fn legacy_migration_preserves_ids_and_unknown_assignments_require_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e, mut c, s) = setup(temp.path(), "one", &a, &v);
    let data = NoteData {
        version: 1,
        filename: "legacy.txt".into(),
        folder: "Old Work".into(),
        folder_id: None,
        body: "legacy body".into(),
        modified_ms: 1,
        pinned: false,
        language: None,
    };
    let id = opaque_id();
    let revision = Revision::seal(&key, &a, &v.id, &id, None, Some(&data)).unwrap();
    remote.upload(&revision).unwrap();
    assert!(e.run(&remote, &c, &key, &s, &[]).is_err());
    assert_eq!(remote.revisions.borrow().len(), 1);
    assert!(!c.folders[0].join("legacy.txt").exists());
    assert_eq!(e.state.legacy_pending[0].id, id);
    c.legacy_note_mappings
        .insert(id.clone(), "AAAAAAAAAAAAAAAAAAAAAA".into());
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(c.folders[0].join("legacy.txt")).unwrap(),
        "legacy body"
    );
    assert_eq!(
        e.state.notes[&id].head.as_deref(),
        Some(revision.id.as_str())
    );
    assert_eq!(
        e.state.catalog.as_ref().unwrap().legacy_notes[&id],
        "AAAAAAAAAAAAAAAAAAAAAA"
    );
    let mut download = Config {
        version: 2,
        server: c.server.clone(),
        issuer: c.issuer.clone(),
        client_id: c.client_id.clone(),
        scopes: c.scopes.clone(),
        download_dir: Some(temp.path().join("fresh/downloads")),
        ..Config::default()
    };
    let mut fresh = Engine::open(temp.path().join("fresh/sync/state.json")).unwrap();
    fresh.bind(&a, &v).unwrap();
    fresh
        .run(
            &remote,
            &download,
            &key,
            &Settings::new(temp.path().join("fresh/notes")),
            &[],
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(&fresh.state.notes[&id].path).unwrap(),
        "legacy body"
    );
    let reserved = fresh.state.bindings["AAAAAAAAAAAAAAAAAAAAAA"].path.clone();
    // Explicit remapping copies notes and never removes the old working file.
    let remap = temp.path().join("fresh/remapped");
    fs::create_dir_all(&remap).unwrap();
    download
        .folder_mappings
        .insert("AAAAAAAAAAAAAAAAAAAAAA".into(), remap.clone());
    fresh
        .run(
            &remote,
            &download,
            &key,
            &Settings::new(temp.path().join("fresh/notes")),
            &[],
        )
        .unwrap();
    assert!(reserved.join("legacy.txt").exists());
    assert_eq!(fresh.state.notes[&id].path.parent(), Some(remap.as_path()));
}

#[test]
fn existing_download_directories_are_not_silently_merged() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut source, c, s) = setup(temp.path(), "one", &a, &v);
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    let root = temp.path().join("downloads");
    fs::create_dir_all(root.join("Notes")).unwrap();
    fs::write(root.join("Notes/keep.txt"), "keep").unwrap();
    let mut target = Engine::open(temp.path().join("target/sync/state.json")).unwrap();
    target.bind(&a, &v).unwrap();
    let config = Config {
        version: 2,
        server: c.server,
        issuer: c.issuer,
        client_id: c.client_id,
        scopes: c.scopes,
        download_dir: Some(root.clone()),
        ..Config::default()
    };
    assert!(target.run(&remote, &config, &key, &s, &[]).is_err());
    assert_eq!(
        fs::read_to_string(root.join("Notes/keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn selective_download_can_read_the_catalog_without_importing_unselected_folders() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut source, c, s) = setup(temp.path(), "one", &a, &v);
    fs::write(c.folders[0].join("note.txt"), "private").unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    let mut target = Engine::open(temp.path().join("target/sync/state.json")).unwrap();
    target.bind(&a, &v).unwrap();
    let mut config = Config {
        version: 2,
        server: c.server,
        issuer: c.issuer,
        client_id: c.client_id,
        scopes: c.scopes,
        download_dir: Some(temp.path().join("download")),
        auto_download: false,
        ..Config::default()
    };
    let settings = Settings::new(temp.path().join("target/notes"));
    target.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(target.state.catalog.is_some());
    assert!(target.state.bindings.is_empty());
    assert_eq!(target.state.deferred.len(), 1);
    assert!(!config.download_dir.as_ref().unwrap().exists());
    config
        .selected_folders
        .insert("AAAAAAAAAAAAAAAAAAAAAA".into());
    target.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(target.state.deferred.is_empty());
    assert_eq!(
        fs::read_to_string(
            target.state.bindings["AAAAAAAAAAAAAAAAAAAAAA"]
                .path
                .join("note.txt")
        )
        .unwrap(),
        "private"
    );
}

#[test]
fn a_note_can_move_from_an_unselected_folder_into_a_selected_one_then_be_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut source, mut c, s) = setup(temp.path(), "one", &a, &v);
    let selected = temp.path().join("one/Selected");
    fs::create_dir_all(&selected).unwrap();
    c.folders.push(selected.clone());
    let old = c.folders[0].join("move.txt");
    fs::write(&old, "moving body").unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    let folder = source
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == selected)
        .unwrap()
        .0
        .clone();
    let id = source
        .state
        .notes
        .iter()
        .find(|(_, n)| n.path == old)
        .unwrap()
        .0
        .clone();
    let mut config = Config {
        version: 2,
        server: c.server.clone(),
        issuer: c.issuer.clone(),
        client_id: c.client_id.clone(),
        scopes: c.scopes.clone(),
        auto_download: false,
        download_dir: Some(temp.path().join("target/downloads")),
        ..Config::default()
    };
    config.selected_folders.insert(folder.clone());
    let mut target = Engine::open(temp.path().join("target/sync/state.json")).unwrap();
    target.bind(&a, &v).unwrap();
    let settings = Settings::new(temp.path().join("target/notes"));
    target.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(!target.state.notes.contains_key(&id));
    assert_eq!(target.state.deferred.len(), 1);
    let moved = Repository.move_note(&old, &selected).unwrap();
    let service = crate::control::Service::new(&temp.path().join("one/settings.json"));
    service.relocate(&old, &moved.path).unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    target.run(&remote, &config, &key, &settings, &[]).unwrap();
    let path = target.state.bindings[&folder].path.join("move.txt");
    assert_eq!(fs::read_to_string(&path).unwrap(), "moving body");
    assert!(target.state.deferred.is_empty());
    fs::remove_file(&moved.path).unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    target.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(!path.exists());
}

#[test]
fn interrupted_initial_history_download_does_not_publish_a_note_before_its_tombstone() {
    struct Paged<'a> {
        remote: &'a Memory,
        calls: std::cell::Cell<usize>,
        fail_at: usize,
    }
    impl Remote for Paged<'_> {
        fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == self.fail_at {
                return Err(error("simulated interruption before tombstone page"));
            }
            let mut page = self.remote.changes(c, t)?;
            page.changes.truncate(1);
            page.next = page.changes.last().map(|ch| ch.sequence).unwrap_or(c);
            Ok(page)
        }
        fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
            self.remote.upload(r)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut source, c, s) = setup(temp.path(), "one", &a, &v);
    let path = c.folders[0].join("deleted.txt");
    fs::write(&path, "must not reappear").unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    fs::remove_file(path).unwrap();
    source.run(&remote, &c, &key, &s, &[]).unwrap();
    let config = Config {
        version: 2,
        server: c.server,
        issuer: c.issuer,
        client_id: c.client_id,
        scopes: c.scopes,
        download_dir: Some(temp.path().join("target/downloads")),
        ..Config::default()
    };
    let journal = temp.path().join("target/sync/state.json");
    let mut target = Engine::open(journal.clone()).unwrap();
    target.bind(&a, &v).unwrap();
    let settings = Settings::new(temp.path().join("target/notes"));
    let paged = Paged {
        remote: &remote,
        calls: std::cell::Cell::new(0),
        fail_at: 6,
    };
    assert!(target.run(&paged, &config, &key, &settings, &[]).is_err());
    assert!(target.state.bootstrap_pending);
    assert!(target.state.notes.is_empty());
    let folder = target.state.bindings["AAAAAAAAAAAAAAAAAAAAAA"].path.clone();
    assert!(!folder.join("deleted.txt").exists());
    drop(target);
    let mut target = Engine::open(journal).unwrap();
    let paged = Paged {
        remote: &remote,
        calls: std::cell::Cell::new(0),
        fail_at: usize::MAX,
    };
    target.run(&paged, &config, &key, &settings, &[]).unwrap();
    assert!(!target.state.bootstrap_pending);
    assert!(!folder.join("deleted.txt").exists());
}

#[test]
fn several_remote_edits_to_the_open_note_validate_the_chain_but_apply_only_the_latest() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    let first = c1.folders[0].join("note.txt");
    let second = c2.folders[0].join("note.txt");
    fs::write(&first, "original").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    for text in ["remote one", "remote two", "remote three"] {
        fs::write(&first, text).unwrap();
        e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    }
    e2.run(&remote, &c2, &key, &s2, std::slice::from_ref(&second))
        .unwrap();
    assert_eq!(e2.state.deferred.len(), 1);
    assert_eq!(fs::read_to_string(&second).unwrap(), "original");
    fs::write(&second, "local while reading").unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(fs::read_to_string(&second).unwrap(), "remote three");
    assert!(e2.state.deferred.is_empty());
    assert!(fs::read_dir(&c2.folders[0])
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|text| text == "local while reading"));
}

#[test]
fn simultaneous_initial_catalogs_adopt_the_winning_confirmed_legacy_ids() {
    struct Race<'a> {
        remote: &'a Memory,
        winner: Revision,
        fired: std::cell::Cell<bool>,
    }
    impl Remote for Race<'_> {
        fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
            self.remote.changes(c, t)
        }
        fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
            if r.note_id == r.vault_id && !self.fired.replace(true) {
                self.remote.upload(&self.winner)?;
            }
            self.remote.upload(r)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e, c, s) = setup(temp.path(), "one", &a, &v);
    let note_id = opaque_id();
    let data = NoteData {
        version: 1,
        filename: "legacy.txt".into(),
        folder: "Notes".into(),
        folder_id: None,
        body: "unchanged".into(),
        modified_ms: 1,
        pinned: false,
        language: None,
    };
    let old = Revision::seal(&key, &a, &v.id, &note_id, None, Some(&data)).unwrap();
    remote.upload(&old).unwrap();
    fs::write(c.folders[0].join("legacy.txt"), &data.body).unwrap();
    e.state.notes.insert(
        note_id.clone(),
        Tracked {
            path: c.folders[0].join("legacy.txt"),
            head: Some(old.id.clone()),
            baseline: Some(data),
            deleted: false,
        },
    );
    e.remember(&old, 1).unwrap();
    e.state.cursor = 1;
    e.state.through = 1;
    e.save().unwrap();
    let winner_id = opaque_id();
    let catalog = FolderCatalog {
        version: FOLDER_FORMAT,
        folders: BTreeMap::from([(
            winner_id.clone(),
            papertrail_crypto::FolderData {
                name: "Notes".into(),
                color: "preset-0".into(),
            },
        )]),
        legacy_notes: BTreeMap::from([(note_id.clone(), winner_id.clone())]),
        deleted_folders: BTreeSet::new(),
    };
    let winner = Revision::seal_record(
        &key,
        &a,
        &v.id,
        &v.id,
        None,
        &Record::FolderCatalog(catalog),
    )
    .unwrap();
    let race = Race {
        remote: &remote,
        winner,
        fired: std::cell::Cell::new(false),
    };
    assert!(e.run(&race, &c, &key, &s, &[]).is_err());
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    assert_eq!(e.state.catalog.as_ref().unwrap().folders.len(), 1);
    assert!(e
        .state
        .catalog
        .as_ref()
        .unwrap()
        .folders
        .contains_key(&winner_id));
    assert_eq!(
        e.state.notes[&note_id]
            .baseline
            .as_ref()
            .unwrap()
            .folder_id
            .as_ref(),
        Some(&winner_id)
    );
    assert_eq!(e.state.notes[&note_id].head.as_ref(), Some(&old.id));
    assert_eq!(remote.revisions.borrow().len(), 2);
}

#[test]
fn a_lost_catalog_response_keeps_folder_ids_stable_across_restart() {
    struct LostResponse<'a> {
        remote: &'a Memory,
        lost: std::cell::Cell<bool>,
    }
    impl Remote for LostResponse<'_> {
        fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
            self.remote.changes(c, t)
        }
        fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
            let result = self.remote.upload(r)?;
            if r.note_id == r.vault_id && !self.lost.replace(true) {
                return Err(error("simulated response loss"));
            }
            Ok(result)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, mut c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, mut c2, mut s2) = setup(temp.path(), "two", &a, &v);
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    let remote_new = temp.path().join("one/Remote new");
    fs::create_dir_all(&remote_new).unwrap();
    c1.folders.push(remote_new);
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let local_new = temp.path().join("two/Local new");
    fs::create_dir_all(&local_new).unwrap();
    c2.folders.push(local_new.clone());
    let lost = LostResponse {
        remote: &remote,
        lost: std::cell::Cell::new(false),
    };
    assert!(e2.run(&lost, &c2, &key, &s2, &[]).is_err());
    let journal = e2.state_path.clone();
    let id = e2
        .state
        .bindings
        .iter()
        .find(|(_, b)| b.path == local_new)
        .unwrap()
        .0
        .clone();
    drop(e2);
    let mut e2 = Engine::open(journal.clone()).unwrap();
    assert!(e2.state.bindings.contains_key(&id));
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(e2.state.catalog.as_ref().unwrap().folders.len(), 3);
    assert_eq!(e2.state.bindings[&id].path, local_new);
    s2.folder_labels
        .insert(local_new.clone(), "Rename with lost response".into());
    let lost = LostResponse {
        remote: &remote,
        lost: std::cell::Cell::new(false),
    };
    assert!(e2.run(&lost, &c2, &key, &s2, &[]).is_err());
    let published = remote.revisions.borrow().len();
    drop(e2);
    let mut e2 = Engine::open(journal).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    e2.folder_settings(&c2, &mut s2);
    assert_eq!(s2.folder_name(&local_new), "Rename with lost response");
    assert_eq!(e2.state.bindings[&id].path, local_new);
    assert_eq!(remote.revisions.borrow().len(), published);
}

#[test]
fn display_label_renames_survive_sync_restart_and_differently_named_device_mappings() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut one, c1, mut s1) = setup(temp.path(), "one", &a, &v);
    let (mut two, mut c2, mut s2) = setup(temp.path(), "two", &a, &v);
    let id = "AAAAAAAAAAAAAAAAAAAAAA";
    let first_path = c1.folders[0].clone();
    let second_path = temp.path().join("Local inbox");
    fs::create_dir(&second_path).unwrap();
    s2.add_folder(second_path.clone(), false);
    c2.folders = vec![second_path.clone()];
    c2.folder_mappings.insert(id.into(), second_path.clone());
    fs::write(first_path.join("keep.txt"), "keep my note").unwrap();
    s1.folder_labels
        .insert(first_path.clone(), "Before first sync".into());
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    assert_eq!(
        one.state.catalog.as_ref().unwrap().folders[id].name,
        "Before first sync"
    );
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    two.folder_settings(&c2, &mut s2);
    assert_eq!(s2.folder_name(&second_path), "Before first sync");

    // iOS edits only folder_labels, not the physical directory.
    s1.folder_labels
        .insert(first_path.clone(), "Renamed on iOS".into());
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    one.folder_settings(&c1, &mut s1);
    assert_eq!(s1.folder_name(&first_path), "Renamed on iOS");
    let journal = one.state_path.clone();
    let note_ids: BTreeSet<_> = one.state.notes.keys().cloned().collect();
    drop(one);
    let mut one = Engine::open(journal.clone()).unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    two.folder_settings(&c2, &mut s2);
    assert_eq!(s2.folder_name(&second_path), "Renamed on iOS");
    let revisions = remote.revisions.borrow().len();
    // Downloaded labels must not be mistaken for a new local rename on retry.
    for _ in 0..2 {
        one.run(&remote, &c1, &key, &s1, &[]).unwrap();
        two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    }
    assert_eq!(remote.revisions.borrow().len(), revisions);
    assert_eq!(
        one.state.notes.keys().cloned().collect::<BTreeSet<_>>(),
        note_ids
    );
    assert_eq!(one.state.bindings[id].path, first_path);
    assert_eq!(two.state.bindings[id].path, second_path);
    assert_eq!(
        fs::read_to_string(first_path.join("keep.txt")).unwrap(),
        "keep my note"
    );
    assert_eq!(
        fs::read_to_string(second_path.join("keep.txt")).unwrap(),
        "keep my note"
    );

    // Old journals have no display-label observation. A pending rename must
    // still be published on the first sync after upgrade.
    drop(one);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    legacy["bindings"][id]
        .as_object_mut()
        .unwrap()
        .remove("observed_label");
    fs::write(&journal, serde_json::to_vec(&legacy).unwrap()).unwrap();
    s1.folder_labels
        .insert(first_path.clone(), "Rename from old journal".into());
    let mut one = Engine::open(journal).unwrap();
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    assert_eq!(
        one.state.catalog.as_ref().unwrap().folders[id].name,
        "Rename from old journal"
    );
    assert!(first_path.is_dir());
}

#[test]
fn new_and_paused_collections_preserve_display_labels_without_renaming_directories() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, mut config, mut settings) = setup(temp.path(), "device", &a, &v);
    engine.state.bindings.clear();
    engine.state.catalog_draft = None;
    let path = config.folders[0].clone();
    settings
        .folder_labels
        .insert(path.clone(), "Custom name before sync".into());
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    let id = engine.state.bindings.keys().next().unwrap().clone();
    assert_eq!(
        engine.state.catalog.as_ref().unwrap().folders[&id].name,
        "Custom name before sync"
    );
    config.excluded_folders.insert(id.clone());
    settings
        .folder_labels
        .insert(path.clone(), "Renamed while paused".into());
    let before = remote.revisions.borrow().len();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    engine.folder_settings(&config, &mut settings);
    assert_eq!(remote.revisions.borrow().len(), before);
    assert_eq!(settings.folder_name(&path), "Renamed while paused");
    config.excluded_folders.remove(&id);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    engine.folder_settings(&config, &mut settings);
    assert_eq!(settings.folder_name(&path), "Renamed while paused");
    // Clearing a label restores the physical basename logically; it does not
    // move or rename the directory.
    settings.folder_labels.remove(&path);
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert_eq!(
        engine.state.catalog.as_ref().unwrap().folders[&id].name,
        "Notes"
    );
    assert_eq!(engine.state.bindings[&id].path, path);
    assert!(path.is_dir());
}

#[test]
fn concurrent_display_label_renames_require_confirmation_and_remote_updates_do_not_echo() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut one, c1, mut s1) = setup(temp.path(), "one", &a, &v);
    let (mut two, c2, mut s2) = setup(temp.path(), "two", &a, &v);
    let id = "AAAAAAAAAAAAAAAAAAAAAA";
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    one.folder_settings(&c1, &mut s1);
    two.folder_settings(&c2, &mut s2);
    s1.folder_labels
        .insert(c1.folders[0].clone(), "First rename".into());
    s2.folder_labels
        .insert(c2.folders[0].clone(), "Second rename".into());
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    assert!(two.run(&remote, &c2, &key, &s2, &[]).is_err());
    assert!(two.state.folder_conflicts.contains(id));
    assert_eq!(s2.folder_name(&c2.folders[0]), "Second rename");
    let journal = two.state_path.clone();
    drop(two);
    let service = crate::control::Service::new_session_only(&temp.path().join("two/settings.json"));
    service.accept_remote_folder(id, &s2).unwrap();
    let mut two = Engine::open(journal).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    two.folder_settings(&c2, &mut s2);
    assert_eq!(s2.folder_name(&c2.folders[0]), "First rename");
    // A second remote rename arrives before another local preparation. The
    // settings label from the previous download is not a competing local edit.
    s1.folder_labels
        .insert(c1.folders[0].clone(), "Third rename".into());
    one.run(&remote, &c1, &key, &s1, &[]).unwrap();
    two.run(&remote, &c2, &key, &s2, &[]).unwrap();
    two.folder_settings(&c2, &mut s2);
    assert_eq!(s2.folder_name(&c2.folders[0]), "Third rename");
    assert!(two.state.folder_conflicts.is_empty());
    assert!(c1.folders[0].is_dir() && c2.folders[0].is_dir());
}

#[test]
fn concurrent_folder_renames_require_explicit_resolution_without_moving_directories() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, mut c1, mut s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, mut c2, mut s2) = setup(temp.path(), "two", &a, &v);
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    let first = c1.folders[0].with_file_name("First rename");
    fs::rename(&c1.folders[0], &first).unwrap();
    let service1 = crate::control::Service::new(&temp.path().join("one/settings.json"));
    service1.relocate(&c1.folders[0], &first).unwrap();
    s1.rename_folder(&c1.folders[0], &first);
    c1.folders[0] = first;
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let second = c2.folders[0].with_file_name("Second rename");
    fs::rename(&c2.folders[0], &second).unwrap();
    let service2 = crate::control::Service::new(&temp.path().join("two/settings.json"));
    service2.relocate(&c2.folders[0], &second).unwrap();
    s2.rename_folder(&c2.folders[0], &second);
    c2.folders[0] = second.clone();
    assert!(e2.run(&remote, &c2, &key, &s2, &[]).is_err());
    assert!(e2.state.folder_conflicts.contains("AAAAAAAAAAAAAAAAAAAAAA"));
    let journal = e2.state_path.clone();
    drop(e2);
    service2
        .accept_remote_folder("AAAAAAAAAAAAAAAAAAAAAA", &s2)
        .unwrap();
    let mut e2 = Engine::open(journal).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(
        e2.state.catalog.as_ref().unwrap().folders["AAAAAAAAAAAAAAAAAAAAAA"].name,
        "First rename"
    );
    assert_eq!(e2.state.bindings["AAAAAAAAAAAAAAAAAAAAAA"].path, second);
    assert!(second.is_dir());
}
impl Remote for DuringFetch<'_> {
    fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 2 {
            if let Some(action) = self.action.borrow_mut().take() {
                action();
            }
        }
        self.remote.changes(c, t)
    }
    fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
        self.remote.upload(r)
    }
}

#[test]
fn navigation_during_network_protects_the_new_editor_and_other_notes_keep_syncing() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    for name in ["a.txt", "b.txt"] {
        fs::write(c1.folders[0].join(name), "original").unwrap();
    }
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    for name in ["a.txt", "b.txt"] {
        fs::write(c1.folders[0].join(name), "remote edit").unwrap();
    }
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let access = e2.local.clone();
    access.lock().unwrap().protected = vec![c2.folders[0].join("a.txt")];
    let b = c2.folders[0].join("b.txt");
    let opened = b.clone();
    let during = DuringFetch {
        remote: &remote,
        calls: std::cell::Cell::new(0),
        action: RefCell::new(Some(Box::new(move || {
            // This must succeed immediately: HTTP must not hold the local file lease.
            access.try_lock().unwrap().protected = vec![opened];
        }))),
    };
    e2.run(&during, &c2, &key, &s2, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(c2.folders[0].join("a.txt")).unwrap(),
        "remote edit"
    );
    assert_eq!(fs::read_to_string(&b).unwrap(), "original");
    assert_eq!(e2.state.deferred.len(), 1);
    assert_eq!(e2.state.cursor, remote.revisions.borrow().len() as u64);
    e2.local.lock().unwrap().protected.clear();
    fs::write(&b, "saved while working").unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    assert!(e2.state.deferred.is_empty());
    assert_eq!(fs::read_to_string(b).unwrap(), "remote edit");
    assert!(fs::read_dir(&c2.folders[0])
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|body| body == "saved while working"));
}

#[test]
fn rename_during_network_preserves_note_identity_without_waiting_for_the_worker() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e, c, s) = setup(temp.path(), "device", &a, &v);
    let old = c.folders[0].join("old.txt");
    let new = c.folders[0].join("new.txt");
    fs::write(&old, "original").unwrap();
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    let id = e.state.notes.keys().next().unwrap().clone();
    fs::write(&old, "saved snapshot").unwrap();
    let service = crate::control::Service::new(&temp.path().join("device/settings.json"));
    let old_copy = old.clone();
    let new_copy = new.clone();
    let during = DuringFetch {
        remote: &remote,
        calls: std::cell::Cell::new(0),
        action: RefCell::new(Some(Box::new(move || {
            fs::rename(&old_copy, &new_copy).unwrap();
            service.relocate(&old_copy, &new_copy).unwrap();
            fs::write(&new_copy, "newer local edit").unwrap();
        }))),
    };
    e.run(&during, &c, &key, &s, &[]).unwrap();
    assert_eq!(e.state.notes[&id].path, new);
    assert_eq!(e.state.notes[&id].path.as_os_str(), new.as_os_str());
    e.run(&remote, &c, &key, &s, &[]).unwrap();
    assert_eq!(e.state.notes.len(), 1);
    assert_eq!(
        remote
            .revisions
            .borrow()
            .last()
            .unwrap()
            .open(&key, &a)
            .unwrap()
            .unwrap()
            .body,
        "newer local edit"
    );
    assert!(!old.exists());
}

#[test]
fn deletion_during_fetch_does_not_recreate_the_deleted_path() {
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    let remote = Memory::default();
    let (mut e1, c1, s1) = setup(temp.path(), "one", &a, &v);
    let (mut e2, c2, s2) = setup(temp.path(), "two", &a, &v);
    let p1 = c1.folders[0].join("n.txt");
    let p2 = c2.folders[0].join("n.txt");
    fs::write(&p1, "original").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    fs::write(&p1, "remote edit").unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    let deleted = p2.clone();
    let during = DuringFetch {
        remote: &remote,
        calls: std::cell::Cell::new(0),
        action: RefCell::new(Some(Box::new(move || fs::remove_file(deleted).unwrap()))),
    };
    e2.run(&during, &c2, &key, &s2, &[]).unwrap();
    assert!(!p2.exists());
    assert!(fs::read_dir(&c2.folders[0])
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
        .any(|body| body == "remote edit"));
    e2.run(&remote, &c2, &key, &s2, &[]).unwrap();
    e1.run(&remote, &c1, &key, &s1, &[]).unwrap();
    assert!(!p1.exists());
}

#[test]
fn closing_during_upload_stops_the_remaining_outbox_without_deleting_or_forgetting_notes() {
    struct CloseAfterUpload<'a> {
        remote: &'a Memory,
        service: crate::control::Service,
        path: PathBuf,
        closed: std::cell::Cell<bool>,
    }
    impl Remote for CloseAfterUpload<'_> {
        fn changes(&self, c: u64, t: Option<u64>) -> io::Result<Changes> {
            self.remote.changes(c, t)
        }
        fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
            let result = self.remote.upload(r)?;
            if !self.closed.replace(true) {
                self.service.pause_folder(&self.path)?;
            }
            Ok(result)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let a = Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    };
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    let remote = Memory::default();
    let (mut engine, config, settings) = setup(temp.path(), "device", &a, &vault);
    fs::write(
        temp.path().join("device/sync/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    fs::write(config.folders[0].join("one"), "one").unwrap();
    fs::write(config.folders[0].join("two"), "two").unwrap();
    let service =
        crate::control::Service::new_session_only(&temp.path().join("device/settings.json"));
    let closing = CloseAfterUpload {
        remote: &remote,
        service: service.clone(),
        path: config.folders[0].clone(),
        closed: Default::default(),
    };
    assert_eq!(
        engine
            .run(&closing, &config, &key, &settings, &[])
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(engine.state.outgoing.len(), 1);
    let count = remote.revisions.borrow().len();
    let paused = service.saved_config().unwrap().unwrap();
    engine.run(&remote, &paused, &key, &settings, &[]).unwrap();
    assert_eq!(remote.revisions.borrow().len(), count);
    assert_eq!(engine.state.outgoing.len(), 1);
    assert_eq!(
        fs::read_to_string(config.folders[0].join("one")).unwrap(),
        "one"
    );
    assert_eq!(
        fs::read_to_string(config.folders[0].join("two")).unwrap(),
        "two"
    );
    let other = temp.path().join("Other");
    fs::create_dir(&other).unwrap();
    let mut closed_settings = Settings::new(other);
    engine.folder_settings(&paused, &mut closed_settings);
    assert!(!closed_settings.note_folders.contains(&config.folders[0]));
    // Explicit selection resumes the same identities and durable pending edit.
    fs::write(
        temp.path().join("device/sync/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    engine.run(&remote, &config, &key, &settings, &[]).unwrap();
    assert!(engine.state.outgoing.is_empty());
    assert_eq!(remote.revisions.borrow().len(), count + 1);
}
