// SPDX-License-Identifier: GPL-3.0-or-later
//! Native public-client OAuth, using OIDC discovery. ID tokens are never API credentials.
use crate::{error, http, https, Config};
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge,
    RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    net::TcpListener,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};
mod callback;
pub mod config;
pub mod device;

pub struct LoginFlow {
    verifier: oauth2::PkceCodeVerifier,
    state: CsrfToken,
    config: Config,
    redirect: String,
    token_endpoint: String,
}
impl LoginFlow {
    /// A native host opens this URL using its system browser and passes
    /// its callback URL to complete(). No client secret or ID token is used.
    pub fn begin(config: &Config, redirect: &str) -> io::Result<(Self, String)> {
        let parsed =
            reqwest::Url::parse(redirect).map_err(|_| error("Invalid native redirect URI"))?;
        if parsed.scheme() != "https" && parsed.scheme() != "io.github.maennlse.paper-trail" {
            return Err(error(
                "Use a registered HTTPS or Paper Trail application redirect",
            ));
        }
        let d = discover(config)?;
        let client = BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_auth_uri(
                AuthUrl::new(d.browser_endpoint()?.to_owned())
                    .map_err(|_| error("Invalid authorization endpoint"))?,
            )
            .set_redirect_uri(
                RedirectUrl::new(redirect.into()).map_err(|_| error("Invalid redirect"))?,
            );
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(challenge);
        for scope in config.scopes.split_whitespace() {
            request = request.add_scope(Scope::new(scope.into()));
        }
        let (url, state) = request.url();
        Ok((
            Self {
                verifier,
                state,
                config: config.clone(),
                redirect: redirect.into(),
                token_endpoint: d.token_endpoint,
            },
            url.to_string(),
        ))
    }
    pub fn complete(self, callback: &str) -> io::Result<Tokens> {
        let url = reqwest::Url::parse(callback).map_err(|_| error("Invalid sign-in callback"))?;
        let expected =
            reqwest::Url::parse(&self.redirect).map_err(|_| error("Invalid redirect"))?;
        if url.scheme() != expected.scheme()
            || url.host_str() != expected.host_str()
            || url.path() != expected.path()
            || url.port() != expected.port()
        {
            return Err(error("Wrong sign-in callback URI"));
        }
        let mut fields = std::collections::HashMap::new();
        for (k, v) in url.query_pairs() {
            if fields.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err(error("Duplicate sign-in callback parameter"));
            }
        }
        if fields.get("state") != Some(self.state.secret())
            || fields
                .get("iss")
                .is_some_and(|iss| iss != &self.config.issuer)
        {
            return Err(error("Sign-in state/issuer mismatch"));
        }
        let code = fields
            .remove("code")
            .ok_or_else(|| error("Sign-in did not provide an authorization code"))?;
        let client = BasicClient::new(ClientId::new(self.config.client_id))
            .set_token_uri(
                TokenUrl::new(self.token_endpoint).map_err(|_| error("Invalid token endpoint"))?,
            )
            .set_redirect_uri(
                RedirectUrl::new(self.redirect).map_err(|_| error("Invalid redirect"))?,
            );
        let response = client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(self.verifier)
            .request(&http()?)
            .map_err(|_| error("Authorization code exchange failed"))?;
        Ok(Tokens {
            access: response.access_token().secret().clone(),
            refresh: response.refresh_token().map(|r| r.secret().clone()),
            expires: now()
                + response
                    .expires_in()
                    .unwrap_or(Duration::from_secs(300))
                    .as_secs(),
        })
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires: u64,
}
#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: Option<String>,
    token_endpoint: String,
    code_challenge_methods_supported: Option<Vec<String>>,
    device_authorization_endpoint: Option<String>,
    grant_types_supported: Option<Vec<String>>,
}
impl Discovery {
    fn browser_endpoint(&self) -> io::Result<&str> {
        if self
            .code_challenge_methods_supported
            .as_ref()
            .is_some_and(|m| !m.iter().any(|m| m == "S256"))
        {
            return Err(error("Issuer does not support PKCE S256"));
        }
        self.authorization_endpoint
            .as_deref()
            .ok_or_else(|| error("Issuer does not advertise browser authorization"))
    }
}
fn discover(config: &Config) -> io::Result<Discovery> {
    config.validate()?;
    config::check(config)?;
    discover_issuer(&config.issuer)
}
fn discover_issuer(issuer: &str) -> io::Result<Discovery> {
    if !https(issuer) {
        return Err(error("An exact HTTPS OIDC issuer is required"));
    }
    let response = http()?
        .get(format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ))
        .send()
        .map_err(|_| error("OIDC discovery failed"))?;
    if !response.status().is_success() {
        return Err(error(
            "OIDC discovery status failed (redirects are not allowed)",
        ));
    }
    let d: Discovery = config::decode(response, 65536)?;
    validate_discovery(d, issuer)
}
fn validate_discovery(d: Discovery, issuer: &str) -> io::Result<Discovery> {
    if d.issuer != issuer
        || d.authorization_endpoint
            .as_ref()
            .is_some_and(|url| !https(url))
        || !https(&d.token_endpoint)
    {
        return Err(error("OIDC issuer/endpoint validation failed"));
    }
    Ok(d)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn login(config: &Config) -> io::Result<Tokens> {
    let d = discover(config)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(
            AuthUrl::new(d.browser_endpoint()?.to_owned())
                .map_err(|_| error("Invalid authorization endpoint"))?,
        )
        .set_token_uri(
            TokenUrl::new(d.token_endpoint).map_err(|_| error("Invalid token endpoint"))?,
        )
        .set_redirect_uri(RedirectUrl::new(redirect.clone()).map_err(error)?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge);
    for scope in config.scopes.split_whitespace() {
        request = request.add_scope(Scope::new(scope.into()));
    }
    let (url, state) = request.url();
    webbrowser::open(url.as_str()).map_err(|_| error("Could not open the system browser"))?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let (code, mut stream) = loop {
        if Instant::now() >= deadline {
            return Err(error("Browser sign-in timed out"));
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut bytes = [0; 8192];
                let n = stream.read(&mut bytes).unwrap_or(0);
                let line = std::str::from_utf8(&bytes[..n])
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                if let Some(code) = callback::code(line, state.secret(), &config.issuer) {
                    break (code, stream);
                }
                let _ = callback::invalid(&mut stream);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e),
        }
    };
    let response = callback::finish(
        &mut stream,
        http().and_then(|http| {
            client
                .exchange_code(AuthorizationCode::new(code))
                .set_pkce_verifier(verifier)
                .request(&http)
                .map_err(|_| error("Authorization code exchange failed"))
        }),
    )?;
    Ok(Tokens {
        access: response.access_token().secret().clone(),
        refresh: response.refresh_token().map(|r| r.secret().clone()),
        expires: now()
            + response
                .expires_in()
                .unwrap_or(Duration::from_secs(300))
                .as_secs(),
    })
}
pub fn refresh(config: &Config, tokens: &Tokens) -> io::Result<Tokens> {
    let d = discover(config)?;
    let client = BasicClient::new(ClientId::new(config.client_id.clone())).set_token_uri(
        TokenUrl::new(d.token_endpoint).map_err(|_| error("Invalid token endpoint"))?,
    );
    let refresh = RefreshToken::new(
        tokens
            .refresh
            .as_ref()
            .ok_or_else(|| error("Sign in again: no refresh token"))?
            .clone(),
    );
    let response = client
        .exchange_refresh_token(&refresh)
        .request(&http()?)
        .map_err(|_| error("Token refresh failed; sign in again"))?;
    Ok(Tokens {
        access: response.access_token().secret().clone(),
        refresh: response
            .refresh_token()
            .map(|r| r.secret().clone())
            .or_else(|| tokens.refresh.clone()),
        expires: now()
            + response
                .expires_in()
                .unwrap_or(Duration::from_secs(300))
                .as_secs(),
    })
}
pub fn access(config: &Config, tokens: &mut Tokens) -> io::Result<String> {
    access_with_persistence(config, tokens, true)
}
pub fn access_with_persistence(
    config: &Config,
    tokens: &mut Tokens,
    persist: bool,
) -> io::Result<String> {
    if tokens.expires <= now() + 30 {
        *tokens = refresh(config, tokens)?;
        if persist {
            let _ = save(config, tokens);
        }
    }
    Ok(tokens.access.clone())
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn entry(config: &Config) -> io::Result<keyring::Entry> {
    let id = papertrail_crypto::digest(
        format!("{}\n{}\n{}", config.server, config.issuer, config.client_id).as_bytes(),
    );
    keyring::Entry::new("io.github.maennlse.paper-trail.sync", &id)
        .map_err(|_| error("Secure credential storage unavailable"))
}
pub fn save(config: &Config, tokens: &Tokens) -> io::Result<()> {
    let json = Zeroizing::new(serde_json::to_string(tokens).map_err(error)?);
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        entry(config)?.set_password(&json).map_err(|_| {
            error("Secure credential storage unavailable; signed in for this session only")
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (config, json);
        Err(error("Use platform secure storage"))
    }
}
pub fn load(config: &Config) -> io::Result<Tokens> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let json = Zeroizing::new(
            entry(config)?
                .get_password()
                .map_err(|_| error("Sign in to sync"))?,
        );
        serde_json::from_str(&json).map_err(|_| error("Invalid stored credentials; sign in again"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = config;
        Err(error("Use platform secure storage"))
    }
}
pub fn logout(config: &Config) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        match entry(config)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(error("Could not remove stored credentials")),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oidc_metadata_requires_exact_advertised_issuer_and_secure_endpoints() {
        let valid = serde_json::json!({"issuer":"https://issuer.example/tenant", "authorization_endpoint":"https://issuer.example/authorize", "token_endpoint":"https://issuer.example/token", "device_authorization_endpoint":"https://issuer.example/device", "code_challenge_methods_supported":["S256"]});
        let issuer = "https://issuer.example/tenant";
        let d = validate_discovery(serde_json::from_value(valid.clone()).unwrap(), issuer).unwrap();
        assert!(d.browser_endpoint().is_ok());
        for (field, value) in [
            ("issuer", "https://issuer.example/tenant/"),
            ("issuer", "https://evil.example"),
            ("authorization_endpoint", "javascript:alert(1)"),
            ("token_endpoint", "http://issuer.example/token"),
        ] {
            let mut bad = valid.clone();
            bad[field] = serde_json::json!(value);
            assert!(validate_discovery(serde_json::from_value(bad).unwrap(), issuer).is_err());
        }
        let mut bad = valid;
        bad["code_challenge_methods_supported"] = serde_json::json!(["plain"]);
        let d = validate_discovery(serde_json::from_value(bad).unwrap(), issuer).unwrap();
        assert!(d.browser_endpoint().is_err());
    }
    fn flow() -> LoginFlow {
        let (_, verifier) = PkceCodeChallenge::new_random_sha256();
        LoginFlow {
            verifier,
            state: CsrfToken::new("expected".into()),
            config: Config {
                version: 1,
                server: "https://sync.example".into(),
                issuer: "https://issuer.example".into(),
                client_id: "native".into(),
                scopes: "openid".into(),
                folders: vec![std::path::PathBuf::from("/notes")],
                ..Config::default()
            },
            redirect: "io.github.maennlse.paper-trail:/oauth/callback".into(),
            token_endpoint: "https://issuer.example/token".into(),
        }
    }
    #[test]
    fn native_callbacks_reject_state_issuer_duplicates_and_wrong_redirects_before_exchange() {
        for callback in [
            "io.github.maennlse.paper-trail:/oauth/callback?state=wrong&code=never-exchange",
            "io.github.maennlse.paper-trail:/oauth/callback?state=expected&iss=https%3A%2F%2Fevil.example&code=never-exchange",
            "io.github.maennlse.paper-trail:/oauth/callback?state=expected&state=expected&code=never-exchange",
            "io.github.maennlse.paper-trail:/wrong?state=expected&code=never-exchange",
            "io.github.maennlse.paper-trail:/oauth/callback?state=expected",
        ] { assert!(flow().complete(callback).is_err()); }
    }
}
