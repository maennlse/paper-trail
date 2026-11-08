# Paper Trail 0.4.0

## Highlights

- Optional client-side end-to-end encrypted sync for GTK and TUI, backed by a
  separately deployed OIDC-authenticated Rust server. Ordinary local text files
  remain unchanged; local-only use still needs no account.
- Independent vault passphrase and recovery key, encrypted folder metadata,
  per-device folder mappings, selective sync, conflict copies, tombstones, and
  rollback checkpoints.
- Non-blocking desktop sync with a bottom-left status/menu icon; editing stays
  available during network operations.
- TUI device-code authentication for SSH, cancellable login, and memory-only
  credentials on SSH/headless hosts. Desktop browser/PKCE login is unchanged.
- Explicit ZITADEL-compatible server introspection and secret-safe request/auth
  diagnostics (`paper-trail-server --verbose`).

Sync is experimental and has not had an independent security audit. Local working
files and comparison baselines remain plaintext. Upgrade the sync server before
connecting updated clients; this version supports encrypted revision v2 and the
multi-folder catalog. See [encrypted sync setup and security boundaries](https://github.com/maennlse/paper-trail/blob/v0.4.0/docs/encrypted-sync.md).

## Downloads

- **GTK desktop, Linux x86_64:** `paper-trail-gtk-linux-x86_64.flatpak` is the
  recommended download. Flatpak installs the separate GNOME runtime from Flathub
  as needed; the bundle does not include the runtime for offline installation.
- **Native GTK, Linux x86_64:** `paper-trail-gtk-linux-x86_64.tar.gz` requires
  compatible GTK/libadwaita/GtkSourceView and glibc libraries. It is not static.
  The archive includes installation guidance and exact build/dependency details.
- **TUI:** choose `paper-trail-tui-<target>.tar.gz` (Linux/macOS) or `.zip`
  (Windows), for x86_64 or ARM64. Linux builds are fully static musl executables.
  Windows builds use the static C runtime plus Windows system DLLs. macOS builds
  link only Apple system libraries dynamically. No GTK, Python, or Rust install
  is needed to run the TUI; external editing requires a separate editor.

Documentation is included in the archives. `SHA256SUMS` and per-asset `.sha256`
files are attached. macOS/Windows binaries are not signed or notarized.

## Changes
