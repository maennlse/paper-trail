// SPDX-License-Identifier: GPL-3.0-or-later
//! RFC 8628 through oauth2's device authorization and polling implementation.
//! Only the verification URI/user code leave the worker. Never expose device_code.
use super::{discover, now, Discovery, Tokens};
use crate::{error, Config};
use chrono::{DateTime, Utc};
use oauth2::{
    basic::{BasicClient, BasicErrorResponseType, BasicTokenResponse, BasicTokenType},
    ClientId, DeviceAuthorizationUrl, DeviceCodeErrorResponseType, RequestTokenError, Scope,
    StandardDeviceAuthorizationResponse, SyncHttpClient, TokenResponse, TokenUrl,
};
use std::{
    io,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const PENDING: u8 = 0;
const CANCELLED: u8 = 1;
const COMMITTED: u8 = 2;
const MAX_LIFETIME: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct Signal {
    state: AtomicU8,
    wait: Mutex<()>,
    wake: Condvar,
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<Signal>);
impl Cancellation {
    /// False means identity validation has finished and credentials are committing.
    pub fn cancel(&self) -> bool {
        let _guard = self.0.wait.lock().unwrap_or_else(|e| e.into_inner());
        let result =
            self.0
                .state
                .compare_exchange(PENDING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst);
        self.0.wake.notify_all();
        !matches!(result, Err(COMMITTED))
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.state.load(Ordering::SeqCst) == CANCELLED
    }
    pub(crate) fn commit(&self) -> bool {
        self.0
            .state
            .compare_exchange(PENDING, COMMITTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
    fn wait(&self, duration: Duration) {
        let guard = self.0.wait.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = self
            .0
            .wake
            .wait_timeout_while(guard, duration, |_| !self.is_cancelled())
            .unwrap_or_else(|e| e.into_inner());
    }
}
fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Device sign-in cancelled")
}
fn expired() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "Device code expired. Run :login to request a new code",
    )
}

