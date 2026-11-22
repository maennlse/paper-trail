use super::*;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use papertrail_crypto::{opaque_id, NoteData};
use tower::ServiceExt;

#[test]
fn sqlite_counters_reject_unsigned_overflow_and_negative_database_values() {
    let db = Connection::open_in_memory().unwrap();
    for value in [0, 1, i64::MAX] {
        let unsigned = u64::try_from(value).unwrap();
        assert_eq!(sql_counter(unsigned).unwrap(), value);
        assert_eq!(
            db.query_row("SELECT ?1", [value], read_counter).unwrap(),
            unsigned
        );
    }
    assert_eq!(sql_counter(u64::MAX).unwrap_err(), StatusCode::BAD_REQUEST);
    assert!(sql_counter(u64::try_from(i64::MAX).unwrap() + 1).is_err());
    assert!(db.query_row("SELECT -1", [], read_counter).is_err());
    assert!(db.query_row("SELECT 1.5", [], read_counter).is_err());
}
#[test]
fn native_settings_are_validated_without_confidential_client_defaults() {
    assert!(
        NativeClientConfig::new("native".into(), "openid offline_access custom:api".into()).is_ok()
    );
    for client in ["", "  ", "native\nsecret"] {
        assert!(NativeClientConfig::new(client.into(), "openid".into()).is_err());
    }
    for scope in [
        "",
        "profile",
        "openid  profile",
        "openid\nprofile",
        "openid bad\\scope",
        "openid bad\"scope",
    ] {
        assert!(NativeClientConfig::new("native".into(), scope.into()).is_err());
    }
}
fn account(sub: &str) -> Account {
    Account {
        iss: "https://issuer.example".into(),
        sub: sub.into(),
    }
}

