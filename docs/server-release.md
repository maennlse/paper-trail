# Paper Trail sync server binaries

Download `paper-trail-server-<target>.tar.gz` (Linux/macOS) or `.zip` (Windows)
from GitHub Releases, or the **Standalone Server** workflow's artifacts for a
development build. Six targets are available: Linux musl, Windows MSVC, and
macOS, each for x86_64 and ARM64, matching the TUI downloads. Verify the archive's
SHA-256 checksum before extracting it.

The archive contains only `paper-trail-server` (`.exe` on Windows) and this
readme. No configuration, credentials, database, or vault keys are bundled.
Linux binaries are fully static. Windows binaries use the static C runtime and
Windows system DLLs; macOS binaries link only Apple system libraries dynamically.
SQLite and cryptography are built into the executable. No GTK, Python, Rust,
SQLite installation, or third-party runtime library is needed to run it.
Windows/macOS binaries are not signed or notarized.

## Configuration and startup

The server requires an existing OIDC provider and an **HTTPS reverse proxy**.
It listens only on loopback (`127.0.0.1:8787` by default). Never expose its HTTP
listener directly or forward it over an unencrypted network. `--help` prints
configuration guidance without contacting the provider or creating a database.

Example for Linux/macOS (replace all placeholders with your deployment values):

```sh
./paper-trail-server --help
PAPERTRAIL_OIDC_ISSUER=https://YOUR_IDENTITY_PROVIDER \
PAPERTRAIL_API_AUDIENCE=YOUR_API_AUDIENCE \
PAPERTRAIL_NATIVE_CLIENT_ID=YOUR_PUBLIC_NATIVE_CLIENT_ID \
PAPERTRAIL_NATIVE_SCOPES='openid offline_access YOUR_PROVIDER_API_SCOPE' \
PAPERTRAIL_DATABASE=/srv/papertrail/papertrail.sqlite3 \
./paper-trail-server
```

On Windows, use the same environment variables in PowerShell or your service
manager and run `.\paper-trail-server.exe`. For example:

```powershell
$env:PAPERTRAIL_OIDC_ISSUER = 'https://YOUR_IDENTITY_PROVIDER'
$env:PAPERTRAIL_API_AUDIENCE = 'YOUR_API_AUDIENCE'
$env:PAPERTRAIL_DATABASE = 'C:\PaperTrail\papertrail.sqlite3'
.\paper-trail-server.exe
```

Create the database's parent directory beforehand and restrict it to the service
account. The server creates the SQLite database; keep it outside the extracted
binary directory so upgrades do not replace data. Configure a service manager
appropriate to the OS; the executable does not install a Windows service itself.

- `PAPERTRAIL_OIDC_ISSUER` and `PAPERTRAIL_API_AUDIENCE` are required. Audience
  validation is server-controlled; ID tokens are never API credentials.
- Optional `PAPERTRAIL_NATIVE_CLIENT_ID` and `PAPERTRAIL_NATIVE_SCOPES` enable
  public authentication discovery for GTK/TUI clients. Scopes default to
  `openid offline_access`; add the existing provider-specific API scopes. Do not
  reuse confidential introspection credentials for this public native client.
- Default JWT validation requires RFC 9068 access tokens. ZITADEL and providers
  issuing opaque or generic JWT access tokens need explicit
  `PAPERTRAIL_TOKEN_VALIDATION=introspection`,
  `PAPERTRAIL_INTROSPECTION_CLIENT_ID`, and
  `PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE`. Keep the server-only secret in a
  protected file, never a command argument, native client, archive, or log.
- `PAPERTRAIL_LISTEN` changes the loopback listen address.
- Logs go to stderr. `--verbose`/`-v` enables server debug logs; `RUST_LOG`
  overrides filtering. Do not enable HTTP body/header tracing.

For provider registration, HTTPS proxying, backup/restore and security details,
read the [encrypted sync deployment guide](https://github.com/maennlse/paper-trail/blob/main/docs/encrypted-sync.md).
Back up SQLite consistently, including its WAL state; do not simply copy a live
database file. Upgrading binaries does not migrate accounts or vault identities.

## Security boundary

The server stores account-scoped encrypted vault envelopes and note history; it
does not hold clients' vault decryption keys. Authentication and vault unlocking
are separate. The implementation remains experimental pending independent
security review. Use an unprivileged service account, HTTPS, restrictive file
permissions, independent backups, and deployment-specific quotas/rate limits.
These binaries do not bundle an identity provider or reverse proxy.

## Building locally

```sh
cargo build --release --locked --bin paper-trail-server
cargo test --locked -p papertrail-server
```

Standalone CI additionally builds/tests each native target, checks Linux static
linkage or Windows/macOS system-only dependencies, and smoke-tests `--help`.
`python build-aux/package_server.py TARGET` packages an existing release build
at `target/TARGET/release/`. Linux standalone builds require the musl toolchain;
Windows builds require static CRT flags, as configured in the server workflow.
Python is a build helper only, not a server runtime dependency.