pub struct Prompt {
    pub verification_uri: String,
    pub user_code: Zeroizing<String>,
    pub expires_at: Instant,
}
impl Prompt {
    pub fn seconds_left(&self) -> u64 {
        self.expires_at
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

struct Session {
    client_id: String,
    token_endpoint: TokenUrl,
    details: StandardDeviceAuthorizationResponse,
    expires_at: Instant,
}
fn verification_url(value: &str) -> bool {
    value.len() <= 4096
        && reqwest::Url::parse(value).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && u.fragment().is_none()
        })
}
fn device_endpoint(discovery: &Discovery) -> io::Result<&str> {
    if discovery
        .grant_types_supported
        .as_ref()
        .is_some_and(|grants| {
            !grants
                .iter()
                .any(|g| g == "urn:ietf:params:oauth:grant-type:device_code")
        })
    {
        return Err(io::Error::new(io::ErrorKind::Unsupported,"Issuer does not support Device Authorization. Enable RFC 8628 or use :login-browser locally"));
    }
    discovery.device_authorization_endpoint.as_deref().filter(|url|crate::https(url)).ok_or_else(||io::Error::new(io::ErrorKind::Unsupported,"Issuer does not advertise a secure device authorization endpoint; use :login-browser locally"))
}
impl Session {
    fn request<C: SyncHttpClient>(
        config: &Config,
        discovery: Discovery,
        http: &C,
        started: Instant,
    ) -> io::Result<Self> {
        let endpoint = device_endpoint(&discovery)?;
        let client = BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_device_authorization_url(
                DeviceAuthorizationUrl::new(endpoint.into())
                    .map_err(|_| error("Invalid device authorization endpoint"))?,
            );
        let mut request = client.exchange_device_code();
        for scope in config.scopes.split_whitespace() {
            request = request.add_scope(Scope::new(scope.into()));
        }
        let details:StandardDeviceAuthorizationResponse = request.request(http).map_err(|err|match err {
            RequestTokenError::ServerResponse(ref response) if matches!(response.error(),BasicErrorResponseType::UnauthorizedClient | BasicErrorResponseType::UnsupportedGrantType | BasicErrorResponseType::InvalidClient) => error("Device authorization is not enabled for this public client. Enable its Device Authorization grant in the provider"),
            _ => error("Device authorization request failed; check issuer, public client ID, grant and scopes"),
        })?;
        Self::from_details(
            config.client_id.clone(),
            TokenUrl::new(discovery.token_endpoint).map_err(|_| error("Invalid token endpoint"))?,
            details,
            started,
        )
    }
    fn from_details(
        client_id: String,
        token_endpoint: TokenUrl,
        details: StandardDeviceAuthorizationResponse,
        started: Instant,
    ) -> io::Result<Self> {
        if details.device_code().secret().is_empty()
            || details.device_code().secret().len() > 16384
            || details.user_code().secret().trim().is_empty()
            || details.user_code().secret() == details.device_code().secret()
            || details.user_code().secret().len() > 128
            || !details
                .user_code()
                .secret()
                .bytes()
                .all(|c| c.is_ascii_graphic() || c == b' ')
            || !verification_url(details.verification_uri().as_str())
            || details
                .verification_uri_complete()
                .is_some_and(|url| !verification_url(url.secret()))
            || details.expires_in().is_zero()
            || details.expires_in() > MAX_LIFETIME
            || details.interval().is_zero()
            || details.interval() > MAX_LIFETIME
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid device authorization response, HTTPS verification URL, or code lifetime",
            ));
        }
        let expires_at = started
            .checked_add(details.expires_in())
            .ok_or_else(expired)?;
        if Instant::now() >= expires_at {
            return Err(expired());
        }
        Ok(Self {
            client_id,
            token_endpoint,
            details,
            expires_at,
        })
    }
    fn prompt(&self) -> Prompt {
        Prompt {
            verification_uri: self.details.verification_uri().to_string(),
            user_code: Zeroizing::new(self.details.user_code().secret().clone()),
            expires_at: self.expires_at,
        }
    }
    fn poll<C, W, T>(
        &self,
        http: &C,
        wait: W,
        clock: T,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> io::Result<Tokens>
    where
        C: SyncHttpClient,
        W: Fn(Duration),
        T: Fn() -> DateTime<Utc> + Send + Sync,
    {
        cancel.check()?;
        let client = BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_token_uri(self.token_endpoint.clone());
        let result = client
            .exchange_device_access_token(&self.details)
            .set_time_fn(clock)
            .set_max_backoff_interval(MAX_LIFETIME)
            .request(http, wait, Some(timeout));
        cancel.check()?;
        let response:BasicTokenResponse = result.map_err(|err|match err {
            RequestTokenError::ServerResponse(ref response) => match response.error() {
                DeviceCodeErrorResponseType::AccessDenied => io::Error::new(io::ErrorKind::PermissionDenied,"Device sign-in was denied. Run :login to try again"),
                DeviceCodeErrorResponseType::ExpiredToken => expired(),
                _ => error("Device token exchange failed; check the public client's Device Authorization grant and scopes"),
            },
            _ => error("Invalid device token response; no credentials were installed"),
        })?;
        if response.token_type() != &BasicTokenType::Bearer
            || response.access_token().secret().is_empty()
        {
            return Err(error("Issuer did not return a Bearer access token"));
        }
        Ok(Tokens {
            access: response.access_token().secret().clone(),
            refresh: response.refresh_token().map(|r| r.secret().clone()),
            expires: now().saturating_add(
                response
                    .expires_in()
                    .unwrap_or(Duration::from_secs(300))
                    .as_secs(),
            ),
        })
    }
}

pub fn login(
    config: &Config,
    show: impl FnOnce(Prompt) -> io::Result<()>,
    cancel: &Cancellation,
) -> io::Result<Tokens> {
    cancel.check()?;
    let discovery = discover(config)?;
    cancel.check()?;
    let http = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| error("Could not initialize device authorization HTTPS client"))?;
    let transport = |request: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
        cancel.check()?;
        http.call(request)
            .map_err(|_| error("Device authorization HTTPS request failed"))
    };
    let session = Session::request(config, discovery, &transport, Instant::now())?;
    cancel.check()?;
    show(session.prompt())?;
    let timeout = session.expires_at.saturating_duration_since(Instant::now());
    let result = session.poll(
        &transport,
        |duration| {
            cancel.wait(duration.min(session.expires_at.saturating_duration_since(Instant::now())))
        },
        || {
            // Use oauth2's polling state machine, with a cancellable monotonic deadline.
            if cancel.is_cancelled() || Instant::now() >= session.expires_at {
                DateTime::<Utc>::MAX_UTC
            } else {
                Utc::now()
            }
        },
        timeout,
        cancel,
    );
    cancel.check()?;
    if Instant::now() >= session.expires_at {
        return Err(expired());
    }
    result
}

#[cfg(test)]
mod tests;
