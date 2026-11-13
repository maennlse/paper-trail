// SPDX-License-Identifier: GPL-3.0-or-later
use std::process::{Command, Output};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_paper-trail-server"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PAPERTRAIL_") || key == "RUST_LOG" {
            command.env_remove(key);
        }
    }
    command
}
fn stderr(output: &Output) -> String {
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(!text.contains('\u{1b}'));
    text
}

#[test]
fn startup_errors_and_info_are_logged_to_stderr_by_default() {
    let output = command().output().unwrap();
    assert!(!output.status.success());
    let text = stderr(&output);
    for expected in [
        "Starting encrypted sync server",
        "Server failed",
        "PAPERTRAIL_OIDC_ISSUER is required",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn verbose_flags_and_environment_filter_control_debug_logs() {
    // Invalid HTTP issuer fails before discovery can make any network request.
    for (args, filter, debug) in [
        (vec![], None, false),
        (vec!["--verbose"], None, true),
        (vec!["-v"], None, true),
        (
            vec!["--verbose"],
            Some("warn,papertrail_server=info"),
            false,
        ),
        (vec![], Some("warn,papertrail_server=debug"), true),
    ] {
        let mut command = command();
        command
            .args(args)
            .env("PAPERTRAIL_OIDC_ISSUER", "http://invalid.example")
            .env("PAPERTRAIL_API_AUDIENCE", "test");
        if let Some(filter) = filter {
            command.env("RUST_LOG", filter);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let text = stderr(&output);
        assert_eq!(
            text.contains("Discovering OIDC configuration"),
            debug,
            "{text}"
        );
        assert!(text.contains("Starting encrypted sync server"));
    }
}

#[test]
fn help_describes_logging_without_requiring_configuration() {
    let output = command().arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("--verbose"));
    assert!(text.contains("RUST_LOG"));
    assert!(text.contains("PAPERTRAIL_NATIVE_CLIENT_ID"));
    assert!(text.contains("PAPERTRAIL_NATIVE_SCOPES"));
    assert!(text.contains("openid offline_access"));
}

#[test]
fn invalid_public_native_settings_fail_before_network_and_do_not_echo_values() {
    for (id, scopes) in [
        ("", "openid"),
        ("native", "openid  SCOPE_PRIVATE"),
        ("NATIVE_PRIVATE\n", "openid"),
    ] {
        let output = command()
            .arg("--verbose")
            .env("PAPERTRAIL_OIDC_ISSUER", "https://never-contact.example")
            .env("PAPERTRAIL_API_AUDIENCE", "SERVER_AUDIENCE_PRIVATE")
            .env("PAPERTRAIL_NATIVE_CLIENT_ID", id)
            .env("PAPERTRAIL_NATIVE_SCOPES", scopes)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = stderr(&output);
        assert!(text.contains("Invalid public native client ID or scopes"));
        for excluded in [
            "Discovering OIDC configuration",
            "SCOPE_PRIVATE",
            "NATIVE_PRIVATE",
            "SERVER_AUDIENCE_PRIVATE",
        ] {
            assert!(!text.contains(excluded));
        }
    }
}
