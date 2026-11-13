// SPDX-License-Identifier: GPL-3.0-or-later
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use papertrail_crypto::Account;
use serde::Deserialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

/// Public/native clients still have no secret. These credentials belong only to
/// the resource server, for authenticated RFC 7662 requests to the trusted issuer.
pub enum TokenValidation {
    Jwt,
    Introspection {
        client_id: String,
        client_secret: Zeroizing<String>,
    },
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: Option<String>,
    introspection_endpoint: Option<String>,
    introspection_endpoint_auth_methods_supported: Option<Vec<String>>,
}
#[derive(Clone, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
}
struct Cache {
    keys: JwkSet,
    fetched: Instant,
}
#[derive(Debug)]
pub struct InvalidToken;
fn rejected(reason: &'static str) -> InvalidToken {
    tracing::warn!(reason, "Access token rejected");
    InvalidToken
}
fn jwt_error(error: jsonwebtoken::errors::Error) -> InvalidToken {
    use jsonwebtoken::errors::ErrorKind;
    let reason = match error.kind() {
        ErrorKind::ExpiredSignature => "expired",
        ErrorKind::InvalidIssuer => "issuer_mismatch",
        ErrorKind::InvalidAudience => "audience_mismatch",
        ErrorKind::ImmatureSignature => "not_yet_valid",
        ErrorKind::InvalidSignature => "invalid_signature",
        ErrorKind::MissingRequiredClaim(_) => "missing_required_claim",
        _ => "invalid_jwt",
    };
    rejected(reason)
}
#[derive(Debug)]
pub enum AuthError {
    InvalidToken,
    Unavailable,
}
enum Validator {
    Jwt {
        jwks_uri: String,
        cache: Mutex<Cache>,
    },
    Introspection {
        endpoint: String,
        client_id: String,
        client_secret: Zeroizing<String>,
    },
}
pub struct Authenticator {
    issuer: String,
    audience: String,
    client: reqwest::Client,
    validator: Validator,
}
pub fn https(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
    })
}
impl Authenticator {
    /// The same exact issuer used by token validation, not a second setting.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub async fn discover(issuer: String, audience: String) -> Result<Self, &'static str> {
        Self::discover_with_mode(issuer, audience, TokenValidation::Jwt).await
    }
    pub async fn discover_with_mode(
        issuer: String,
        audience: String,
        mode: TokenValidation,
    ) -> Result<Self, &'static str> {
        tracing::debug!("Discovering OIDC configuration");
        if !https(&issuer) || audience.is_empty() {
            return Err("HTTPS issuer and API audience required");
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "HTTP client")?;
        let d: Discovery = client
            .get(format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ))
            .send()
            .await
            .map_err(|_| "OIDC discovery failed")?
            .error_for_status()
            .map_err(|_| "OIDC discovery status")?
            .json()
            .await
            .map_err(|_| "OIDC discovery format")?;
        if d.issuer != issuer {
            return Err("OIDC issuer mismatch");
        }
        let validator = match mode {
            TokenValidation::Jwt => {
                let jwks_uri = d
                    .jwks_uri
                    .filter(|uri| https(uri))
                    .ok_or("Missing or insecure JWKS URI")?;
                let keys = client
                    .get(&jwks_uri)
                    .send()
                    .await
                    .map_err(|_| "JWKS fetch failed")?
                    .error_for_status()
                    .map_err(|_| "JWKS status")?
                    .json()
                    .await
                    .map_err(|_| "JWKS format")?;
                Validator::Jwt {
                    jwks_uri,
                    cache: Mutex::new(Cache {
                        keys,
                        fetched: Instant::now(),
                    }),
                }
            }
            TokenValidation::Introspection {
                client_id,
                client_secret,
            } => {
                if client_id.is_empty() || client_secret.is_empty() {
                    return Err("Introspection requires server-only client ID and secret");
                }
                if d.introspection_endpoint_auth_methods_supported
                    .as_ref()
                    .is_some_and(|methods| !methods.iter().any(|m| m == "client_secret_basic"))
                {
                    return Err("Issuer does not advertise client_secret_basic introspection authentication");
                }
                let endpoint = d
                    .introspection_endpoint
                    .filter(|uri| https(uri))
                    .ok_or("Missing or insecure introspection endpoint")?;
                Validator::Introspection {
                    endpoint,
                    client_id,
                    client_secret,
                }
            }
        };
        tracing::info!(
            validation_mode = match &validator {
                Validator::Jwt { .. } => "jwt",
                Validator::Introspection { .. } => "introspection",
            },
            issuer,
            audience,
            "Authentication configured"
        );
        Ok(Self {
            issuer,
            audience,
            client,
            validator,
        })
    }
    pub async fn authenticate(&self, token: &str) -> Result<Account, AuthError> {
        if token.is_empty() || token.len() > 16384 {
            rejected("empty_or_oversized_token");
            return Err(AuthError::InvalidToken);
        }
        match &self.validator {
            Validator::Jwt { jwks_uri, cache } => self
                .authenticate_jwt(token, jwks_uri, cache)
                .await
                .map_err(|_| AuthError::InvalidToken),
            Validator::Introspection {
                endpoint,
                client_id,
                client_secret,
            } => {
                self.introspect(token, endpoint, client_id, client_secret)
                    .await
            }
        }
    }
    async fn authenticate_jwt(
        &self,
        token: &str,
        jwks_uri: &str,
        cache: &Mutex<Cache>,
    ) -> Result<Account, InvalidToken> {
        let header = decode_header(token).map_err(jwt_error)?;
        // RFC 9068 access-token profile. ID tokens (usually typ JWT) are rejected.
        if !matches!(
            header.typ.as_deref(),
            Some("at+jwt") | Some("application/at+jwt")
        ) {
            return Err(rejected(
                "unsupported_jwt_type_use_introspection_for_zitadel",
            ));
        }
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err(rejected("unsupported_jwt_algorithm"));
        }
        let kid = header
            .kid
            .as_deref()
            .ok_or_else(|| rejected("missing_key_id"))?;
        let mut cache = cache.lock().await;
        if cache.fetched.elapsed() > Duration::from_secs(300)
            || (cache.keys.find(kid).is_none() && cache.fetched.elapsed() > Duration::from_secs(30))
        {
            // Rate-limit unknown-kid refreshes, including failed refresh attempts.
            cache.fetched = Instant::now();
            tracing::debug!("Refreshing OIDC signing keys");
            cache.keys = self
                .client
                .get(jwks_uri)
                .send()
                .await
                .map_err(|_| {
                    tracing::error!("JWKS refresh transport failure");
                    InvalidToken
                })?
                .error_for_status()
                .map_err(|e| {
                    tracing::error!(
                        status = e.status().map(|s| s.as_u16()),
                        "JWKS refresh HTTP failure"
                    );
                    InvalidToken
                })?
                .json()
                .await
                .map_err(|_| {
                    tracing::error!("Malformed JWKS response");
                    InvalidToken
                })?;
        }
        verify(token, &self.issuer, &self.audience, &cache.keys)
    }
    async fn introspect(
        &self,
        token: &str,
        endpoint: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Account, AuthError> {
        // RFC 6749 section 2.3.1: form-encode each credential before Basic auth.
        let id = form_credential(client_id)?;
        let secret = form_credential(client_secret)?;
        tracing::debug!("Introspecting access token");
        let mut response = self
            .client
            .post(endpoint)
            .basic_auth(id.as_str(), Some(secret.as_str()))
            .form(&[("token", token), ("token_type_hint", "access_token")])
            .send()
            .await
            .map_err(|e| {
                tracing::error!(timeout = e.is_timeout(), "Introspection transport failure");
                AuthError::Unavailable
            })?;
        // Bad resource-server credentials, outages and malformed provider responses
        // must not be misreported as a bad end-user login. Never echo their bodies.
        if !response.status().is_success() {
            tracing::error!(status = response.status().as_u16(), "Introspection HTTP failure; check server API credentials and provider availability");
            return Err(AuthError::Unavailable);
        }
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            tracing::error!("Introspection response read failed");
            AuthError::Unavailable
        })? {
            if body.len() + chunk.len() > 65536 {
                tracing::error!("Introspection response exceeds size limit");
                return Err(AuthError::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        let claims: Introspection = serde_json::from_slice(&body).map_err(|_| {
            tracing::error!("Malformed introspection response");
            AuthError::Unavailable
        })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::Unavailable)?
            .as_secs();
        claims
            .validate(&self.issuer, &self.audience, now)
            .map_err(|_| AuthError::InvalidToken)
    }
}

