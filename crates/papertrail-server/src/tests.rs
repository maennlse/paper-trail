use super::*;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use papertrail_crypto::{opaque_id, NoteData};
use tower::ServiceExt;
fn account(sub: &str) -> Account {
    Account {
        iss: "https://issuer.example".into(),
        sub: sub.into(),
    }
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
