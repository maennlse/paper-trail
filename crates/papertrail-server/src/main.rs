// SPDX-License-Identifier: GPL-3.0-or-later
use papertrail_server::{
    auth::{Authenticator, TokenValidation},
    router, Server,
};
use std::{net::SocketAddr, path::Path, sync::Arc};
use zeroize::Zeroizing;

fn token_validation() -> Result<TokenValidation, Box<dyn std::error::Error>> {
    match std::env::var("PAPERTRAIL_TOKEN_VALIDATION").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("jwt") => Ok(TokenValidation::Jwt),
        Ok("introspection") => {
            use std::io::Read;
            let client_id = std::env::var("PAPERTRAIL_INTROSPECTION_CLIENT_ID")
                .map_err(|_| "PAPERTRAIL_INTROSPECTION_CLIENT_ID is required for introspection")?;
            let path =
                std::env::var("PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE").map_err(|_| {
                    "PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE is required for introspection"
                })?;
            let mut contents = Zeroizing::new(String::new());
            std::fs::File::open(path)?
                .take(8193)
                .read_to_string(&mut contents)?;
            let secret = contents.trim_end_matches(['\r', '\n']);
            if secret.is_empty() || contents.len() > 8192 || secret.contains(['\r', '\n']) {
                return Err("Introspection secret file must contain a single nonempty line (at most 8192 bytes)".into());
            }
            Ok(TokenValidation::Introspection {
                client_id,
                client_secret: Zeroizing::new(secret.into()),
            })
        }
        _ => Err("PAPERTRAIL_TOKEN_VALIDATION must be jwt or introspection".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--help") {
        println!("paper-trail-server [--verbose|-v]\nRequired: PAPERTRAIL_OIDC_ISSUER, PAPERTRAIL_API_AUDIENCE\nOptional: PAPERTRAIL_DATABASE (papertrail.sqlite3), PAPERTRAIL_LISTEN (127.0.0.1:8787)\nPAPERTRAIL_TOKEN_VALIDATION: jwt (default, RFC 9068 at+jwt) or introspection (RFC 7662; ZITADEL-compatible).\nIntrospection requires PAPERTRAIL_INTROSPECTION_CLIENT_ID and PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE (server-only API credentials).\nLogging: stderr, info by default; --verbose/-v enables server debug logs. RUST_LOG overrides the filter (e.g. warn,papertrail_server=debug).\nRequires an HTTPS reverse proxy. Only loopback listen addresses are accepted.\nNever use an ID token as an API credential.");
        return Ok(());
    }
    let verbose = std::env::args()
        .skip(1)
        .any(|a| a == "--verbose" || a == "-v");
    papertrail_server::logging::init(verbose)?;
    if std::env::args()
        .skip(1)
        .any(|a| a != "--verbose" && a != "-v")
    {
        return Err("Unknown argument; use --help".into());
    }
    tracing::info!(
        target: "papertrail_server",
        version = env!("CARGO_PKG_VERSION"),
        "Starting encrypted sync server"
    );
    let result = run().await;
    if let Err(error) = &result {
        tracing::error!(target: "papertrail_server", %error, "Server failed");
    }
    result
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let issuer = std::env::var("PAPERTRAIL_OIDC_ISSUER")
        .map_err(|_| "PAPERTRAIL_OIDC_ISSUER is required")?;
    let audience = std::env::var("PAPERTRAIL_API_AUDIENCE")
        .map_err(|_| "PAPERTRAIL_API_AUDIENCE is required")?;
    let address: SocketAddr = std::env::var("PAPERTRAIL_LISTEN")
        .unwrap_or("127.0.0.1:8787".into())
        .parse()?;
    if !address.ip().is_loopback() {
        return Err("Use a loopback listener behind an HTTPS reverse proxy".into());
    }
    let auth = Authenticator::discover_with_mode(issuer, audience, token_validation()?).await?;
    let path = std::env::var("PAPERTRAIL_DATABASE").unwrap_or("papertrail.sqlite3".into());
    tracing::debug!(target: "papertrail_server", "Opening database");
    let server = Arc::new(Server::open(Path::new(&path), Some(auth))?);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(target: "papertrail_server", %address, "Encrypted sync listening; HTTPS reverse proxy required");
    axum::serve(listener, router(server))
        .with_graceful_shutdown(shutdown())
        .await?;
    tracing::info!(target: "papertrail_server", "Server stopped");
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if result.is_err() {tracing::error!(target: "papertrail_server", "Interrupt signal listener failed");}
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                tracing::error!(target: "papertrail_server", "Could not register termination signal listener");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!(target: "papertrail_server", "Interrupt signal listener failed");
    }
    tracing::info!(target: "papertrail_server", "Shutdown requested; draining connections");
}