fn form_credential(value: &str) -> Result<Zeroizing<String>, AuthError> {
    let encoded = Zeroizing::new(
        serde_urlencoded::to_string([("v", value)]).map_err(|_| AuthError::Unavailable)?,
    );
    Ok(Zeroizing::new(
        encoded
            .strip_prefix("v=")
            .ok_or(AuthError::Unavailable)?
            .to_owned(),
    ))
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}
#[derive(Deserialize)]
struct Introspection {
    active: bool,
    iss: Option<String>,
    sub: Option<String>,
    aud: Option<Audience>,
    exp: Option<u64>,
    nbf: Option<u64>,
    token_type: Option<String>,
}
impl Introspection {
    fn validate(self, issuer: &str, audience: &str, now: u64) -> Result<Account, InvalidToken> {
        let intended = match &self.aud {
            Some(Audience::One(a)) => a == audience,
            Some(Audience::Many(a)) => a.iter().any(|a| a == audience),
            None => false,
        };
        let reason = if !self.active {
            Some("inactive_token")
        } else if self.iss.as_deref() != Some(issuer) {
            Some("issuer_missing_or_mismatch")
        } else if !intended {
            Some("audience_missing_or_mismatch")
        } else if !self.exp.is_some_and(|exp| exp > now) {
            Some("expired_or_missing_expiry")
        } else if self.nbf.is_some_and(|nbf| nbf > now.saturating_add(30)) {
            Some("not_yet_valid")
        } else if !self
            .token_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("Bearer"))
        {
            Some("missing_or_non_bearer_token_type")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(rejected(reason));
        }
        let sub = self
            .sub
            .filter(|s| !s.is_empty())
            .ok_or_else(|| rejected("missing_subject"))?;
        tracing::debug!("Access token validated by introspection");
        Ok(Account {
            iss: issuer.into(),
            sub,
        })
    }
}
pub fn verify(
    token: &str,
    issuer: &str,
    audience: &str,
    keys: &JwkSet,
) -> Result<Account, InvalidToken> {
    let header = decode_header(token).map_err(jwt_error)?;
    if !matches!(
        header.typ.as_deref(),
        Some("at+jwt") | Some("application/at+jwt")
    ) || !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256)
    {
        return Err(rejected("unsupported_jwt_type_or_algorithm"));
    }
    let jwk = keys
        .find(
            header
                .kid
                .as_deref()
                .ok_or_else(|| rejected("missing_key_id"))?,
        )
        .ok_or_else(|| rejected("unknown_signing_key"))?;
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|u| *u != jsonwebtoken::jwk::PublicKeyUse::Signature)
        || jwk
            .common
            .key_operations
            .as_ref()
            .is_some_and(|ops| !ops.contains(&jsonwebtoken::jwk::KeyOperations::Verify))
        || jwk
            .common
            .key_algorithm
            .as_ref()
            .is_some_and(|a| format!("{a:?}") != format!("{:?}", header.alg))
    {
        return Err(rejected("unsuitable_signing_key"));
    }
    let key = DecodingKey::from_jwk(jwk).map_err(jwt_error)?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    validation.set_required_spec_claims(&["exp", "iss", "sub", "aud"]);
    validation.validate_nbf = true;
    validation.leeway = 30;
    let c = decode::<Claims>(token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    if c.iss != issuer || c.sub.is_empty() {
        return Err(rejected("issuer_mismatch_or_missing_subject"));
    }
    tracing::debug!("JWT access token validated");
    Ok(Account {
        iss: c.iss,
        sub: c.sub,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    #[tokio::test]
    async fn public_native_config_reuses_validator_issuer_and_never_exposes_server_credentials() {
        use axum::{body::Body, http::Request};
        use http_body_util::BodyExt;
        use std::{path::Path, sync::Arc};
        use tower::ServiceExt;
        let auth = Authenticator {
            issuer: "https://existing-issuer.example/tenant".into(),
            audience: "SERVER_AUDIENCE_PRIVATE".into(),
            client: reqwest::Client::new(),
            validator: Validator::Introspection {
                endpoint: "https://existing-issuer.example/introspect".into(),
                client_id: "CONFIDENTIAL_CLIENT_ID".into(),
                client_secret: Zeroizing::new("CONFIDENTIAL_CLIENT_SECRET".into()),
            },
        };
        let mut server = Arc::new(
            crate::Server::open(Path::new(":memory:"), Some(auth))
                .unwrap()
                .with_native_client(Some(
                    crate::NativeClientConfig::new(
                        "public-native".into(),
                        "openid offline_access provider:api".into(),
                    )
                    .unwrap(),
                )),
        );
        let response = crate::router(server.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.headers().contains_key("x-request-id"));
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"issuer":"https://existing-issuer.example/tenant", "client_id":"public-native", "scope":"openid offline_access provider:api"})
        );
        let body = std::str::from_utf8(&bytes).unwrap();
        for secret in [
            "SERVER_AUDIENCE_PRIVATE",
            "CONFIDENTIAL_CLIENT_ID",
            "CONFIDENTIAL_CLIENT_SECRET",
            "introspect",
            "access_token",
            "refresh_token",
            "vault",
            "private_key",
        ] {
            assert!(!body.contains(secret), "leaked {secret}");
        }
        for path in [
            "/v1/identity",
            "/v1/vault",
            "/v1/changes",
            "/v1/export",
            "/v1/account",
        ] {
            let response = crate::router(server.clone())
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "unprotected {path}");
        }
        for method in ["POST", "PUT", "DELETE"] {
            assert_eq!(
                crate::router(server.clone())
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri("/v1/auth/config")
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                405
            );
        }
        Arc::get_mut(&mut server).unwrap().native_client = None;
        let response = crate::router(server)
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty());
        // An incomplete existing deployment still starts and serves a safe error.
        let server = Arc::new(crate::Server::open(Path::new(":memory:"), None).unwrap());
        let response = crate::router(server)
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty());
    }
    #[test]
    fn rejection_logs_are_specific_and_never_include_claim_values() {
        let logs = crate::logging::tests::capture(|| {
            let valid = serde_json::json!({"active":true,"iss":"https://issuer.example","sub":"SUBJECT_SECRET","aud":"api","exp":2000,"token_type":"Bearer"});
            for (field, value) in [
                ("active", serde_json::json!(false)),
                ("iss", serde_json::json!("ISSUER_SECRET")),
                ("aud", serde_json::json!("AUDIENCE_SECRET")),
                ("exp", serde_json::json!(900)),
                ("token_type", serde_json::json!("TYPE_SECRET")),
            ] {
                let mut claims = valid.clone();
                claims[field] = value;
                assert!(serde_json::from_value::<Introspection>(claims)
                    .unwrap()
                    .validate("https://issuer.example", "api", 1000)
                    .is_err());
            }
            assert!(verify("TOKEN_SECRET", "https://issuer.example", "api", &keys()).is_err());
        });
        for reason in [
            "inactive_token",
            "issuer_missing_or_mismatch",
            "audience_missing_or_mismatch",
            "expired_or_missing_expiry",
            "missing_or_non_bearer_token_type",
            "invalid_jwt",
        ] {
            assert!(logs.contains(reason), "missing {reason}");
        }
        for secret in [
            "SUBJECT_SECRET",
            "ISSUER_SECRET",
            "AUDIENCE_SECRET",
            "TYPE_SECRET",
            "TOKEN_SECRET",
        ] {
            assert!(!logs.contains(secret));
        }
    }
    fn test_keys() -> &'static (EncodingKey, JwkSet) {
        use aws_lc_rs::{
            rand::SystemRandom,
            signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
        };
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        static KEYS: std::sync::OnceLock<(EncodingKey, JwkSet)> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| {
            // Ephemeral test signing material: never stored in the repository.
            let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING,&SystemRandom::new()).unwrap();
            let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING,document.as_ref()).unwrap();
            let public = pair.public_key().as_ref();
            let keys = serde_json::from_value(serde_json::json!({"keys":[{"kty":"EC","crv":"P-256","x":URL_SAFE_NO_PAD.encode(&public[1..33]),"y":URL_SAFE_NO_PAD.encode(&public[33..65]),"kid":"test","alg":"ES256","use":"sig"}]})).unwrap();
            (EncodingKey::from_ec_der(document.as_ref()),keys)
        })
    }
    fn keys() -> JwkSet {
        test_keys().1.clone()
    }
    #[test]
    fn introspection_checks_active_type_issuer_audience_and_times() {
        let valid = serde_json::json!({"active":true,"iss":"https://issuer.example","sub":"alice","aud":["other","papertrail-api"],"exp":2000,"nbf":900,"token_type":"Bearer"});
        let parse = |v| {
            serde_json::from_value::<Introspection>(v)
                .unwrap()
                .validate("https://issuer.example", "papertrail-api", 1000)
        };
        assert_eq!(parse(valid.clone()).unwrap().sub, "alice");
        let mut single = valid.clone();
        single["aud"] = serde_json::json!("papertrail-api");
        assert!(parse(single).is_ok());
        for (field, value) in [
            ("active", serde_json::json!(false)),
            ("iss", serde_json::json!("https://other.example")),
            ("aud", serde_json::json!(["native-client"])),
            ("exp", serde_json::json!(1000)),
            ("nbf", serde_json::json!(1031)),
            ("sub", serde_json::json!("")),
            ("token_type", serde_json::json!("refresh_token")),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(parse(bad).is_err(), "accepted invalid {field}");
        }
        for field in ["iss", "sub", "aud", "exp", "token_type"] {
            let mut bad = valid.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(parse(bad).is_err(), "accepted missing {field}");
        }
        assert!(parse(serde_json::json!({"active":false})).is_err());
        assert!(
            serde_json::from_value::<Introspection>(serde_json::json!({"active":"true"})).is_err()
        );
        assert_eq!(
            form_credential("client +:secret").unwrap().as_str(),
            "client+%2B%3Asecret"
        );
    }

    #[test]
    fn introspection_accepts_zitadel_style_jwt_and_opaque_access_tokens_only() {
        let logs = crate::logging::tests::capture(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        use axum::{
            extract::Form,
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::post,
            Json, Router,
        };
        use std::collections::HashMap;
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("JWT".into());
        header.kid = Some("test".into());
        let jwt = encode(
            &header,
            &serde_json::json!({"test":"access"}),
            &test_keys().0,
        )
        .unwrap();
        let accepted = jwt.clone();
        let app = Router::new().route("/introspect",post(move |headers:HeaderMap,Form(fields):Form<HashMap<String,String>>| {
            let accepted = accepted.clone();
            async move {
                assert_eq!(headers.get("authorization").unwrap(),"Basic Y2xpZW50OnNlY3JldA==");
                assert_eq!(fields.get("token_type_hint").unwrap(),"access_token");
                let token = fields.get("token").unwrap();
                if ["outage","bad-client","redirect"].contains(&token.as_str()) {
                    return (StatusCode::from_u16(match token.as_str() {"outage" => 503,"bad-client" => 401,_ => 302}).unwrap(), "PROVIDER_BODY_SECRET").into_response();
                }
                if token == "oversized" { return "x".repeat(65537).into_response(); }
                let response = if token == &accepted || token == "opaque-access" {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    serde_json::json!({"active":true,"iss":"https://issuer.example","sub":"alice","aud":["papertrail-api"],"exp":now+300,"token_type":"Bearer"})
                } else { serde_json::json!({"active":false}) };
                Json(response).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Only this private test constructor permits HTTP. Production discovery
        // requires HTTPS and builds an HTTPS-only, no-redirect client.
        let auth = Authenticator {
            issuer: "https://issuer.example".into(),
            audience: "papertrail-api".into(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            validator: Validator::Introspection {
                endpoint: format!("http://{address}/introspect"),
                client_id: "client".into(),
                client_secret: Zeroizing::new("secret".into()),
            },
        };
        assert_eq!(auth.authenticate(&jwt).await.unwrap().sub, "alice");
        assert_eq!(
            auth.authenticate("opaque-access").await.unwrap().sub,
            "alice"
        );
        for token in ["id-token", "refresh-token", "revoked", ""] {
            assert!(matches!(
                auth.authenticate(token).await,
                Err(AuthError::InvalidToken)
            ));
        }
        for token in ["outage", "bad-client", "redirect", "oversized"] {
            assert!(matches!(
                auth.authenticate(token).await,
                Err(AuthError::Unavailable)
            ));
        }
        assert!(matches!(
            auth.authenticate(&"x".repeat(16385)).await,
            Err(AuthError::InvalidToken)
        ));
        // Check the public API's distinction too: user token rejection is 401,
        // upstream/configuration failure is 503, and neither response leaks data.
        use tower::ServiceExt;
        let server = std::sync::Arc::new(
            crate::Server::open(std::path::Path::new(":memory:"), Some(auth)).unwrap(),
        );
        for (token, status) in [("revoked", 401), ("outage", 503)] {
            let request = axum::http::Request::builder()
                .uri("/v1/identity")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap();
            assert_eq!(
                crate::router(server.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
        task.abort();
        });
        });
        for expected in [
            "Introspection HTTP failure",
            "status=401",
            "status=503",
            "inactive_token",
            "Introspection response exceeds size limit",
            "Access token validated by introspection",
        ] {
            assert!(logs.contains(expected), "missing {expected}: {logs}");
        }
        for secret in [
            "PROVIDER_BODY_SECRET",
            "opaque-access",
            "id-token",
            "refresh-token",
            "Y2xpZW50OnNlY3JldA==",
        ] {
            assert!(!logs.contains(secret), "leaked {secret}");
        }
    }
    #[test]
    fn validates_access_token_not_id_token_and_all_claims() {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test".into());
        header.typ = Some("at+jwt".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = serde_json::json!({"iss":"https://issuer.example","sub":"alice","aud":"papertrail-api","exp":now+300,"nbf":now-1});
        let key = test_keys().0.clone();
        let token = encode(&header, &claims, &key).unwrap();
        assert_eq!(
            verify(&token, "https://issuer.example", "papertrail-api", &keys())
                .unwrap()
                .sub,
            "alice"
        );
        assert!(verify(&token, "https://wrong.example", "papertrail-api", &keys()).is_err());
        assert!(verify(&token, "https://issuer.example", "wrong-audience", &keys()).is_err());
        for (field, value) in [
            ("exp", serde_json::json!(now - 100)),
            ("nbf", serde_json::json!(now + 100)),
            ("sub", serde_json::json!("")),
        ] {
            let mut altered = claims.clone();
            altered[field] = value;
            assert!(verify(
                &encode(&header, &altered, &key).unwrap(),
                "https://issuer.example",
                "papertrail-api",
                &keys()
            )
            .is_err());
        }
        header.typ = Some("JWT".into());
        assert!(verify(
            &encode(&header, &claims, &key).unwrap(),
            "https://issuer.example",
            "papertrail-api",
            &keys()
        )
        .is_err());
        header.typ = Some("at+jwt".into());
        header.kid = Some("unknown".into());
        assert!(verify(
            &encode(&header, &claims, &key).unwrap(),
            "https://issuer.example",
            "papertrail-api",
            &keys()
        )
        .is_err());
        let mut tampered = token.into_bytes();
        let last = tampered.len() - 5;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(verify(
            std::str::from_utf8(&tampered).unwrap(),
            "https://issuer.example",
            "papertrail-api",
            &keys()
        )
        .is_err());
    }
}
