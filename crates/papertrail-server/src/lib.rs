// SPDX-License-Identifier: GPL-3.0-or-later
pub mod auth;
pub mod logging;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use papertrail_crypto::{Account, Change, Changes, Revision, Vault, MAX_BLOB};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub struct Server {
    db: Mutex<Connection>,
    pub auth: Option<auth::Authenticator>,
    native_client: Option<NativeClientConfig>,
}
/// Public native-client settings. Never populate this from introspection credentials.
#[derive(Serialize)]
pub struct NativeClientConfig {
    client_id: String,
    scope: String,
}
impl NativeClientConfig {
    pub fn new(client_id: String, scope: String) -> Result<Self, &'static str> {
        if client_id.trim().is_empty()
            || client_id.len() > 1024
            || client_id.chars().any(char::is_control)
            || scope.len() > 8192
            || !scope.split(' ').any(|s| s == "openid")
            || scope.split(' ').any(|s| {
                s.is_empty()
                    || !s.bytes().all(|b| {
                        b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b)
                    })
            })
        {
            return Err("Invalid public native client ID or scopes (openid is required)");
        }
        Ok(Self { client_id, scope })
    }
}
type ApiResult<T> = Result<T, StatusCode>;
impl Server {
    pub fn open(
        path: &std::path::Path,
        auth: Option<auth::Authenticator>,
    ) -> rusqlite::Result<Self> {
        let db = Connection::open(path)?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS vaults (iss TEXT NOT NULL, sub TEXT NOT NULL, generation INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(iss,sub));
            CREATE TABLE IF NOT EXISTS revisions (iss TEXT NOT NULL, sub TEXT NOT NULL, sequence INTEGER NOT NULL, note TEXT NOT NULL, revision TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(iss,sub,revision), UNIQUE(iss,sub,sequence));
            CREATE INDEX IF NOT EXISTS note_history ON revisions(iss,sub,note,sequence);
            PRAGMA user_version=1;")?;
        Ok(Self {
            db: Mutex::new(db),
            auth,
            native_client: None,
        })
    }
    pub fn with_native_client(mut self, native_client: Option<NativeClientConfig>) -> Self {
        self.native_client = native_client;
        self
    }
}
pub fn router(server: Arc<Server>) -> Router {
    routes(server.clone())
        // Preserve the protected router's fallback when merging a public route.
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::from_fn_with_state(server.clone(), authenticate))
        // Merge after applying authentication: only this read-only route is public.
        .merge(
            Router::new()
                .route("/v1/auth/config", get(auth_config))
                .with_state(server),
        )
        .layer(middleware::from_fn(logging::request))
}
async fn auth_config(State(server): State<Arc<Server>>) -> ApiResult<Json<serde_json::Value>> {
    let auth = server
        .auth
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let native = server
        .native_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !auth::https(auth.issuer())
        || auth.issuer().len() > 4096
        || auth
            .issuer()
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || reqwest::Url::parse(auth.issuer()).is_ok_and(|url| url.query().is_some())
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    // Explicit allowlist: never serialize Authenticator, its audience, or credentials.
    Ok(Json(serde_json::json!({
        "issuer": auth.issuer(), "client_id": native.client_id, "scope": native.scope,
    })))
}
// Separate from authentication only to allow deterministic authorization tests.
fn routes(server: Arc<Server>) -> Router {
    Router::new()
        .route("/v1/identity", get(identity))
        .route("/v1/vault", get(vault).put(put_vault))
        .route("/v1/changes", get(changes))
        .route("/v1/notes/{note}/revisions", post(append))
        .route("/v1/notes/{note}/revisions/{revision}", get(revision))
        .route("/v1/export", get(export))
        .route("/v1/account", axum::routing::delete(delete_account))
        .layer(DefaultBodyLimit::max(MAX_BLOB * 2))
        .with_state(server)
}
async fn authenticate(
    State(server): State<Arc<Server>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        tracing::warn!(
            reason = "missing_or_malformed_bearer",
            "Authentication rejected"
        );
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(auth) = &server.auth else {
        tracing::error!("Authentication is not configured");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match auth.authenticate(token).await {
        Ok(account) => {
            request.extensions_mut().insert(account);
            next.run(request).await
        }
        Err(auth::AuthError::InvalidToken) => StatusCode::UNAUTHORIZED.into_response(),
        Err(auth::AuthError::Unavailable) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
fn database_error(error: rusqlite::Error) -> StatusCode {
    // SQLite messages can contain application data; log only the error code.
    tracing::error!(code = ?error.sqlite_error_code(), "Database operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
fn parse<T: serde::de::DeserializeOwned>(s: &str) -> ApiResult<T> {
    serde_json::from_str(s).map_err(|_| {
        tracing::error!("Stored record could not be decoded");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
async fn identity(Extension(a): Extension<Account>) -> Json<Account> {
    Json(a)
}
async fn vault(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
) -> ApiResult<Json<Vault>> {
    let db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM vaults WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    Ok(Json(parse(&data.ok_or(StatusCode::NOT_FOUND)?)?))
}
async fn put_vault(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
    headers: HeaderMap,
    Json(v): Json<Vault>,
) -> ApiResult<StatusCode> {
    v.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let expected = headers
        .get("if-match")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or(StatusCode::PRECONDITION_REQUIRED)?;
    let mut db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tx = db.transaction().map_err(database_error)?;
    let old: Option<String> = tx
        .query_row(
            "SELECT data FROM vaults WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    match old {
        Some(data) => {
            let old: Vault = parse(&data)?;
            if expected != old.generation
                || v.id != old.id
                || v.generation != expected.checked_add(1).ok_or(StatusCode::BAD_REQUEST)?
            {
                return Err(StatusCode::CONFLICT);
            }
        }
        None => {
            if expected != 0 || v.generation != 1 {
                return Err(StatusCode::CONFLICT);
            }
        }
    }
    tx.execute("INSERT INTO vaults VALUES (?,?,?,?) ON CONFLICT(iss,sub) DO UPDATE SET generation=excluded.generation,data=excluded.data",params![a.iss,a.sub,v.generation,serde_json::to_string(&v).unwrap()]).map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn append(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
    Path(note): Path<String>,
    Json(r): Json<Revision>,
) -> ApiResult<(StatusCode, [(&'static str, String); 1])> {
    r.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    if r.note_id != note {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tx = db.transaction().map_err(database_error)?;
    let vault: Option<String> = tx
        .query_row(
            "SELECT data FROM vaults WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if parse::<Vault>(&vault.ok_or(StatusCode::NOT_FOUND)?)?.id != r.vault_id {
        return Err(StatusCode::BAD_REQUEST);
    }
    let data = serde_json::to_string(&r).unwrap();
    let existing: Option<(u64, String)> = tx
        .query_row(
            "SELECT sequence,data FROM revisions WHERE iss=? AND sub=? AND revision=?",
            params![a.iss, a.sub, r.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((sequence, existing)) = existing {
        return if existing == data {
            Ok((
                StatusCode::NO_CONTENT,
                [("x-papertrail-sequence", sequence.to_string())],
            ))
        } else {
            Err(StatusCode::CONFLICT)
        };
    }
    let head: Option<String> = tx.query_row("SELECT revision FROM revisions WHERE iss=? AND sub=? AND note=? ORDER BY sequence DESC LIMIT 1",params![a.iss,a.sub,note],|r| r.get(0)).optional().map_err(database_error)?;
    if head != r.parent {
        return Err(StatusCode::CONFLICT);
    }
    let seq: u64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM revisions WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    tx.execute(
        "INSERT INTO revisions VALUES (?,?,?,?,?,?)",
        params![a.iss, a.sub, seq, note, r.id, data],
    )
    .map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        [("x-papertrail-sequence", seq.to_string())],
    ))
}
#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    cursor: u64,
    through: Option<u64>,
}
async fn changes(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
    Query(q): Query<Cursor>,
) -> ApiResult<Json<Changes>> {
    let db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let latest: u64 = db
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM revisions WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let through = q.through.unwrap_or(latest);
    if through > latest || q.cursor > through {
        return Err(StatusCode::CONFLICT);
    }
    let mut stmt = db.prepare("SELECT sequence,data FROM revisions WHERE iss=? AND sub=? AND sequence>? AND sequence<=? ORDER BY sequence LIMIT 100").map_err(database_error)?;
    let rows = stmt
        .query_map(params![a.iss, a.sub, q.cursor, through], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut changes = vec![];
    let mut bytes = 0;
    for row in rows {
        let (sequence, data) = row.map_err(database_error)?;
        if !changes.is_empty() && bytes + data.len() > MAX_BLOB * 2 {
            break;
        }
        bytes += data.len();
        changes.push(Change {
            sequence,
            revision: parse(&data)?,
        });
    }
    let next = changes.last().map(|c| c.sequence).unwrap_or(q.cursor);
    Ok(Json(Changes {
        through,
        next,
        changes,
    }))
}
async fn revision(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
    Path((note, id)): Path<(String, String)>,
) -> ApiResult<Json<Revision>> {
    let db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM revisions WHERE iss=? AND sub=? AND note=? AND revision=?",
            params![a.iss, a.sub, note, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    Ok(Json(parse(&data.ok_or(StatusCode::NOT_FOUND)?)?))
}
#[derive(Serialize)]
struct Export {
    version: u32,
    account: Account,
    vault: Option<Vault>,
    revisions: Vec<Change>,
}
async fn export(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
) -> ApiResult<Json<Export>> {
    let db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let v: Option<String> = db
        .query_row(
            "SELECT data FROM vaults WHERE iss=? AND sub=?",
            params![a.iss, a.sub],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    let mut stmt = db
        .prepare("SELECT sequence,data FROM revisions WHERE iss=? AND sub=? ORDER BY sequence")
        .map_err(database_error)?;
    let rows = stmt
        .query_map(params![a.iss, a.sub], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut revisions = vec![];
    for row in rows {
        let (sequence, data) = row.map_err(database_error)?;
        revisions.push(Change {
            sequence,
            revision: parse(&data)?,
        });
    }
    Ok(Json(Export {
        version: 1,
        account: a,
        vault: v.map(|v| parse(&v)).transpose()?,
        revisions,
    }))
}
async fn delete_account(
    State(s): State<Arc<Server>>,
    Extension(a): Extension<Account>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    if headers
        .get("x-papertrail-confirm")
        .and_then(|v| v.to_str().ok())
        != Some("delete-account")
    {
        return Err(StatusCode::PRECONDITION_REQUIRED);
    }
    let mut db = s.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tx = db.transaction().map_err(database_error)?;
    tx.execute(
        "DELETE FROM revisions WHERE iss=? AND sub=?",
        params![a.iss, a.sub],
    )
    .map_err(database_error)?;
    tx.execute(
        "DELETE FROM vaults WHERE iss=? AND sub=?",
        params![a.iss, a.sub],
    )
    .map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
