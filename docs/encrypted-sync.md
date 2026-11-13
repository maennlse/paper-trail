# Encrypted synchronization

Paper Trail offers **client-side end-to-end encryption where the server does not
hold the decryption key**. This is not an absolute “zero knowledge” claim. Treat
this new implementation as experimental pending independent security review and
the platform checks listed below. Keep independent backups of your local notes.

## Existing architecture and storage decision

`papertrail-core` owns path-based notes, summaries, settings, and the autosaving
editor session. `papertrail-storage` reads ordinary files (including extensionless
files), normalizes text on read, and supplies atomic writes, cooperative locks,
no-clobber moves, and local content conflict checks. GTK4/libadwaita/GtkSourceView
and Ratatui are separate frontends sharing these crates. There was no server,
network authentication or encryption before this work.

**Local files are not replaced.** UTF-8 text remains the working format, preserving
external-editor compatibility. Invalid UTF-8/newline normalization still follows
the existing repository adapter; sync is for text, not arbitrary binary files.
The encrypted format is a transport/history format, not a conversion of note files.

`papertrail-crypto` defines portable envelopes. `papertrail-sync` supplies HTTPS,
native public-client OAuth, a durable local journal/outbox, and reconciliation.
`papertrail-server` authenticates and stores opaque revisions in SQLite.
Only GTK, TUI and the sync server are currently included. Shared behavior remains
portable without maintaining a separate mobile client or binding generator.

Sync is opt-in for explicitly selected folders. GTK Configure Sync is prefilled and
lists local/known remote folders with selection checkboxes and directory mapping
buttons. Configuration lives in `sync/config.json` beside the chosen settings
file. The local `sync/state.json`
is outside note folders, because existing listing treats every regular file as a
note. It holds opaque-ID/path mappings, plaintext comparison baselines, known
revision digests, pending writes/downloads, and high-water marks. It is private
local data, **not** a server upload or an encrypted export. Do not casually delete
it: that also discards rollback knowledge and file identities.
The local journal is version 3 and configuration version 2. V1/V2 journals upgrade
without resetting IDs/checkpoints. Update both desktop and TUI when sharing a
profile; older clients fail closed instead of dropping folder mappings or deferred
revisions. **Upgrade the server first:** it must accept v2 encrypted revisions.
The existing vault/key envelope and AEAD blob formats stay v1; passphrases, recovery
keys and plaintext working files do not need conversion or replacement.

## User flow

GTK: open the **Sync icon at the bottom-left** → Configure Sync → Sign In → Create Vault or
Unlock Vault. Setup uses native dialogs; creating/changing credentials requires
saving and confirming a recovery key before publishing the new envelopes.
Sync Now, Recover Vault, Change Passphrase, Lock Vault, and Sign Out are in that
dedicated menu. Status appears directly in the menu/tooltip without a duplicate
status dialog. The circular-arrow icon rotates while working, stopping when done
or on failure; disabled system animations are respected. Menus stay accessible.

TUI:

```text
:sync-configure https://sync.example
# Review the displayed sync server and identity provider; type yes to trust it.
:login
# Open the displayed verification URL/code on another device. No SSH browser needed.
:create-vault
# Save the displayed recovery key, then type yes at the confirmation prompt.
:unlock
:sync
:sync-status
:recover-vault
:change-passphrase
:lock
:logout
```

