use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    sync::atomic::AtomicU64,
};

fn details() -> serde_json::Value {
    serde_json::json!({"device_code":"PRIVATE_DEVICE_CODE","user_code":"ABCD-EFGH","verification_uri":"https://issuer.example/activate","verification_uri_complete":"https://issuer.example/activate?user_code=ABCD-EFGH","expires_in":900,"interval":5})
}
fn session(value: serde_json::Value) -> io::Result<Session> {
    Session::from_details(
        "public-native".into(),
        TokenUrl::new("https://issuer.example/token".into()).unwrap(),
        serde_json::from_value(value).unwrap(),
        Instant::now(),
    )
}
fn response(status: u16, value: serde_json::Value) -> oauth2::HttpResponse {
    oauth2::http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&value).unwrap())
        .unwrap()
}
fn clock(seconds: &Arc<AtomicU64>) -> impl Fn() -> DateTime<Utc> + Send + Sync {
    let seconds = seconds.clone();
    move || {
        DateTime::from_timestamp(1_700_000_000 + seconds.load(Ordering::SeqCst) as i64, 0).unwrap()
    }
}

#[test]
fn device_request_is_public_scoped_and_does_not_require_browser_pkce_metadata() {
    let config = Config {
        client_id: "public-native".into(),
        scopes: "openid offline_access notes".into(),
        ..Config::default()
    };
    let discovery = Discovery {
        issuer: "https://issuer.example".into(),
        authorization_endpoint: None,
        token_endpoint: "https://issuer.example/token".into(),
        code_challenge_methods_supported: Some(Vec::new()),
        device_authorization_endpoint: Some("https://issuer.example/device".into()),
        grant_types_supported: Some(vec!["urn:ietf:params:oauth:grant-type:device_code".into()]),
    };
    let http = |request: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
        assert_eq!(request.uri(), "https://issuer.example/device");
        assert_eq!(request.method(), "POST");
        assert!(!request.headers().contains_key("authorization"));
        let body = String::from_utf8(request.into_body()).unwrap();
        assert!(body.contains("client_id=public-native"));
        assert!(body.contains("scope=openid+offline_access+notes"));
        assert!(!body.contains("client_secret"));
        Ok(response(200, details()))
    };
    let session = Session::request(&config, discovery, &http, Instant::now()).unwrap();
    let prompt = session.prompt();
    assert_eq!(prompt.verification_uri, "https://issuer.example/activate");
    assert_eq!(prompt.user_code.as_str(), "ABCD-EFGH");
    assert!(!prompt.verification_uri.contains("PRIVATE_DEVICE_CODE"));
    assert!(prompt.seconds_left() <= 900);
}

#[test]
fn maintained_polling_handles_pending_slow_down_network_backoff_and_access_tokens() {
    let session = session(details()).unwrap();
    let seconds = Arc::new(AtomicU64::new(0));
    let waits = RefCell::new(Vec::new());
    let calls = Cell::new(0);
    let responses = RefCell::new(VecDeque::from([
        Ok(response(
            400,
            serde_json::json!({"error":"authorization_pending"}),
        )),
        Ok(response(400, serde_json::json!({"error":"slow_down"}))),
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "simulated network timeout",
        )),
        Ok(response(
            200,
            serde_json::json!({"access_token":"API_ACCESS_TOKEN","refresh_token":"REFRESH_TOKEN","id_token":"NEVER_USE_AS_API_CREDENTIAL","token_type":"Bearer","expires_in":300}),
        )),
    ]));
    let http = |request: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
        calls.set(calls.get() + 1);
        assert_eq!(request.uri(), "https://issuer.example/token");
        assert!(!request.headers().contains_key("authorization"));
        let body = String::from_utf8(request.into_body()).unwrap();
        assert!(body.contains("device_code=PRIVATE_DEVICE_CODE"));
        assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
        assert!(body.contains("client_id=public-native"));
        assert!(!body.contains("client_secret"));
        responses.borrow_mut().pop_front().unwrap()
    };
    let tokens = session
        .poll(
            &http,
            |duration| {
                waits.borrow_mut().push(duration.as_secs());
                seconds.fetch_add(duration.as_secs(), Ordering::SeqCst);
            },
            clock(&seconds),
            Duration::from_secs(900),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(calls.get(), 4);
    assert_eq!(*waits.borrow(), vec![5, 10, 20]);
    assert_eq!(tokens.access, "API_ACCESS_TOKEN");
    assert_eq!(tokens.refresh.as_deref(), Some("REFRESH_TOKEN"));
}

