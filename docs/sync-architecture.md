# Encrypted sync architecture at a glance

- **Existing flow:** GTK and Ratatui share the Rust core and filesystem adapter.
  Notes are plain text; titles/previews derive from content and filenames. Settings
  are path-keyed JSON. Atomic saves, local content conflicts, external editors,
  and local-only operation remain intact. Scope is GTK, TUI, and the sync server.
- **Storage adaptation:** keep those files. Add a private local sync journal with
  random note-ID/path mappings, durable pending work, baselines and checkpoints.
  The server's SQLite database stores ciphertext and sync metadata, not a new
  authoritative local note database. Local files/baselines remain plaintext.
- **Multiple folders:** a conditional encrypted catalog at the opaque vault ID
  assigns random folder IDs and encrypts names, colors and legacy-note membership.
  V2 note payloads reference those IDs, never directory names. Local bindings map
  IDs to selected directories; unmatched folders get separate app-managed directories
  or require an explicit mapping on collision. Unchecking never deletes notes.
  Empty folders sync through the catalog. Physical directory names may differ by
  device; logical names/colors appear in the clients' existing folder lists.
- **Crypto:** shared RustCrypto XChaCha20-Poly1305 with fresh random nonces and a
  random 256-bit vault key. Versioned envelopes authenticate account, vault, note,
  revision, parent, and operation. Filenames, folder names, note metadata and body
  are encrypted. Argon2id wraps the vault key using a separate vault passphrase;
  an independent user-held recovery secret supplies a second wrapper.
- **Authentication:** discover a configured exact OIDC issuer. Native public
  GTK clients use system-browser Authorization Code + PKCE. TUI `:login` uses
  discovered RFC 8628 device authorization for SSH/headless hosts; `:login-browser`
  is an explicit alternative. Neither native flow embeds a client secret.
  API credentials are access tokens, not ID tokens. V1 accepts RFC 9068 JWT access
  tokens through discovered JWKS, with signature/issuer/expiry/audience validation.
  Explicit RFC 7662 introspection mode supports ZITADEL JWT and opaque access
  tokens using separate server-only API credentials, validating active status,
  Bearer type, issuer, expiry and audience. Every account is `(iss, sub)`.
- **API:** identity; get/conditionally replace vault envelopes; changes since a
  cursor; immutable revision fetch; conditional revision append; encrypted export;
  explicitly confirmed account deletion. All data routes are account-scoped.
- **Conflicts/deletions:** stale-parent writes conflict. Preserve local edits as a
  new conflict-copy ID; retain the remote version at its original ID. Authenticated
  tombstones stop resurrection. The server does not merge ciphertext. No automatic
  history/tombstone expiry is defined or implemented.
- **Rollback:** retain acknowledged upload sequences, cursor high-water marks,
  envelope generation, and known revision digests/sequences. Reject detected
  backwards/replayed/non-descendant history. This cannot guarantee detection of
  all withholding/forking. A new device initially trusts the supplied history.
- **Platforms:** GTK has a bottom-left Sync icon/menu, live editing during network
  work, a quiet status indicator, and durable deferral of active-note updates.
  TUI has equivalent commands and masked
  secret prompts. Tokens use platform secure storage, keys are memory-only.
  Sync follows unlock/foreground/manual requests. Configuration is prefilled and
  the sync arrow rotates during work when system animations are enabled.
  Real-provider/browser and Flatpak integration checks remain manual.
  Device login shows only the verification URL/user code, can be cancelled without
  losing edits, and never stores tokens on SSH/headless sessions (even on refresh).
- **Exposure and recovery:** the server sees account identity, opaque IDs, sizes,
  revision links, deletions and activity. Offline passphrase guessing, compromised
  endpoints, plaintext local files and denial of service remain risks. Recovery
  requires the passphrase, saved recovery key, or an unlocked device. Passphrase
  changes rewrap keys and rotate the recovery secret, not all note revisions.
  Old backups/previously disclosed keys are not revoked by that operation.

See [format, setup, API and operations](encrypted-sync.md) for exact encodings,
server deployment, backup/account-deletion retention, and remaining limitations.
