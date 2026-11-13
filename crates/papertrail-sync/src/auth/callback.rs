// SPDX-License-Identifier: GPL-3.0-or-later
//! Loopback browser replies contain no OAuth parameters, tokens or provider errors.
use std::{
    collections::HashMap,
    io::{self, Write},
};

pub(super) fn code(line: &str, state: &str, issuer: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" || !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{path}")).ok()?;
    if url.path() != "/callback" || url.fragment().is_some() {
        return None;
    }
    let mut fields = HashMap::new();
    for (name, value) in url.query_pairs() {
        if fields
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return None;
        }
    }
    if fields.get("state").map(String::as_str) != Some(state)
        || fields.get("iss").is_some_and(|value| value != issuer)
        || fields.contains_key("error")
    {
        return None;
    }
    fields.remove("code").filter(|code| !code.is_empty())
}

const CLOSE_SCRIPT: &str = "try { window.history.replaceState(null, '', '/callback'); } catch (_) {}\nwindow.setTimeout(() => window.close(), 3000);";

fn respond(stream: &mut impl Write, status: &str, message: &str, close: bool) -> io::Result<()> {
    // Only this freshly generated nonce is interpolated, never request parameters.
    let nonce = oauth2::CsrfToken::new_random();
    let nonce = nonce.secret();
    let script = if close {
        format!("<script nonce=\"{nonce}\">{CLOSE_SCRIPT}</script>")
    } else {
        String::new()
    };
    let script_policy = if close {
        format!("; script-src 'nonce-{nonce}'")
    } else {
        String::new()
    };
    let body = format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Paper Trail Sign-In</title></head><body><h1>Paper Trail</h1><p>{message}</p>{script}</body></html>");
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'{script_policy}\r\nConnection: close\r\n\r\n{body}", body.len())
}

pub(super) fn invalid(stream: &mut impl Write) -> io::Result<()> {
    respond(
        stream,
        "400 Bad Request",
        "Invalid sign-in callback. Return to Paper Trail for details. You can close this tab.",
        false,
    )
}

/// Only a completed code exchange gets the auto-close page. Browser write errors
/// must not discard successfully received credentials (e.g. the tab already closed).
pub(super) fn finish<T>(stream: &mut impl Write, result: io::Result<T>) -> io::Result<T> {
    let _ = if result.is_ok() {
        respond(stream, "200 OK", "Browser authorization complete. Return to Paper Trail to finish signing in. This tab will try to close in 3 seconds. If it stays open, you can safely close it manually.", true)
    } else {
        respond(stream, "500 Internal Server Error", "Sign-in could not be completed. Return to Paper Trail for details. You can close this tab.", false)
    };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_callbacks_keep_state_issuer_and_duplicate_parameter_checks() {
        assert_eq!(
            code(
                "GET /callback?state=expected&code=secret HTTP/1.1",
                "expected",
                "https://issuer.example"
            ),
            Some("secret".into())
        );
        assert_eq!(code("GET /callback?state=expected&code=secret&iss=https%3A%2F%2Fissuer.example HTTP/1.1", "expected", "https://issuer.example"), Some("secret".into()));
        for request in [
            "GET /callback?state=wrong&code=secret HTTP/1.1",
            "GET /callback?state=expected&code=secret&iss=https%3A%2F%2Fother.example HTTP/1.1",
            "GET /callback?state=wrong&state=expected&code=secret HTTP/1.1",
            "GET /callback?state=expected&code=first&code=second HTTP/1.1",
            "GET /callback?state=expected&code=secret&error=access_denied HTTP/1.1",
            "GET /callback?state=expected&error=access_denied HTTP/1.1",
            "GET /callback?state=expected&code= HTTP/1.1",
            "GET /callback?code=secret HTTP/1.1",
            "GET /wrong?state=expected&code=secret HTTP/1.1",
            "GET //evil.example/callback?state=expected&code=secret HTTP/1.1",
            "GET https://evil.example/callback?state=expected&code=secret HTTP/1.1",
            "POST /callback?state=expected&code=secret HTTP/1.1",
        ] {
            assert!(
                code(request, "expected", "https://issuer.example").is_none(),
                "accepted {request}"
            );
        }
    }

    #[test]
    fn successful_exchange_attempts_delayed_close_with_safe_headers_and_manual_fallback() {
        let mut bytes = Vec::new();
        assert_eq!(
            finish(&mut bytes, Ok("ACCESS_TOKEN_PRIVATE")).unwrap(),
            "ACCESS_TOKEN_PRIVATE"
        );
        let response = String::from_utf8(bytes).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        for expected in [
            "Content-Type: text/html; charset=utf-8",
            "Cache-Control: no-store",
            "Referrer-Policy: no-referrer",
            "X-Content-Type-Options: nosniff",
            "default-src 'none'",
            "frame-ancestors 'none'",
            "window.setTimeout(() => window.close(), 3000)",
            "window.history.replaceState(null, '', '/callback')",
            "close it manually",
        ] {
            assert!(response.contains(expected), "missing {expected}");
        }
        assert!(!response.contains("ACCESS_TOKEN_PRIVATE"));
        let nonce = response
            .split("script-src 'nonce-")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        assert!(response.contains(&format!("<script nonce=\"{nonce}\">")));
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.contains(&format!("Content-Length: {}\r\n", body.len())));
        let mut other = Vec::new();
        finish(&mut other, Ok(())).unwrap();
        assert_ne!(response, String::from_utf8(other).unwrap());
    }

    #[test]
    fn invalid_callbacks_and_failed_exchanges_stay_visible_without_leaking_errors() {
        let mut invalid_bytes = Vec::new();
        invalid(&mut invalid_bytes).unwrap();
        let mut failed = Vec::new();
        let result = finish::<()>(&mut failed, Err(io::Error::other("PROVIDER_ERROR_PRIVATE")));
        assert!(result.is_err());
        for bytes in [invalid_bytes, failed] {
            let response = String::from_utf8(bytes).unwrap();
            assert!(!response.contains("<script"));
            assert!(!response.contains("window.close"));
            assert!(!response.contains("PROVIDER_ERROR_PRIVATE"));
            assert!(response.contains("You can close this tab"));
            assert!(response.contains("Cache-Control: no-store"));
        }
    }

    #[test]
    fn a_closed_browser_does_not_discard_successful_credentials() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            finish(&mut Closed, Ok("ACCESS_TOKEN_PRIVATE")).unwrap(),
            "ACCESS_TOKEN_PRIVATE"
        );
    }
}