#[test]
fn erasure_schema_enables_secure_deletion() {
    {
        let server = Server::open(std::path::Path::new(":memory:"), None).unwrap();
        let db = server.db.lock().unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("PRAGMA secure_delete", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM erased_notes", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
}

async fn erase_call(
    s: Arc<Server>,
    a: Account,
    request: &papertrail_crypto::Erasure,
    confirm: bool,
) -> StatusCode {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/erase-folder")
        .header("content-type", "application/json");
    if confirm {
        builder = builder.header("x-papertrail-confirm", "erase-folder");
    }
    let mut request = builder
        .body(Body::from(serde_json::to_vec(request).unwrap()))
        .unwrap();
    request.extensions_mut().insert(a);
    routes(s).oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn folder_erasure_is_atomic_account_scoped_and_blocks_stale_uploads() {
    use papertrail_crypto::{
        ErasedRevision, Erasure, FolderCatalog, FolderData, Record, FOLDER_FORMAT,
    };
    let server = Arc::new(Server::open(std::path::Path::new(":memory:"), None).unwrap());
    let a = account("alice");
    let (vault, key, _) = Vault::create(&a, "a long enough test passphrase").unwrap();
    assert_eq!(
        call(
            server.clone(),
            a.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&vault).unwrap(),
            Some(0)
        )
        .await
        .0,
        204
    );
    let folder = opaque_id();
    let mut catalog = FolderCatalog {
        version: FOLDER_FORMAT,
        folders: std::collections::BTreeMap::from([(
            folder.clone(),
            FolderData {
                name: "Secret".into(),
                color: "preset-0".into(),
            },
        )]),
        ..Default::default()
    };
    let initial = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &vault.id,
        None,
        &Record::FolderCatalog(catalog.clone()),
    )
    .unwrap();
    let data = NoteData {
        version: FOLDER_FORMAT,
        filename: "secret".into(),
        folder: "Secret".into(),
        folder_id: Some(folder.clone()),
        body: "private body".into(),
        modified_ms: 1,
        pinned: false,
        language: None,
    };
    let id = opaque_id();
    let first = Revision::seal(&key, &a, &vault.id, &id, None, Some(&data)).unwrap();
    let second = Revision::seal(
        &key,
        &a,
        &vault.id,
        &id,
        Some(first.id.clone()),
        Some(&data),
    )
    .unwrap();
    for r in [&initial, &first, &second] {
        assert_eq!(
            call(
                server.clone(),
                a.clone(),
                "POST",
                &format!("/v1/notes/{}/revisions", r.note_id),
                serde_json::to_value(r).unwrap(),
                None
            )
            .await
            .0,
            201
        );
    }
    catalog.deleted_folders.insert(folder.clone());
    let marker = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &vault.id,
        Some(initial.id.clone()),
        &Record::FolderCatalog(catalog),
    )
    .unwrap();
    let mut request = Erasure {
        through: 3,
        catalog: marker.clone(),
        replacements: [&first, &second]
            .into_iter()
            .map(|r| ErasedRevision {
                original_digest: digest(&serde_json::to_vec(r).unwrap()),
                revision: r.erase(&key, &a, &folder).unwrap(),
            })
            .collect(),
    };
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, false).await,
        428
    );
    let mut other_issuer = a.clone();
    other_issuer.iss = "https://other-issuer.example".into();
    assert_eq!(
        erase_call(server.clone(), other_issuer, &request, true).await,
        404
    );
    request.through = 2;
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, true).await,
        409
    );
    request.through = 3;
    let last = request.replacements.pop().unwrap();
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, true).await,
        400
    );
    request.replacements.push(last);
    request.replacements[0].original_digest = "altered".into();
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, true).await,
        409
    );
    request.replacements[0].original_digest = digest(&serde_json::to_vec(&first).unwrap());
    // Failed requests did not replace any ciphertext.
    {
        let db = server.db.lock().unwrap();
        let stored: String = db
            .query_row(
                "SELECT data FROM revisions WHERE revision=?",
                [&first.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, serde_json::to_string(&first).unwrap());
    }
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, true).await,
        204
    );
    assert_eq!(
        erase_call(server.clone(), a.clone(), &request, true).await,
        409
    );
    let (status, bytes) = call(
        server.clone(),
        a.clone(),
        "GET",
        "/v1/changes",
        serde_json::Value::Null,
        None,
    )
    .await;
    assert_eq!(status, 200);
    let changes: Changes = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(changes.through, 4);
    assert_eq!(
        changes
            .changes
            .iter()
            .map(|c| c.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    for change in &changes.changes[1..3] {
        assert!(matches!(
            change.revision.open_record(&key, &a).unwrap(),
            Record::Erased { .. }
        ));
    }
    let (_, export) = call(
        server.clone(),
        a.clone(),
        "GET",
        "/v1/export",
        serde_json::Value::Null,
        None,
    )
    .await;
    let export = String::from_utf8(export).unwrap();
    assert!(!export.contains(&first.blob.ciphertext));
    assert!(!export.contains(&second.blob.ciphertext));
    assert_eq!(
        call(
            server.clone(),
            a.clone(),
            "POST",
            &format!("/v1/notes/{id}/revisions"),
            serde_json::to_value(&second).unwrap(),
            None
        )
        .await
        .0,
        410
    );
    let new = Revision::seal(&key, &a, &vault.id, &opaque_id(), None, Some(&data)).unwrap();
    assert_eq!(
        call(
            server.clone(),
            a.clone(),
            "POST",
            &format!("/v1/notes/{}/revisions", new.note_id),
            serde_json::to_value(&new).unwrap(),
            None
        )
        .await
        .0,
        412
    );
    for (head, status) in [(&initial.id, 412), (&marker.id, 201)] {
        let mut upload = Request::builder()
            .method("POST")
            .uri(format!("/v1/notes/{}/revisions", new.note_id))
            .header("content-type", "application/json")
            .header("x-papertrail-catalog-head", head)
            .body(Body::from(serde_json::to_vec(&new).unwrap()))
            .unwrap();
        upload.extensions_mut().insert(a.clone());
        assert_eq!(
            routes(server.clone())
                .oneshot(upload)
                .await
                .unwrap()
                .status(),
            status
        );
    }
}
#[tokio::test]
async fn same_subject_from_a_different_issuer_cannot_access_or_delete_a_vault() {
    let server = Arc::new(Server::open(std::path::Path::new(":memory:"), None).unwrap());
    let owner = account("same-subject");
    let (vault, _, _) = Vault::create(&owner, "a long enough passphrase").unwrap();
    assert_eq!(
        call(
            server.clone(),
            owner.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(vault).unwrap(),
            Some(0)
        )
        .await
        .0,
        204
    );
    let mut other = owner.clone();
    other.iss = "https://different-issuer.example".into();
    assert_eq!(
        call(
            server.clone(),
            other.clone(),
            "GET",
            "/v1/vault",
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        404
    );
    let mut request = Request::builder()
        .method("DELETE")
        .uri("/v1/account")
        .header("x-papertrail-confirm", "delete-account")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(other);
    assert_eq!(
        routes(server.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        call(
            server,
            owner,
            "GET",
            "/v1/vault",
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        200
    );
}
async fn call(
    s: Arc<Server>,
    a: Account,
    method: &str,
    path: &str,
    body: serde_json::Value,
    expected: Option<u64>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(n) = expected {
        request = request.header("if-match", n);
    }
    let mut request = request.body(Body::from(body.to_string())).unwrap();
    request.extensions_mut().insert(a);
    let response = routes(s).oneshot(request).await.unwrap();
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
#[tokio::test]
async fn authorization_conflicts_history_tombstones_and_cursor() {
    let s = Arc::new(Server::open(std::path::Path::new(":memory:"), None).unwrap());
    let a = account("alice");
    let b = account("bob");
    let (vault, key, _) = Vault::create(&a, "a long enough passphrase").unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&vault).unwrap(),
            Some(0)
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(
            s.clone(),
            b.clone(),
            "GET",
            "/v1/vault",
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        404
    );
    let data = NoteData {
        version: 1,
        filename: "private.txt".into(),
        folder: "Notes".into(),
        folder_id: None,
        body: "private".into(),
        modified_ms: 1,
        pinned: false,
        language: None,
    };
    let id = opaque_id();
    let r = Revision::seal(&key, &a, &vault.id, &id, None, Some(&data)).unwrap();
    let path = format!("/v1/notes/{id}/revisions");
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &path,
            serde_json::to_value(&r).unwrap(),
            None
        )
        .await
        .0,
        201
    );
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &path,
            serde_json::to_value(&r).unwrap(),
            None
        )
        .await
        .0,
        204
    );
    let conflicting = Revision::seal(&key, &a, &vault.id, &id, None, Some(&data)).unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &path,
            serde_json::to_value(conflicting).unwrap(),
            None
        )
        .await
        .0,
        409
    );
    let read = format!("{path}/{}", r.id);
    assert_eq!(
        call(
            s.clone(),
            b.clone(),
            "GET",
            &read,
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            s.clone(),
            b.clone(),
            "POST",
            &path,
            serde_json::to_value(&r).unwrap(),
            None
        )
        .await
        .0,
        404
    );
    let mut other_issuer = a.clone();
    other_issuer.iss = "https://other.example".into();
    assert_eq!(
        call(
            s.clone(),
            other_issuer,
            "GET",
            &read,
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        404
    );
    let tombstone = Revision::seal(&key, &a, &vault.id, &id, Some(r.id), None).unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &path,
            serde_json::to_value(tombstone).unwrap(),
            None
        )
        .await
        .0,
        201
    );
    let (_, body) = call(
        s.clone(),
        a.clone(),
        "GET",
        "/v1/changes?cursor=0",
        serde_json::Value::Null,
        None,
    )
    .await;
    let page: Changes = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.changes.len(), 2);
    assert!(page.changes[1].revision.deleted);
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "GET",
            "/v1/changes?cursor=3",
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        409
    );
    let (_, body) = call(
        s.clone(),
        b.clone(),
        "GET",
        "/v1/export",
        serde_json::Value::Null,
        None,
    )
    .await;
    let export: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(export["revisions"].as_array().unwrap().len(), 0);
    assert!(export["vault"].is_null());
    let request = Request::builder()
        .uri("/v1/vault")
        .body(Body::empty())
        .unwrap();
    assert_eq!(router(s).oneshot(request).await.unwrap().status(), 401);
}