#[test]
fn denied_expired_malformed_and_non_bearer_responses_do_not_expose_secrets() {
    for (status, value, kind) in [
        (
            400,
            serde_json::json!({"error":"access_denied","error_description":"PRIVATE_DEVICE_CODE"}),
            io::ErrorKind::PermissionDenied,
        ),
        (
            400,
            serde_json::json!({"error":"expired_token"}),
            io::ErrorKind::TimedOut,
        ),
        (
            200,
            serde_json::json!({"id_token":"PRIVATE_DEVICE_CODE","token_type":"Bearer"}),
            io::ErrorKind::Other,
        ),
        (
            200,
            serde_json::json!({"access_token":"PRIVATE_DEVICE_CODE","token_type":"mac"}),
            io::ErrorKind::Other,
        ),
    ] {
        let http = |_: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
            Ok(response(status, value.clone()))
        };
        let err = session(details())
            .unwrap()
            .poll(
                &http,
                |_| {},
                Utc::now,
                Duration::from_secs(900),
                &Cancellation::default(),
            )
            .err()
            .unwrap();
        assert_eq!(err.kind(), kind);
        assert!(!err.to_string().contains("PRIVATE_DEVICE_CODE"));
    }
}

#[test]
fn cancellation_stops_polling_and_cannot_race_a_committed_identity() {
    let cancel = Cancellation::default();
    let wait_cancel = cancel.clone();
    let clock_cancel = cancel.clone();
    let calls = Cell::new(0);
    let http = |_: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
        calls.set(calls.get() + 1);
        Ok(response(
            400,
            serde_json::json!({"error":"authorization_pending"}),
        ))
    };
    let err = session(details())
        .unwrap()
        .poll(
            &http,
            move |_| {
                assert!(wait_cancel.cancel());
            },
            move || {
                if clock_cancel.is_cancelled() {
                    DateTime::<Utc>::MAX_UTC
                } else {
                    Utc::now()
                }
            },
            Duration::from_secs(900),
            &cancel,
        )
        .err()
        .unwrap();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert_eq!(calls.get(), 1);
    assert!(!cancel.commit());
    let committed = Cancellation::default();
    assert!(committed.commit());
    assert!(!committed.cancel());
    assert!(!committed.is_cancelled());
    let early = Cancellation::default();
    assert!(early.cancel());
    let err = session(details())
        .unwrap()
        .poll(
            &http,
            |_| panic!("cancelled request slept"),
            Utc::now,
            Duration::from_secs(900),
            &early,
        )
        .err()
        .unwrap();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert_eq!(calls.get(), 1);
}

#[test]
fn local_expiry_stops_without_requesting_another_code() {
    let seconds = Arc::new(AtomicU64::new(0));
    let calls = Cell::new(0);
    let http = |_: oauth2::HttpRequest| -> io::Result<oauth2::HttpResponse> {
        calls.set(calls.get() + 1);
        Ok(response(
            400,
            serde_json::json!({"error":"authorization_pending"}),
        ))
    };
    let err = session(details())
        .unwrap()
        .poll(
            &http,
            |duration| {
                seconds.fetch_add(duration.as_secs(), Ordering::SeqCst);
            },
            clock(&seconds),
            Duration::from_secs(1),
            &Cancellation::default(),
        )
        .err()
        .unwrap();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert_eq!(calls.get(), 1);
}

#[test]
fn verification_urls_lifetimes_intervals_and_display_codes_are_validated() {
    for (field, value) in [
        (
            "verification_uri",
            serde_json::json!("http://issuer.example/activate"),
        ),
        (
            "verification_uri",
            serde_json::json!("https://user:password@issuer.example/activate"),
        ),
        (
            "verification_uri_complete",
            serde_json::json!("http://issuer.example/activate?user_code=x"),
        ),
        ("expires_in", serde_json::json!(0)),
        ("expires_in", serde_json::json!(3601)),
        ("interval", serde_json::json!(0)),
        ("user_code", serde_json::json!("    ")),
        ("user_code", serde_json::json!("PRIVATE_DEVICE_CODE")),
        ("user_code", serde_json::json!("bad\u{001b}code")),
    ] {
        let mut value_object = details();
        value_object[field] = value;
        assert!(session(value_object).is_err(), "accepted invalid {field}");
    }
    let mut defaults = details();
    defaults.as_object_mut().unwrap().remove("interval");
    assert_eq!(
        session(defaults).unwrap().details.interval(),
        Duration::from_secs(5)
    );
    let mut discovery = Discovery {
        issuer: "https://issuer.example".into(),
        authorization_endpoint: None,
        token_endpoint: "https://issuer.example/token".into(),
        code_challenge_methods_supported: None,
        device_authorization_endpoint: None,
        grant_types_supported: None,
    };
    assert_eq!(
        device_endpoint(&discovery).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    discovery.device_authorization_endpoint = Some("http://issuer.example/device".into());
    assert!(device_endpoint(&discovery).is_err());
    discovery.device_authorization_endpoint = Some("https://issuer.example/device".into());
    discovery.grant_types_supported = Some(vec!["authorization_code".into()]);
    assert!(device_endpoint(&discovery).is_err());
}
