// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in encrypted synchronization; local note files intentionally remain plaintext.
pub mod auth;
pub mod control;
pub mod engine;
pub mod folders;
pub mod local;
use papertrail_crypto::{Account, Changes, Revision, Vault};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, time::Duration};
use zeroize::Zeroizing;
fn json<T: serde::de::DeserializeOwned>(response: Response) -> io::Result<T> {
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take((papertrail_crypto::MAX_BLOB * 2 + 65536) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Could not read API response"))?;
    serde_json::from_slice(&bytes).map_err(|_| error("Invalid or oversized API response"))
}

pub fn error(message: impl std::fmt::Display) -> io::Error {
    io::Error::other(message.to_string())
}
pub fn read_config(bytes: &[u8]) -> io::Result<Config> {
    let config: Config =
        serde_json::from_slice(bytes).map_err(|_| error("Invalid sync configuration"))?;
    config.validate()?;
    Ok(config)
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub server: String,
    pub issuer: String,
    pub client_id: String,
    pub scopes: String,
    pub folders: Vec<PathBuf>,
    #[serde(default)]
    pub download_dir: Option<PathBuf>,
    #[serde(default)]
    pub folder_mappings: std::collections::BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub excluded_folders: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub legacy_note_mappings: std::collections::BTreeMap<String, String>,
    #[serde(default = "enabled_by_default")]
    pub auto_download: bool,
    #[serde(default)]
    pub selected_folders: std::collections::BTreeSet<String>,
}
fn enabled_by_default() -> bool {
    true
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 2,
            server: String::new(),
            issuer: String::new(),
            client_id: String::new(),
            scopes: String::new(),
            folders: Vec::new(),
            download_dir: None,
            folder_mappings: Default::default(),
            excluded_folders: Default::default(),
            legacy_note_mappings: Default::default(),
            auto_download: true,
            selected_folders: Default::default(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> io::Result<()> {
        if !matches!(self.version, 1 | 2)
            || !https(&self.server)
            || !https(&self.issuer)
            || self.client_id.is_empty()
            || (self.folders.is_empty() && self.download_dir.is_none())
        {
            return Err(error("Sync needs format 1, HTTPS server/issuer, public client ID, and at least one selected folder"));
        }
        for folder in &self.folders {
            if !folder.is_absolute() {
                return Err(error("Sync folders must be absolute paths"));
            }
        }
        if self.download_dir.as_ref().is_some_and(|p| !p.is_absolute())
            || self
                .folder_mappings
                .iter()
                .any(|(id, p)| !papertrail_crypto::valid_id(id) || !p.is_absolute())
            || self
                .excluded_folders
                .iter()
                .any(|id| !papertrail_crypto::valid_id(id))
            || self
                .selected_folders
                .iter()
                .any(|id| !papertrail_crypto::valid_id(id))
            || self.legacy_note_mappings.iter().any(|(note, folder)| {
                !papertrail_crypto::valid_id(note) || !papertrail_crypto::valid_id(folder)
            })
        {
            return Err(error("Invalid folder ID or non-absolute folder mapping"));
        }
        let paths: std::collections::BTreeSet<_> = self
            .folder_mappings
            .values()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .collect();
        if paths.len() != self.folder_mappings.len() {
            return Err(error("Each remote folder needs a distinct local directory"));
        }
        let selected: std::collections::BTreeSet<_> = self
            .folders
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .collect();
        if selected.len() != self.folders.len() {
            return Err(error(
                "The same local directory cannot be selected through several aliases",
            ));
        }
        Ok(())
    }
    pub fn download_root(&self) -> io::Result<PathBuf> {
        self.download_dir
            .clone()
            .or_else(|| {
                self.folders
                    .first()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("Paper Trail Synced"))
            })
            .ok_or_else(|| error("Choose a directory for downloaded folders"))
    }
}
pub fn https(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && u.query().is_none()
    })
}
pub fn http() -> io::Result<Client> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| error("Could not initialize HTTPS client"))
}
pub struct Api {
    client: Client,
    base: String,
    token: Zeroizing<String>,
}
impl Api {
    pub fn new(config: &Config, access_token: String) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            client: http()?,
            base: config.server.trim_end_matches('/').into(),
            token: Zeroizing::new(access_token),
        })
    }
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.token.as_str())
    }
    fn send(request: reqwest::blocking::RequestBuilder) -> io::Result<Response> {
        let response = request
            .send()
            .map_err(|_| error("Sync HTTPS request failed"))?;
        if response.status().is_success()
            || response.status() == reqwest::StatusCode::NOT_FOUND
            || response.status() == reqwest::StatusCode::CONFLICT
        {
            Ok(response)
        } else {
            Err(error(format!("Sync API returned {}", response.status())))
        }
    }
    pub fn identity(&self) -> io::Result<Account> {
        json(Self::send(
            self.request(reqwest::Method::GET, "/v1/identity"),
        )?)
    }
    pub fn vault(&self) -> io::Result<Option<Vault>> {
        let r = Self::send(self.request(reqwest::Method::GET, "/v1/vault"))?;
        if r.status() == 404 {
            return Ok(None);
        }
        let v: Vault = json(r)?;
        v.validate().map_err(error)?;
        Ok(Some(v))
    }
    pub fn put_vault(&self, v: &Vault, expected: u64) -> io::Result<()> {
        let r = Self::send(
            self.request(reqwest::Method::PUT, "/v1/vault")
                .header("if-match", expected)
                .json(v),
        )?;
        if !r.status().is_success() {
            return Err(error(
                "Vault changed on another device; unlock again before retrying",
            ));
        }
        Ok(())
    }
    pub fn changes(&self, cursor: u64, through: Option<u64>) -> io::Result<Changes> {
        let path = format!(
            "/v1/changes?cursor={cursor}{}",
            through.map(|t| format!("&through={t}")).unwrap_or_default()
        );
        let r = Self::send(self.request(reqwest::Method::GET, &path))?;
        if r.status() == 409 {
            return Err(error("Server history moved backward; sync stopped"));
        }
        json(r)
    }
    pub fn upload(&self, r: &Revision) -> io::Result<Option<u64>> {
        let response = Self::send(
            self.request(
                reqwest::Method::POST,
                &format!("/v1/notes/{}/revisions", r.note_id),
            )
            .json(r),
        )?;
        if response.status() == 409 {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(error("Revision upload failed"));
        }
        let sequence = response
            .headers()
            .get("x-papertrail-sequence")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| error("Missing upload checkpoint"))?;
        Ok(Some(sequence))
    }
}