#[tokio::test]
async fn deletion_and_envelope_updates_never_cross_accounts() {
    let s = Arc::new(Server::open(std::path::Path::new(":memory:"), None).unwrap());
    let a = account("alice");
    let b = account("bob");
    let (v, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&v).unwrap(),
            Some(0)
        )
        .await
        .0,
        204
    );
    let (v2, _) = v.rewrap(&a, "another long passphrase", &key).unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&v2).unwrap(),
            Some(0)
        )
        .await
        .0,
        409
    );
    assert_eq!(
        call(
            s.clone(),
            b.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&v2).unwrap(),
            Some(1)
        )
        .await
        .0,
        409
    );
    let mut request = Request::builder()
        .method("DELETE")
        .uri("/v1/account")
        .header("x-papertrail-confirm", "delete-account")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(b);
    assert_eq!(
        routes(s.clone()).oneshot(request).await.unwrap().status(),
        204
    );
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "GET",
            "/v1/vault",
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        200
    );
    let id = opaque_id();
    let mut r = Revision::seal(&key, &a, &v.id, &id, None, None).unwrap();
    r.version = 99;
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &format!("/v1/notes/{id}/revisions"),
            serde_json::to_value(r).unwrap(),
            None
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(s, a, "DELETE", "/v1/account", serde_json::Value::Null, None)
            .await
            .0,
        428
    );
}

