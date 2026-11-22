// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed transport diagnostics: never expose URLs, credentials or backend messages.
use std::{error::Error, io};

pub(crate) fn discovery_error(context: &str, failure: reqwest::Error) -> io::Error {
    let reason = if failure.is_timeout() {
        "request timed out; check connectivity and the server"
    } else {
        source_reason(&failure).unwrap_or(if failure.is_connect() {
            "connection failed; check connectivity, proxy settings and the server"
        } else {
            "network request failed; check connectivity and the server"
        })
    };
    crate::error(format!("{context}: {reason}"))
}

fn source_reason(failure: &(dyn Error + 'static)) -> Option<&'static str> {
    // Reqwest does not expose typed DNS/TLS categories. Recognize known backend
    // diagnostics only in the source chain, never the top-level URL-bearing error.
    // Unknown backend errors deliberately fall back to a generic category.
    let mut source = failure.source();
    while let Some(cause) = source {
        let message = cause.to_string().to_ascii_lowercase();
        if message == "dns error" {
            return Some("DNS lookup failed; check the server hostname and DNS settings");
        }
        if message.starts_with("invalid peer certificate:") {
            return Some(
                "TLS certificate verification failed; check the server certificate chain and system clock",
            );
        }
        if message == "tls handshake eof" || message.starts_with("received fatal alert:") {
            return Some("TLS handshake failed; check the server and any HTTPS proxy");
        }
        source = cause.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Cause(&'static str, Option<Box<Cause>>);
    impl std::fmt::Display for Cause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl Error for Cause {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1.as_deref().map(|e| e as &dyn Error)
        }
    }

    #[test]
    fn backend_categories_never_echo_private_details() {
        for (message, expected) in [
            ("dns error", "DNS lookup failed"),
            (
                "invalid peer certificate: PRIVATE_CERTIFICATE_DETAILS",
                "TLS certificate verification failed",
            ),
            ("tls handshake eof", "TLS handshake failed"),
            (
                "received fatal alert: PRIVATE_ALERT",
                "TLS handshake failed",
            ),
        ] {
            let failure = Cause(
                "https://user:PRIVATE_PASSWORD@example.test/PRIVATE_PATH",
                Some(Box::new(Cause(
                    "connection wrapper",
                    Some(Box::new(Cause(message, None))),
                ))),
            );
            let reason = source_reason(&failure).unwrap();
            assert!(reason.starts_with(expected));
            assert!(!reason.contains("PRIVATE"));
            assert!(!reason.contains("example.test"));
        }
    }

    #[test]
    fn unknown_sources_and_top_level_messages_are_not_classified() {
        assert!(source_reason(&Cause("dns error", None)).is_none());
        assert!(source_reason(&Cause(
            "request failed",
            Some(Box::new(Cause("PRIVATE_BACKEND_ERROR", None))),
        ))
        .is_none());
    }
}
