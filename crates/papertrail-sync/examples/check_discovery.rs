// SPDX-License-Identifier: GPL-3.0-or-later
//! Check unauthenticated discovery with the application's HTTP client.
//! No configuration, credentials or issuer trust decisions are saved.
fn main() -> std::io::Result<()> {
    let server = std::env::args()
        .nth(1)
        .ok_or_else(|| std::io::Error::other("Usage: check_discovery https://sync.example"))?;
    papertrail_sync::auth::config::prepare(papertrail_sync::Config {
        server,
        download_dir: Some(std::env::temp_dir()),
        ..Default::default()
    })?;
    println!("Sync authentication and OIDC discovery succeeded (no configuration saved).");
    Ok(())
}