#[tokio::test]
async fn encrypted_folder_catalogs_use_the_same_account_scoped_immutable_api() {
    use papertrail_crypto::{FolderCatalog, FolderData, Record, FOLDER_FORMAT};
    let s = Arc::new(Server::open(std::path::Path::new(":memory:"), None).unwrap());
    let a = account("alice");
    let b = account("bob");
    let (vault, key, _) = Vault::create(&a, "long enough passphrase").unwrap();
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "PUT",
            "/v1/vault",
            serde_json::to_value(&vault).unwrap(),
            Some(0)
        )
        .await
        .0,
        204
    );
    let folder = opaque_id();
    let catalog = FolderCatalog {
        version: FOLDER_FORMAT,
        folders: std::collections::BTreeMap::from([(
            folder.clone(),
            FolderData {
                name: "Secret folder".into(),
                color: "preset-1".into(),
            },
        )]),
        legacy_notes: Default::default(),
        deleted_folders: Default::default(),
    };
    let r = Revision::seal_record(
        &key,
        &a,
        &vault.id,
        &vault.id,
        None,
        &Record::FolderCatalog(catalog),
    )
    .unwrap();
    let path = format!("/v1/notes/{}/revisions", vault.id);
    assert_eq!(
        call(
            s.clone(),
            a.clone(),
            "POST",
            &path,
            serde_json::to_value(&r).unwrap(),
            None
        )
        .await
        .0,
        201
    );
    let (_, bytes) = call(
        s.clone(),
        a.clone(),
        "GET",
        &format!("{path}/{}", r.id),
        serde_json::Value::Null,
        None,
    )
    .await;
    assert!(!String::from_utf8_lossy(&bytes).contains("Secret folder"));
    assert_eq!(
        call(
            s,
            b,
            "GET",
            &format!("{path}/{}", r.id),
            serde_json::Value::Null,
            None
        )
        .await
        .0,
        404
    );
}
