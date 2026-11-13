use papertrail_crypto::*;
fn account() -> Account {
    Account {
        iss: "https://issuer.example".into(),
        sub: "alice".into(),
    }
}
fn note() -> NoteData {
    NoteData {
        version: 1,
        filename: "private.txt".into(),
        folder: "Notes".into(),
        folder_id: None,
        body: "Title\nsecret 🦀".into(),
        modified_ms: 1,
        pinned: true,
        language: None,
    }
}
#[test]
fn revisions_authenticate_every_binding_and_use_fresh_nonces() {
    let a = account();
    let key = Secret::random();
    let vault = opaque_id();
    let id = opaque_id();
    let r = Revision::seal(&key, &a, &vault, &id, None, Some(&note())).unwrap();
    assert!(r.open(&key, &a).unwrap() == Some(note()));
    let r2 = Revision::seal(&key, &a, &vault, &id, None, Some(&note())).unwrap();
    assert_ne!(r.blob.nonce, r2.blob.nonce);
    assert!(r.open(&Secret::random(), &a).is_err());
    let mut other = a.clone();
    other.sub = "bob".into();
    assert!(r.open(&key, &other).is_err());
    let mut other_issuer = a.clone();
    other_issuer.iss = "https://another-issuer.example".into();
    assert!(r.open(&key, &other_issuer).is_err());
    for field in 0..6 {
        let mut altered = r.clone();
        match field {
            0 => altered.note_id = opaque_id(),
            1 => altered.id = opaque_id(),
            2 => altered.parent = Some(opaque_id()),
            3 => altered.deleted = true,
            4 => altered.vault_id = opaque_id(),
            _ => altered.blob.ciphertext.replace_range(
                0..1,
                if altered.blob.ciphertext.starts_with('A') {
                    "B"
                } else {
                    "A"
                },
            ),
        }
        assert!(altered.open(&key, &a).is_err());
    }
    let tombstone = Revision::seal(&key, &a, &vault, &id, Some(r.id), None).unwrap();
    assert!(tombstone.open(&key, &a).unwrap().is_none());
}
#[test]
fn passphrase_recovery_rewrap_and_versions() {
    let a = account();
    let (v, key, recovery) = Vault::create(&a, "a long and strong vault passphrase").unwrap();
    assert_eq!(
        key.0,
        v.unlock(&a, "a long and strong vault passphrase")
            .unwrap()
            .0
    );
    assert!(v.unlock(&a, "wrong").is_err());
    assert_eq!(key.0, v.recover(&a, &recovery).unwrap().0);
    assert!(v.recover(&a, &Secret::random()).is_err());
    let mut other_issuer = a.clone();
    other_issuer.iss = "https://another-issuer.example".into();
    assert!(v.recover(&other_issuer, &recovery).is_err());
    let (v2, new_recovery) = v.rewrap(&a, "new strong vault passphrase", &key).unwrap();
    assert_eq!(
        key.0,
        v2.unlock(&a, "new strong vault passphrase").unwrap().0
    );
    assert_eq!(key.0, v2.recover(&a, &new_recovery).unwrap().0);
    assert!(v2.recover(&a, &recovery).is_err());
    let mut invalid = v;
    invalid.version = 99;
    assert!(matches!(invalid.unlock(&a, "x"), Err(Error::Version)));
    invalid.version = 1;
    invalid.kdf.memory_kib = u32::MAX;
    assert!(invalid.unlock(&a, "x").is_err());
}

#[test]
fn v2_folder_catalog_and_note_membership_are_authenticated_and_versioned() {
    let a = account();
    let key = Secret::random();
    let vault = opaque_id();
    let folder = opaque_id();
    let id = opaque_id();
    let catalog = FolderCatalog {
        version: FOLDER_FORMAT,
        folders: std::collections::BTreeMap::from([(
            folder.clone(),
            FolderData {
                name: "Private projects".into(),
                color: "#aabbcc".into(),
            },
        )]),
        legacy_notes: std::collections::BTreeMap::from([(id.clone(), folder.clone())]),
    };
    let revision = Revision::seal_record(
        &key,
        &a,
        &vault,
        &vault,
        None,
        &Record::FolderCatalog(catalog.clone()),
    )
    .unwrap();
    assert_eq!(revision.version, 2);
    assert_eq!(revision.blob.version, 1);
    assert!(
        matches!(revision.open_record(&key,&a).unwrap(),Record::FolderCatalog(value) if value == catalog)
    );
    assert!(revision.open(&key, &a).is_err());
    let serialized = serde_json::to_string(&revision).unwrap();
    assert!(!serialized.contains("Private projects"));
    assert!(!serialized.contains("legacy_notes"));
    let mut n = note();
    n.version = FOLDER_FORMAT;
    n.folder_id = Some(folder);
    let r = Revision::seal(&key, &a, &vault, &id, None, Some(&n)).unwrap();
    assert!(r.open(&key, &a).unwrap() == Some(n));
    let mut altered = r;
    altered.note_id = opaque_id();
    assert!(altered.open_record(&key, &a).is_err());
    let mut wrong = a;
    wrong.sub = "another-account".into();
    assert!(revision.open_record(&key, &wrong).is_err());
    assert!(Revision::seal_record(
        &key,
        &wrong,
        &vault,
        &opaque_id(),
        None,
        &Record::FolderCatalog(catalog.clone())
    )
    .is_err());
    let mut unsupported = catalog;
    unsupported.version = 99;
    assert!(matches!(
        Revision::seal_record(
            &key,
            &wrong,
            &vault,
            &vault,
            None,
            &Record::FolderCatalog(unsupported)
        ),
        Err(Error::Version)
    ));
}