Secrets use masked prompts, never command arguments or command completion/history.
TUI `:login` uses discovered RFC 8628 device authorization. The verification URL
and short-lived **user code** appear only in the current UI, not the high-entropy
device code or tokens. `:login-show` expands the instructions; `:login-cancel` or
Ctrl-C cancels without losing edits. `:login-browser` explicitly selects the local
browser/PKCE alternative. GTK keeps browser/PKCE sign-in. See [SSH setup](tui.md#device-code-sign-in-including-ssh).
New passphrases require at least 16 characters; prefer several randomly chosen
words. GTK requires confirmation of newly entered passphrases. The TUI
currently prompts once, so verify the passphrase carefully before setup.

After each launch the key is locked; unlock triggers sync. Unlocked Linux clients
sync periodically; GTK also debounces saved edits and foreground activation.
GTK keeps menus, navigation and editing enabled while HTTP runs. Short local file
leases serialize note writes and navigation, never network requests; the worker
reads the current protected editor path at application time, not just at job start.
Authenticated remote revisions for that note are durably deferred, while the cursor
and other notes continue syncing. Closing/switching notes lets a subsequent sync
apply those revisions and preserve concurrent saved edits as conflict copies.
Each deferred parent link is authenticated/checked immediately; only the latest
encrypted snapshot per note needs to remain in the local queue. Immutable
intermediate revisions remain on the server. This also lets a note move from an
unselected collection into a selected one without importing its old folder first.
The active buffer, selection and undo history are not replaced on sync completion.
Local renames are journaled without waiting for the network worker. Metadata results
merge into current GTK settings instead of replacing preferences changed during sync.

Background failures update the indicator/status without modal dialogs. GTK retries
with backoff (up to five minutes), and foreground/menu focus changes cannot start a
repeated immediate-sync loop. User-requested account/key operations still use native
dialogs. Lock/sign-out requested during a job are queued until it finishes, stop
automatic scheduling immediately, and report that pending state. TUI editing is
still paused during its own sync job; device-code authorization does not pause
editing/autosave. Its protected remote revisions also defer.
Local saves continue without network access.

Application renames retain opaque note identity via local mapping updates. External
renames cannot be identified reliably without filesystem metadata: they may appear
as a deletion plus creation. Removing a folder bookmark is not a remote deletion;
an unavailable selected folder stops sync rather than uploading deletions. Symlinks
are not imported by the sync scanner. Initial import of an existing second working
tree can produce duplicates; use an empty folder on a new device when possible.

## Multiple folders and device-local mappings

All selected folders share one vault/key, but each collection has a random stable
folder ID. A v2 `folder_catalog` record, stored at the reserved opaque vault ID,
encrypts the ID-to-name/color table and confirmed legacy note membership. Catalog
revisions are immutable and conditionally appended just like note revisions. A
single conditional catalog prevents two first-upgrade devices from independently
publishing incompatible legacy folder identities. The server never reads the table
or learns folder membership from API paths.
It can recognize the catalog's reserved opaque ID and infer metadata-update activity
from its size/timing; this does not hide all usage information.

Configure Sync lists checked local folders and known remote folders. Pick a local
directory for a remote ID, or leave its automatic mapping in place. Newly discovered
remote collections get separate directories under **New remote folders are
downloaded into** (by default a sibling `Paper Trail Synced` directory). An existing
unmanaged directory is never silently adopted/merged: select it explicitly. Duplicate
logical names get distinct managed directories/IDs. Notes never fall back into the
first selected folder. On a new download-only device, uncheck the empty default
folder during setup to avoid registering it as an additional empty collection.
Turn off **Download folders created on other devices** before unlocking when you
want to select individual remote IDs first rather than import all collections.

Folder scans remain non-recursive, matching the existing application model. Select
subdirectories separately when needed; this is a list of collections, not a mirrored
arbitrary directory tree. Empty folders exist in the encrypted catalog and therefore
sync even with no notes. Names/colors are logical metadata; each device can map them
to a differently named physical directory, and remote renames do not move arbitrary
user directories. Folder labels in GTK/TUI show the synced logical name.

Unchecking a folder stops its uploads and local application of downloads without
deleting notes, changing its ID, or dropping pending revisions. Its encrypted changes
still pass through the account cursor (membership is encrypted), and remain deferred
until re-enabled. Switching an existing mapping copies the current working notes to
the new location without removing old files. Close notes in that collection before
remapping. Missing/unavailable directories stop sync rather than becoming mass
deletions. Global folder deletion is not exposed; unchecking is always device-local.

Legacy migration preserves note IDs, vault keys, history, cursors, and files. Known
note IDs are assigned using their existing local paths, not ambiguous names. Unknown
legacy assignments stop safely and appear as confirmation dropdowns in Configure
Sync. Explicit confirmation publishes the encrypted assignment for subsequent devices.
Name/color conflicts keep local directories intact and require acknowledging remote
metadata in Configure Sync; rename again afterward if a different global name is wanted.

TUI: `:sync-folders` opens an interactive checklist for local and known remote
collections. Space/click toggles, Enter saves, and Esc cancels; `i` shows full
folder names, local paths and opaque IDs. Selection changes never delete notes.
Existing path/ID commands remain compatibility shortcuts:

```text
:sync-folders
:sync-download-dir /absolute/download/root
:sync-auto-download off
:sync-select-folder FOLDER_ID
:sync-unselect-folder FOLDER_ID
:sync-map-folder FOLDER_ID '/absolute/local/directory'
:sync-map-legacy NOTE_ID FOLDER_ID
:sync-accept-remote FOLDER_ID
```

## Authentication, HTTPS, and secure storage

Account login and vault unlock are separate. New GTK setup discovers authentication
from the sync server by default; TUI uses `:sync-configure HTTPS_SERVER`. Clients
fetch the public `GET /v1/auth/config`, validate its settings, then fetch the issuer's
standard OIDC discovery metadata and require an **exact** issuer match. The public
response contains only:

```json
{
  "issuer": "https://login.example",
  "client_id": "YOUR_PUBLIC_NATIVE_CLIENT_ID",
  "scope": "openid offline_access YOUR_PROVIDER_API_SCOPE"
}
```

The examples are placeholders, not built-in provider settings. Configuration and
metadata fetches use HTTPS, fixed timeouts, size limits and no redirects. Invalid,
incomplete, unavailable or mismatched discovery fails closed without using tokens
or silently falling back to manual/insecure settings. The endpoint is public and
read-only; missing server native-client settings produce an empty HTTP 503. Older
servers can return 404, with instructions to configure authentication manually.

### Configuration ownership and precedence

| Setting                        | Source                                                                                                                                  |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| Advertised issuer              | Existing `PAPERTRAIL_OIDC_ISSUER`, read from the server's token authenticator                                                           |
| Expected access-token audience | Existing `PAPERTRAIL_API_AUDIENCE`; remains independent server-side validation                                                          |
| Advertised public client ID    | Optional new `PAPERTRAIL_NATIVE_CLIENT_ID`, copied from the existing provider native-app registration                                   |
| Advertised scopes              | Optional new `PAPERTRAIL_NATIVE_SCOPES`, default `openid offline_access`; include the existing provider-specific API scopes when needed |

Previously, native IDs/scopes existed only in client `sync/config.json`, not in the
server's deployment configuration. These two optional native settings make that
public configuration available server-side. No issuer/audience setting was added.
The confidential `PAPERTRAIL_INTROSPECTION_CLIENT_ID` and secret file are unrelated
server credentials and are **never** advertised or used as native defaults.

The expected audience is normally **not advertised**: it is a resource-server
validation rule, not a universal OAuth request parameter. Existing flows select the
API through the provider's native registration/scopes; discovery adds no `audience`
or `resource` parameter. Advertised/requested scopes do not grant server permissions
and do not replace signature, issuer, audience, time or account-authorization checks.

- Existing client configurations remain **manual**; missing `discover_auth` means
  `false`. Explicit issuer/client/scopes continue taking precedence, with no fetch
  from the new endpoint. Existing servers need no configuration changes to keep
  these clients working.
- Select **Discover authentication from sync server** in GTK or run the one-argument
  TUI setup command to explicitly opt in. Discovery uses the advertised native ID
  and scopes as a complete set, not a silent mix of manual and discovered fields.
- For manual/older-server setup, turn that switch off or use
  `:sync-configure HTTPS_SERVER HTTPS_ISSUER PUBLIC_CLIENT_ID 'SCOPES'`. The optional
  scopes argument retains existing scopes, or defaults to `openid offline_access`.
- Discovery failures never overwrite a saved manual configuration. Discovery mode
  requires the configuration endpoint to remain available; there is no automatic
  downgrade when it fails. You may explicitly select manual configuration.

### Issuer trust and account identity

Treat the sync server's advertised identity provider as untrusted. Before first
authentication, GTK/TUI display both URLs and require explicit confirmation. TUI
requires `yes`; Esc or any other answer rejects. Discovery can be cancelled without
saving configuration (TUI: `:sync-discovery-cancel`). The accepted server/issuer pair
is persisted in the profile's `sync/config.json`; `issuer` is the trust pin, not a
value that runtime discovery may overwrite. A previously explicit manual issuer
for the same server also represents an existing trust decision.

The same accepted issuer continues automatically. Discovery mode rechecks advertised
settings before authentication, refresh and authenticated sync operations. An issuer
change stops those operations with instructions to review Configure Sync (TUI:
`:sync-configure SERVER`); no new issuer is accepted without confirmation. Native
client ID/scope changes also require explicitly reviewing configuration. Non-interactive
shared-client calls reject changes rather than prompting or accepting them.

**Confirmation never migrates an identity.** Existing journal/account protection
still refuses changing server, issuer or native client ID once a journal exists;
use a separate settings profile and note directory for a changed identity. No
accounts are merged, journals reset, vaults adopted, encryption keys changed, or
encrypted formats rewritten. Identity remains the server-validated `(iss, sub)`.
Switching an unbound profile clears its old in-memory credentials/keys; credential
storage remains namespaced by server, issuer and native client ID. Older clients
that do not understand the discovery flag reject that configuration rather than
silently dropping its trust policy; update both GTK and TUI for a shared profile.

### Existing OAuth flows

Configure the provider's public native client, registered redirects, and scopes
granting the API audience. There is no hardcoded Zitadel dependency. The issuer discovery
document supplies authorization/device-authorization/token/JWKS endpoints; insecure
endpoints and issuer mismatches fail. GTK and optional TUI browser login use
Authorization Code with PKCE S256 through the system browser. TUI device login uses
RFC 8628 through the maintained `oauth2` library, honoring pending/slow-down errors,
polling intervals, network backoff, denial, and expiry. Native apps have **no embedded
client secret**. For browser login, state is verified; an
authorization-response issuer, when supplied, must match. ID tokens are not used
as credentials or decoded as the source of account identity: the authenticated
API returns the access token's validated `(iss, sub)` identity.

Linux browser login registers `http://127.0.0.1:{ephemeral-port}/callback` per RFC 8252.
The callback page tries to close its tab after three seconds once the authorization-code
exchange succeeds. Browser security may block closing a system-browser tab; a
manual-close message remains visible. Error pages do not auto-close, and no tokens
or provider error details are included in the page. The successful page attempts
to remove OAuth query parameters from its address bar before the timer starts.
This loopback redirect is the narrow HTTP exception; all issuer and API requests require
HTTPS and do not follow redirects. API audience selection is issuer-specific;
configure it with the provider's client/scopes rather than assuming a universal
nonstandard `audience` authorization parameter.

**Default token validation (`PAPERTRAIL_TOKEN_VALIDATION=jwt`):** signed RFC 9068 JWT access tokens, `typ=at+jwt` or
`application/at+jwt`, RS256 or ES256, with `kid`, exact issuer, nonempty subject,
required expiry, intended API audience, and valid not-before if present. Validation
allows 30 seconds of clock skew. Generic `typ=JWT` ID tokens are rejected, even if
someone mistakenly gives one the API audience. This mode does not accept opaque
tokens or ZITADEL's generic `typ=JWT` access tokens. JWKS is cached for five minutes; unknown-key refresh attempts are
rate-limited to 30 seconds. New keys may therefore take that long to become usable.

**Introspection (`PAPERTRAIL_TOKEN_VALIDATION=introspection`):** use this mode with
ZITADEL or an issuer that does not issue RFC 9068 access tokens. The server discovers
the HTTPS RFC 7662 endpoint and authenticates with `client_secret_basic`. Every
request is introspected; there is no positive-response cache or automatic fallback
from JWT mode. Both JWT-formatted and opaque **access tokens** are supported.
Introspection must confirm `active=true`, `token_type=Bearer`, the exact configured
issuer, a nonempty subject, the intended API audience, required unexpired `exp`,
and a valid optional `nbf` (30 seconds of not-before clock skew). Missing claims
fail closed. The trusted issuer verifies the token/signature and rejects ID tokens;
`token_type_hint=access_token` alone is not treated as proof of token type.
Refresh tokens and inactive/revoked tokens are not API credentials. Rejected user
tokens produce HTTP 401; provider outages, malformed responses, and server-client
authentication failures produce HTTP 503 without exposing response bodies.

Only the **server** holds the introspection API client secret. This secret cannot
unwrap vault keys and is never distributed to native apps. Store it in a protected
single-line file and set `PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE`; do not put it
in shell command arguments, native-client configuration, Git, or logs. Set its
client ID with `PAPERTRAIL_INTROSPECTION_CLIENT_ID`. Introspection mode requires
provider availability and adds an HTTPS round trip per API request. Private-key JWT
authentication for introspection is not yet implemented.

Refresh/access tokens are saved only in Linux Secret Service, Apple Keychain, or
Windows Credential Manager through `keyring`. If secure storage is unavailable,
Linux login is session-only; no plaintext credential fallback exists. Refresh-token
rotation is supported.
SSH/headless TUI sessions intentionally bypass the desktop credential store, keeping
access/refresh tokens in memory (including refresh); sign-out clears that session
without deleting other graphical sessions' stored credentials. Device requests are
limited to a one-hour lifetime; verification URLs must use HTTPS. Secrets/provider
response bodies are never echoed in diagnostics. The unlocked vault key is memory-only
on all clients: no persistent key cache is implemented. Lock drops keys and pending
wrappers; sign-out also removes stored tokens when using persistent storage.
Neither action erases local plaintext notes, local
comparison baselines, swap, terminal history maintained by the terminal itself,
or other programs' copies. Sign-out is local credential removal, not provider-wide
session logout or OAuth token revocation.

Flatpak now requests network access and access to `org.freedesktop.secrets`.
Browser/loopback and secret-service portal behavior require manual Flatpak checks.

## Versioned crypto/key format

Use established RustCrypto `chacha20poly1305`, `argon2`, OS-backed `rand_core`, and
`zeroize`; do not implement cryptographic primitives. GTK and TUI use the same
Rust implementation. Tests include an
independently generated libsodium XChaCha20-Poly1305 combined-mode vector and the
RFC 9106 Argon2id vector.

V1 has a random 32-byte vault key and random 16-byte opaque note/revision/vault IDs.
Every encrypted object uses XChaCha20-Poly1305 and a fresh random 24-byte nonce.
The tag is appended to the ciphertext. Binary fields use unpadded base64url.
Maximum plaintext revision size is 16 MiB. No compression or padding is performed.

Revision JSON contains `version`, `vault_id`, `note_id`, `id`, `parent`, `deleted`,
and `blob: { version, nonce, ciphertext }`. Authenticated associated data is the
UTF-8 compact JSON serialization of the fixed-order tuple:

```text
["papertrail/note",1,{"iss":"exact issuer","sub":"subject"},
 "vault-id","note-id","revision-id","parent-id-or-null",false]
```

The parent is a JSON string or actual `null`, not the illustrative text above.
`deleted` is a JSON Boolean. V1 plaintext is a `NoteData` object or JSON `null` for
a tombstone. V2 uses the same AAD tuple with version `2`, with a tagged encrypted
record: `note`, `folder_catalog`, or `deleted_note`. Note records carry body,
filename, modification milliseconds, pin, language and a stable `folder_id`.
Titles/previews/search are derived locally. Absolute paths never go to the server.
Folder colors/names sync encrypted; folder order and window/font preferences remain
device-local. This remains text-note synchronization, not arbitrary filesystem replication.

The vault envelope contains format version, vault ID, monotonic generation, KDF
parameters, and independent passphrase/recovery blobs wrapping the same random
vault key. Argon2id v1.3 derives **only** the wrapping key: initially 64 MiB, three
passes, four lanes, random 16-byte per-vault salt, 32-byte output. Parameters are
stored, versioned, authenticated, and bounded before allocation (64–256 MiB,
3–10 passes, 1–4 lanes). Passphrases are UTF-8 without trimming or normalization.
Benchmark these defaults on the slowest supported client before production release.

Wrapper AAD is compact JSON:
`["papertrail/key",1,account,vault_id,generation,purpose,kdf]`, where purpose is
`passphrase` or `recovery`. Account fields serialize in `iss,sub` order; KDF fields
in `version,algorithm,memory_kib,passes,lanes,salt` order. A wrapping plaintext is
exactly the 32-byte vault key. The independent random 32-byte recovery secret is
shown as unpadded base64url. It is never uploaded. Unknown versions fail closed.

Changing the passphrase rewraps without re-encrypting notes. Since both wrappers
bind their generation, this implementation also rotates the recovery secret and
requires saving the replacement. Losing all unlock methods means permanent loss.
Old envelopes in backups remain usable with old credentials; changing a passphrase
does not revoke a previously stolen vault key. Key compromise needs a future full
key-rotation/re-encryption workflow, not just passphrase change.

## API and consistency

Every route except login discovery requires `Authorization: Bearer ACCESS_TOKEN`.
Every database query/mutation is scoped to validated `(iss, sub)`, not email or a
caller-selected account. IDs do not contain titles or filenames.

| Route                                     | Behavior                                                           |
| ----------------------------------------- | ------------------------------------------------------------------ |
| `GET /v1/auth/config`                     | Public native authentication settings; no Bearer token required    |
| `GET /v1/identity`                        | Validated account identity                                         |
| `GET /v1/vault`                           | Current encrypted wrapping envelopes                               |
| `PUT /v1/vault`                           | `If-Match: 0` creates; current generation conditionally updates    |
| `GET /v1/changes?cursor=N[&through=T]`    | Ordered immutable revisions, stable page boundary                  |
| `POST /v1/notes/{id}/revisions`           | Conditional append using authenticated parent revision             |
| `GET /v1/notes/{id}/revisions/{revision}` | Read immutable encrypted historical revision                       |
| `GET /v1/export`                          | Account ciphertext/envelopes/history/metadata export               |
| `DELETE /v1/account`                      | Explicit deletion; requires `X-Papertrail-Confirm: delete-account` |

Revision IDs are idempotency identifiers: retrying identical bytes succeeds, reuse
with different bytes conflicts. Stale parents return HTTP 409. Appends and change
sequences commit transactionally. Successful appends/retries return the persisted
sequence in `X-Papertrail-Sequence`; clients retain that acknowledgement as a
rollback checkpoint even before the next pull. Tombstones are authenticated encrypted revisions
with an observable deletion marker, not physical note-row removal. No background
revision/tombstone pruning exists. A conflict preserves the pending body as a new
conflict-copy ID and keeps the remote version at the original ID. Edit/delete
conflicts do not resurrect the deleted ID. No server-side ciphertext merge occurs.

Local high-water marks (including envelope generation) and revision digests detect
backward checkpoints, modified known revisions, omitted cursor entries, and
non-descendant revisions. The client checks its cursor before making remote writes.
AEAD alone does not stop replay. These checks are **not** a transparency log:
malicious withholding, some forked histories, local-state rollback, and permanent
denial of service remain possible. A new device without checkpoints initially
trusts the server's supplied history. Existing state is never silently reset.

## Run and operate the server

Prebuilt server binaries for Linux, Windows, and macOS (x86_64 and ARM64) are
available from GitHub Releases and **Standalone Server** CI artifacts. See the
[server binary deployment guide](server-release.md) for downloads, runtime
requirements and setup. To build from source:

```sh
cargo build --release --locked --bin paper-trail-server
PAPERTRAIL_OIDC_ISSUER=https://login.example \
PAPERTRAIL_API_AUDIENCE=papertrail-api \
PAPERTRAIL_NATIVE_CLIENT_ID=YOUR_PUBLIC_NATIVE_CLIENT_ID \
PAPERTRAIL_NATIVE_SCOPES='openid offline_access YOUR_PROVIDER_API_SCOPE' \
PAPERTRAIL_DATABASE=/srv/papertrail/papertrail.sqlite3 \
./target/release/paper-trail-server
```

### ZITADEL setup

1. Keep the desktop applications **Native**, using Authorization Code + PKCE,
   no client secret, and their existing registered callbacks.
   For the TUI, enable **Device Authorization** and **Refresh Token** on that same
   public/native client. Do not enable device login on the separate server API
   client or copy its secret to the TUI. Existing API-audience scopes are unchanged.
2. In the **same ZITADEL project**, add a separate **API** application using
   **Basic** authentication. Save its generated client ID and secret on the server
   only. Do not reuse the native client ID for introspection authentication.
3. Put the API secret in a readable-only-by-the-service file, for example
   `/etc/papertrail/introspection-secret` (mode `0600`, or a systemd credential).
   A final newline is allowed; the secret itself must be one nonempty line.
4. Set the following environment in the server service and restart the **server**:

```sh
PAPERTRAIL_OIDC_ISSUER=https://YOUR-INSTANCE.zitadel.cloud
PAPERTRAIL_API_AUDIENCE=YOUR_PROJECT_ID
PAPERTRAIL_TOKEN_VALIDATION=introspection
PAPERTRAIL_INTROSPECTION_CLIENT_ID=YOUR_SERVER_API_CLIENT_ID
PAPERTRAIL_INTROSPECTION_CLIENT_SECRET_FILE=/etc/papertrail/introspection-secret
PAPERTRAIL_NATIVE_CLIENT_ID=YOUR_EXISTING_PUBLIC_NATIVE_CLIENT_ID
PAPERTRAIL_NATIVE_SCOPES='openid offline_access urn:zitadel:iam:org:project:id:YOUR_PROJECT_ID:aud'
```

The native client's scopes should be
`openid offline_access urn:zitadel:iam:org:project:id:YOUR_PROJECT_ID:aud`.
Replace the example `papertrail` scope; it is not automatically defined by ZITADEL.
The expected audience is the **project ID**, not either application's client ID.
No changes to native binaries are required for this server-side validation mode.
Do not enable generic `JWT` acceptance in offline JWT mode as a shortcut: that
would remove its protection against confusing ID tokens with access tokens.

### HTTPS and operations

The server logs to **stderr**, captured by systemd/Docker by default. Normal logs
include startup, configured issuer/audience and token-validation mode, completed
requests (method, route template, status, elapsed milliseconds, request ID), and
authentication/operational failures. Run `paper-trail-server --verbose` (or `-v`)
for server debug events, including introspection and signing-key refresh activity.
`RUST_LOG` overrides the filter; use `RUST_LOG=warn,papertrail_server=debug` for
targeted diagnostics instead of enabling verbose HTTP/dependency tracing globally.
Logs are plain text without ANSI escapes. Configure retention/rotation in the
service manager; the server does not create its own log files. Ctrl-C and Unix
SIGTERM log shutdown and drain active connections.

Each response carries a server-generated `X-Request-ID`, also recorded in its log
span. IDs are process-local and reset after restart. Incoming request IDs are not
trusted. Logs use route templates rather than raw URLs: queries, note/revision IDs,
request/response bodies, authorization headers, tokens, account subjects, and raw
provider responses are never logged by server diagnostics, even in verbose mode.

For a TUI `401`, reproduce once and inspect the corresponding server request:

- `missing_or_malformed_bearer`: no usable Bearer header reached the server; check
  client/proxy configuration without printing the credential.
- `unsupported_jwt_type_use_introspection_for_zitadel`: the running server is in
  strict JWT mode; configure ZITADEL introspection as described above and restart.
- `inactive_token`: the provider did not consider the access token active.
- `audience_missing_or_mismatch`: verify the project-ID audience and native client's
  `urn:zitadel:iam:org:project:id:PROJECT_ID:aud` scope, then sign in again.
- `issuer_missing_or_mismatch`, `expired_or_missing_expiry`, or
  `missing_or_non_bearer_token_type`: the introspected response failed the named
  validation check. Validation remains strict; do not bypass it to silence errors.
- `Introspection HTTP failure` with upstream `401`/`403`: check the separate
  server API application's Basic credentials. Provider HTTP/network/format failures
  return **503** to the client, not an end-user-token `401`.

If the client reports `401` but no request appears in the server logs (with info
logging enabled), check the reverse proxy and whether the client reaches the
intended server instance. Do not share tokens or secrets while troubleshooting.

`PAPERTRAIL_LISTEN` defaults to `127.0.0.1:8787`; only loopback listeners are accepted.
Run behind an HTTPS reverse proxy (for example Caddy with automatic certificates).
Do not expose plaintext HTTP via a tunnel or an unencrypted proxy. HTTPS termination
is intentionally external; the Rust binary does not manage TLS certificates.

Use a dedicated unprivileged service account and restrictive permissions on the
database directory. Do not enable HTTP body/header tracing in proxies or clients.
Never log access/refresh tokens, passphrases, recovery/vault keys, plaintext notes,
or note request/response bodies (including ciphertext). Apply deployment-specific
rate limits, disk quotas, request timeouts, and monitoring that uses status/counts
only. The built-in request limit is 32 MiB; change pages are also size bounded.
The export endpoint currently materializes the account history in memory; large
deployments need streaming export and quotas before exposing unbounded storage.

Back up the SQLite database consistently with SQLite's backup API or `.backup`,
not by copying only the live database file while WAL writes are active. This backs
up ciphertext, envelopes, and sync metadata together. No client keys belong in a
server backup. Test restoration into an isolated environment first.

Export and account deletion are currently authenticated API operations, not app UI
menus. Account deletion removes live account data; it does not erase copies held
by other devices, issued OIDC tokens, SQLite free pages, or previous backups.
Operators must publish a backup retention/expiry policy. Do not silently expire
old revisions/tombstones. Keep an independent administrative deletion record and
reapply deletions before reopening any restored backup. The software cannot promise
physical erasure from snapshots or storage media. Encrypted export does not itself
provide an offline-import UI; keep its envelopes and your unlock credentials.

## Remaining platform checks

Real-provider login, Secret Service, Flatpak browser behavior, and non-Linux TUI
secure storage need platform/deployment testing. Independent security review is
still required before treating this experimental implementation as production-audited.

References: [RFC 9106](https://www.rfc-editor.org/rfc/rfc9106.html),
[libsodium AEAD guidance](https://doc.libsodium.org/secret-key_cryptography/aead/chacha20-poly1305/xchacha20-poly1305_construction),
[native OAuth, RFC 8252](https://www.rfc-editor.org/rfc/rfc8252),
[JWT access tokens, RFC 9068](https://www.rfc-editor.org/rfc/rfc9068).
