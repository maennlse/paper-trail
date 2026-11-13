// SPDX-License-Identifier: GPL-3.0-or-later
//! Public server configuration is untrusted until the frontend confirms its issuer.
use crate::{error, http, https, Config};
use serde::Deserialize;
use std::io::{self, Read};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PublicConfig {
    pub issuer: String,
    pub client_id: String,
    pub scope: String,
}
impl PublicConfig {
    pub fn validate(&self) -> io::Result<()> {
        if !https(&self.issuer)
            || self.issuer.len() > 4096
            || self.client_id.trim().is_empty()
            || self.client_id.len() > 1024
            || self.client_id.chars().any(char::is_control)
            || self.scope.len() > 8192
            || !self.scope.split(' ').any(|s| s == "openid")
            || self.scope.split(' ').any(|s| {
                s.is_empty()
                    || !s.bytes().all(|b| {
                        b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b)
                    })
            })
        {
            return Err(error("Invalid public authentication configuration: HTTPS issuer, public client ID and OIDC scopes are required"));
        }
        Ok(())
    }
}
pub(super) fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
    limit: u64,
) -> io::Result<T> {
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Could not read authentication discovery response"))?;
    if bytes.len() as u64 > limit {
        return Err(error(
            "Authentication discovery response exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| error("Invalid authentication discovery response"))
}

pub fn fetch(server: &str) -> io::Result<PublicConfig> {
    if !https(server) {
        return Err(error(
            "Discovery requires an HTTPS sync server without credentials, query or fragment",
        ));
    }
    fetch_with(&http()?, server)
}
fn fetch_with(client: &reqwest::blocking::Client, server: &str) -> io::Result<PublicConfig> {
    // No Bearer header: the sync server has not established an identity yet.
    let response = client
        .get(format!("{}/v1/auth/config", server.trim_end_matches('/')))
        .send()
        .map_err(|_| {
            error("Authentication configuration request failed or timed out; check the sync server")
        })?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(error("Server does not offer authentication discovery; use manual issuer/client/scopes configuration"));
    }
    if !response.status().is_success() {
        return Err(error("Server authentication configuration is unavailable; check deployment settings or explicitly configure authentication manually"));
    }
    let config: PublicConfig = decode(response, 16384)?;
    config.validate()?;
    Ok(config)
}

/// Prepare, but do not save or authenticate. The UI must confirm a new issuer.
pub fn prepare(mut config: Config) -> io::Result<Config> {
    prepare_with(&mut config, fetch, |issuer| {
        super::discover_issuer(issuer).map(|_| ())
    })?;
    Ok(config)
}
fn prepare_with(
    config: &mut Config,
    fetch: impl FnOnce(&str) -> io::Result<PublicConfig>,
    metadata: impl FnOnce(&str) -> io::Result<()>,
) -> io::Result<()> {
    let public = fetch(&config.server)?;
    public.validate()?;
    metadata(&public.issuer)?;
    config.issuer = public.issuer;
    config.client_id = public.client_id;
    config.scopes = public.scope;
    config.discover_auth = true;
    config.validate()?;
    Ok(())
}

/// An existing explicit manual issuer is also an explicit trust decision.
pub fn needs_confirmation(previous: Option<&Config>, candidate: &Config) -> bool {
    !previous.is_some_and(|old| old.server == candidate.server && old.issuer == candidate.issuer)
}

pub fn trust_description(previous: Option<&Config>, candidate: &Config) -> String {
    let old = previous
        .filter(|old| old.server == candidate.server && old.issuer != candidate.issuer)
        .map(|old| {
            format!(
                "Previously accepted identity provider: {}\n\n",
                old.issuer.escape_default()
            )
        })
        .unwrap_or_default();
    format!("Sync server: {}\n\n{old}Advertised identity provider: {}\n\nOnly continue if you trust this identity provider. Accepting does not migrate accounts, vaults or encryption keys. Existing bound profiles cannot switch issuers.", candidate.server.escape_default(), candidate.issuer.escape_default())
}

pub fn check(config: &Config) -> io::Result<()> {
    if config.discover_auth {
        let advertised = fetch(&config.server)?;
        verify(config, &advertised)?;
    }
    Ok(())
}
fn verify(config: &Config, advertised: &PublicConfig) -> io::Result<()> {
    advertised.validate()?;
    if advertised.issuer != config.issuer {
        return Err(error("The sync server advertised a different identity provider. Authentication and sync stopped; review Configure Sync (TUI: :sync-configure SERVER). Confirm the new issuer explicitly; an existing account/vault requires a separate profile"));
    }
    if advertised.client_id != config.client_id || advertised.scope != config.scopes {
        return Err(error("Server native client settings changed; review Configure Sync before signing in again (TUI: :sync-configure SERVER)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    fn public() -> PublicConfig {
        PublicConfig {
            issuer: "https://issuer.example/tenant".into(),
            client_id: "native".into(),
            scope: "openid offline_access api:notes".into(),
        }
    }
    fn accepted() -> Config {
        Config {
            server: "https://sync.example/base".into(),
            issuer: public().issuer,
            client_id: public().client_id,
            scopes: public().scope,
            discover_auth: true,
            folders: vec![std::env::temp_dir().join("papertrail-test-notes")],
            ..Config::default()
        }
    }
    fn response(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }
    // Only this test transport allows HTTP. Production fetch is HTTPS-only.
    fn mock(reply: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = format!("http://{}/base", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            let request = std::str::from_utf8(&bytes[..n]).unwrap();
            assert!(request.starts_with("GET /base/v1/auth/config HTTP/1.1"));
            assert!(!request.to_lowercase().contains("authorization:"));
            let _ = stream.write_all(reply.as_bytes());
        });
        server
    }
    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap()
    }
    #[test]
    fn successful_public_discovery_is_unauthenticated_and_preserves_base_path() {
        let body = r#"{"issuer":"https://issuer.example/tenant","client_id":"native","scope":"openid offline_access api:notes"}"#;
        assert_eq!(
            fetch_with(&client(), &mock(response("200 OK", body))).unwrap(),
            public()
        );
        let mut candidate = Config {
            server: "https://sync.example/base".into(),
            folders: vec![std::env::temp_dir().join("papertrail-test-notes")],
            ..Config::default()
        };
        prepare_with(&mut candidate, |_| fetch_with(&client(), &mock(response("200 OK", body))), |issuer| {
            let metadata = serde_json::from_value(serde_json::json!({"issuer":"https://issuer.example/tenant", "authorization_endpoint":"https://issuer.example/authorize", "token_endpoint":"https://issuer.example/token", "device_authorization_endpoint":"https://issuer.example/device", "code_challenge_methods_supported":["S256"]})).unwrap();
            super::super::validate_discovery(metadata, issuer).map(|_| ())
        }).unwrap();
        assert!(candidate.discover_auth);
        assert_eq!(candidate.issuer, public().issuer);
        assert_eq!(candidate.client_id, public().client_id);
        assert_eq!(candidate.scopes, public().scope);
        assert!(needs_confirmation(None, &candidate));
    }
    #[test]
    fn incomplete_malformed_oversized_missing_and_redirect_responses_fail_closed() {
        for body in [
            "not json",
            "{}",
            r#"{"issuer":"https://issuer.example","client_id":"native"}"#,
            r#"{"issuer":"http://issuer.example","client_id":"native","scope":"openid"}"#,
        ] {
            assert!(fetch_with(&client(), &mock(response("200 OK", body))).is_err());
        }
        assert!(fetch_with(&client(), &mock(response("200 OK", &"x".repeat(16385)))).is_err());
        let err = fetch_with(
            &client(),
            &mock(response("404 Not Found", "PRIVATE_PROVIDER_ERROR")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("manual"));
        assert!(!err.to_string().contains("PRIVATE_PROVIDER_ERROR"));
        for status in ["503 Service Unavailable", "401 Unauthorized"] {
            assert!(fetch_with(&client(), &mock(response(status, "PRIVATE_ERROR"))).is_err());
        }
        let redirect = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never-follow\r\nContent-Length: 0\r\n\r\n";
        assert!(fetch_with(&client(), &mock(redirect.into())).is_err());
        assert!(fetch_with(&client(), "http://127.0.0.1:0").is_err());
        assert!(fetch_with(
            &client(),
            &mock("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{".into())
        )
        .is_err());
    }
    #[test]
    fn stalled_server_is_bounded_by_the_http_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            // Do not send response headers: wait until the timed-out client closes.
            let _ = stream.read_to_end(&mut Vec::new());
        });
        assert!(fetch_with(&client(), &format!("http://{address}"))
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        worker.join().unwrap();
    }
    #[test]
    fn untrusted_urls_and_invalid_public_settings_are_rejected() {
        for url in [
            "http://localhost:8787",
            "file:///etc/passwd",
            "https://user:password@sync.example",
            "https://sync.example/#x",
            "https://sync.example/?x",
            "https://sync.example/\nfoo",
        ] {
            assert!(fetch(url).is_err());
            let mut p = public();
            p.issuer = url.into();
            assert!(p.validate().is_err());
        }
        for scope in [
            "",
            "profile",
            "openid  profile",
            "openid\tprofile",
            "openid bad\\scope",
            "openid bad\"scope",
        ] {
            let mut p = public();
            p.scope = scope.into();
            assert!(p.validate().is_err());
        }
        for id in ["", "  ", "native\n"] {
            let mut p = public();
            p.client_id = id.into();
            assert!(p.validate().is_err());
        }
    }
    #[test]
    fn mismatched_oidc_metadata_cannot_replace_a_saved_configuration() {
        let mut config = accepted();
        let original = config.clone();
        let result = prepare_with(
            &mut config,
            |_| Ok(public()),
            |issuer| {
                let metadata = serde_json::from_value(serde_json::json!({"issuer":"https://wrong-issuer.example", "token_endpoint":"https://issuer.example/token"})).unwrap();
                super::super::validate_discovery(metadata, issuer).map(|_| ())
            },
        );
        assert!(result.is_err());
        assert!(config == original);
    }
    #[test]
    fn first_trust_reuse_and_changed_issuer_are_explicit_and_do_not_mutate_identity() {
        let config = accepted();
        assert!(needs_confirmation(None, &config));
        assert!(!needs_confirmation(Some(&config), &config));
        let mut changed = config.clone();
        changed.issuer = "https://other.example".into();
        assert!(needs_confirmation(Some(&config), &changed));
        let mut advertised = public();
        advertised.issuer = changed.issuer.clone();
        assert!(verify(&config, &advertised)
            .unwrap_err()
            .to_string()
            .contains("different identity provider"));
        assert_eq!(config.issuer, "https://issuer.example/tenant");
        assert!(verify(&config, &public()).is_ok());
        advertised = public();
        advertised.client_id = "another-native".into();
        assert!(verify(&config, &advertised).is_err());
        advertised = public();
        advertised.scope = "openid extra".into();
        assert!(verify(&config, &advertised).is_err());
        let saved = serde_json::to_vec(&config).unwrap();
        assert_eq!(crate::read_config(&saved).unwrap().issuer, config.issuer);
    }
    #[test]
    fn legacy_manual_configuration_does_not_contact_discovery_or_overwrite_issuer() {
        let mut config = accepted();
        config.discover_auth = false;
        let saved = serde_json::to_vec(&config).unwrap();
        assert!(!std::str::from_utf8(&saved)
            .unwrap()
            .contains("discover_auth"));
        let loaded = crate::read_config(&saved).unwrap();
        assert!(!loaded.discover_auth);
        check(&loaded).unwrap(); // sync.example is deliberately not contacted.
        assert_eq!(loaded.issuer, config.issuer);
    }
}
