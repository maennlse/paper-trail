// SPDX-License-Identifier: GPL-3.0-or-later
//! Allowlisted diagnostics: no headers, bodies, query strings or account identifiers.
use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use tracing::Instrument;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub async fn request(request: Request, next: Next) -> Response {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str())
        .unwrap_or("unmatched")
        .to_owned();
    // Do not trust incoming request IDs or log arbitrary HTTP method strings.
    let method = match request.method().as_str() {
        "GET" => "GET",
        "PUT" => "PUT",
        "POST" => "POST",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "PATCH" => "PATCH",
        _ => "OTHER",
    };
    let span = tracing::info_span!("request", request_id = id, method, route);
    async move {
        let started = Instant::now();
        tracing::debug!("Request started");
        let mut response = next.run(request).await;
        response.headers_mut().insert(
            "x-request-id",
            id.to_string().parse().expect("numeric request ID"),
        );
        let status = response.status().as_u16();
        let duration_ms = started.elapsed().as_millis() as u64;
        if status >= 500 {
            tracing::error!(status, duration_ms, "Request completed");
        } else if status >= 400 {
            tracing::warn!(status, duration_ms, "Request completed");
        } else {
            tracing::info!(status, duration_ms, "Request completed");
        }
        response
    }
    .instrument(span)
    .await
}

pub fn init(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    let default = if verbose {
        "warn,papertrail_server=debug"
    } else {
        "warn,papertrail_server=info"
    };
    let filter = match std::env::var("RUST_LOG") {
        Ok(value) => tracing_subscriber::EnvFilter::try_new(value)?,
        Err(std::env::VarError::NotPresent) => tracing_subscriber::EnvFilter::new(default),
        Err(error) => return Err(error.into()),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .map_err(|_| "Could not initialize server logging")?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        cell::RefCell,
        io::{self, Write},
        sync::{Arc, Mutex, Once},
    };

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    thread_local! {
        static CAPTURE: RefCell<Option<Buffer>> = const { RefCell::new(None) };
    }
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    pub(crate) fn capture(run: impl FnOnce()) -> String {
        // Keep one stable subscriber so parallel tests cannot invalidate cached
        // callsite interest by installing/dropping scoped subscribers. Only the
        // current capture thread's output is collected.
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(|| -> Box<dyn Write> {
                    CAPTURE.with(|capture| -> Box<dyn Write> {
                        match capture.borrow().clone() {
                            Some(buffer) => Box::new(buffer),
                            None => Box::new(io::sink()),
                        }
                    })
                })
                .finish();
            tracing::subscriber::set_global_default(subscriber).unwrap();
        });
        let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
        struct Restore(Option<Buffer>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CAPTURE.with(|capture| capture.replace(self.0.take()));
            }
        }
        let restore = Restore(CAPTURE.with(|capture| capture.replace(Some(buffer.clone()))));
        run();
        drop(restore);
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn captures_are_isolated_across_parallel_threads() {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|id| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let logs = capture(|| {
                        barrier.wait();
                        tracing::warn!(capture_id = id, "capture isolation");
                    });
                    assert!(logs.contains(&format!("capture_id={id}")));
                    assert!(!logs.contains(&format!("capture_id={}", 1 - id)));
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn requests_log_status_and_correlation_without_untrusted_data() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let logs = capture(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let server = Arc::new(
                        crate::Server::open(std::path::Path::new(":memory:"), None).unwrap(),
                    );
                    for (path, authorization, status) in [
                        ("/v1/identity?secret=QUERY_SECRET", None, 401),
                        ("/v1/identity", Some("Bearer TOKEN_SECRET"), 503),
                        ("/UNKNOWN_SECRET", None, 401),
                        ("/v1/notes/NOTE_SECRET/revisions/REVISION_SECRET", None, 401),
                    ] {
                        let mut request = Request::builder()
                            .uri(path)
                            .header("x-request-id", "UNTRUSTED_ID");
                        if let Some(header) = authorization {
                            request = request.header("authorization", header);
                        }
                        let response = crate::router(server.clone())
                            .oneshot(request.body(Body::from("BODY_SECRET")).unwrap())
                            .await
                            .unwrap();
                        assert_eq!(response.status().as_u16(), status);
                        assert!(response.headers()["x-request-id"]
                            .to_str()
                            .unwrap()
                            .parse::<u64>()
                            .is_ok());
                    }
                    let response = axum::Router::new()
                        .route("/ok", axum::routing::get(|| async { "RESPONSE_SECRET" }))
                        .layer(axum::middleware::from_fn(super::request))
                        .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
                        .await
                        .unwrap();
                    assert_eq!(response.status(), 200);
                });
        });
        for expected in [
            "request_id=",
            "method=",
            "/v1/identity",
            "status=401",
            "status=503",
            "status=200",
            "duration_ms=",
            "missing_or_malformed_bearer",
            "Authentication is not configured",
            "/v1/notes/{note}/revisions/{revision}",
        ] {
            assert!(logs.contains(expected), "missing {expected}: {logs}");
        }
        for secret in [
            "QUERY_SECRET",
            "TOKEN_SECRET",
            "UNKNOWN_SECRET",
            "NOTE_SECRET",
            "REVISION_SECRET",
            "UNTRUSTED_ID",
            "BODY_SECRET",
            "RESPONSE_SECRET",
        ] {
            assert!(!logs.contains(secret), "leaked {secret}");
        }
    }
}
