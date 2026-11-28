// SPDX-License-Identifier: GPL-3.0-or-later
use std::ffi::OsString;

pub const HELP: &str = "\
End-to-end encrypted Paper Trail sync server.

Usage: paper-trail-server [OPTIONS]

Options:
  -h, --help       Print help and exit
  -V, --version    Print the release version and exit
  -v, --verbose    Enable server debug logs on stderr

Required environment variables:
  PAPERTRAIL_OIDC_ISSUER       Exact HTTPS OpenID Connect issuer URL
  PAPERTRAIL_API_AUDIENCE      Expected API access-token audience

Optional environment variables:
  PAPERTRAIL_DATABASE          SQLite database path [default: papertrail.sqlite3]
  PAPERTRAIL_LISTEN            Loopback listen address [default: 127.0.0.1:8787]
  PAPERTRAIL_TOKEN_VALIDATION  jwt or introspection [default: jwt]
  RUST_LOG                    Override the logging filter [default: info]
                              Example: warn,papertrail_server=debug

Native client authentication discovery:
  PAPERTRAIL_NATIVE_CLIENT_ID  Public native client ID, not the introspection client
  PAPERTRAIL_NATIVE_SCOPES     Native OIDC/API scopes [default: openid offline_access]
                              Add provider-specific API scopes when needed.
  GET /v1/auth/config returns 503 until a native client ID is configured.
  The issuer comes from PAPERTRAIL_OIDC_ISSUER; the API audience stays server-only.

Introspection mode requires:
  PAPERTRAIL_INTROSPECTION_CLIENT_ID
  PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE
  These are server-only API credentials. The secret file must contain one nonempty
  line (at most 8192 bytes). JWT mode uses RFC 9068 at+jwt access tokens;
  introspection uses RFC 7662 (ZITADEL-compatible).

Deployment:
  Requires an HTTPS reverse proxy. Only loopback listen addresses are accepted.
  Never use an ID token as an API credential. Logs go to stderr; RUST_LOG overrides
  --verbose/-v. Help and version do not require configuration or start the server.";

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    Serve { verbose: bool },
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, &'static str> {
    let mut help = false;
    let mut version = false;
    let mut verbose = false;
    let mut end_options = false;
    for arg in args {
        if end_options {
            return Err("Positional arguments are not supported; use --help");
        }
        match arg.to_str() {
            Some("-h" | "--help") => help = true,
            Some("-V" | "--version") => version = true,
            Some("-v" | "--verbose") => verbose = true,
            Some("--") => end_options = true,
            _ => return Err("Unknown argument; use --help"),
        }
    }
    Ok(if help {
        Action::Help
    } else if version {
        Action::Version
    } else {
        Action::Serve { verbose }
    })
}
