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
    for flag in ["--help", "-h"] {
        let output = command().arg(flag).output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        for expected in [
            "Usage: paper-trail-server [OPTIONS]",
            "-h, --help",
            "-V, --version",
            "-v, --verbose",
            env!("CARGO_PKG_VERSION"),
            "RUST_LOG",
            "PAPERTRAIL_OIDC_ISSUER",
            "PAPERTRAIL_API_AUDIENCE",
            "PAPERTRAIL_DATABASE",
            "papertrail.sqlite3",
            "PAPERTRAIL_LISTEN",
            "127.0.0.1:8787",
            "PAPERTRAIL_TOKEN_VALIDATION",
            "PAPERTRAIL_NATIVE_CLIENT_ID",
            "PAPERTRAIL_NATIVE_SCOPES",
            "openid offline_access",
            "PAPERTRAIL_INTROSPECTION_CLIENT_ID",
            "PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE",
            "HTTPS reverse proxy",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }
}

#[test]
fn version_matches_the_shared_release_version_without_configuration() {
    for flag in ["--version", "-V"] {
        let output = command().arg(flag).output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("paper-trail-server {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn informational_flags_ignore_broken_server_configuration_and_logging() {
    for flag in ["--help", "-h", "--version", "-V"] {
        let database = std::env::temp_dir().join(format!(
            "papertrail-cli-{}-{}.sqlite3",
            std::process::id(),
            flag.trim_start_matches('-')
        ));
        assert!(!database.exists());
        let output = command()
            .args(["--verbose", flag])
            .env("PAPERTRAIL_LISTEN", "not-an-address")
            .env("PAPERTRAIL_DATABASE", &database)
            .env("PAPERTRAIL_TOKEN_VALIDATION", "not-a-mode")
            .env("PAPERTRAIL_NATIVE_CLIENT_ID", "")
            .env("RUST_LOG", "[")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.is_empty());
        assert!(!database.exists());
    }
}

#[test]
fn invalid_arguments_exit_with_usage_error_before_startup() {
    for args in [
        vec!["--unknown"],
        vec!["--version=private"],
        vec!["private-positional"],
        vec!["-v", "--unknown"],
        vec!["--help", "--unknown"],
        vec!["--", "--version"],
    ] {
        let output = command().args(args).env("RUST_LOG", "[").output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let text = stderr(&output);
        assert!(text.contains("--help"));
        assert!(!text.contains("Starting encrypted sync server"));
        assert!(!text.contains("private"));
    }
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
