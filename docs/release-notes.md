# Paper Trail 0.6.0

## Highlights

- Permanently erase a synced folder's server-side note content/history with an
  explicit confirmation in GTK or TUI. Local files are kept, and authenticated
  deletion markers prevent other devices from silently uploading the folder again.
- Closing a folder now pauses its sync on that device. Reopening restores local
  access only; explicitly re-enable sync to resume. Pending work and server copies
  are retained, and late GTK sync results cannot reopen closed bookmarks.
- Both sync configuration interfaces remain open after deletion and preserve other
  draft selections. Retained local folders offer **Sync as New Folder** with fresh
  folder/note IDs; server-only deletion markers are hidden.
- Optional **Remember encryption password on this device** in graphical sessions,
  with secure credential-store integration and asynchronous startup restoration.
  SSH/headless sessions retain manual, session-only authentication.
- Improved cancellable browser sign-in, Linux portal/browser launching, branded
  callback pages, account/vault prompts, and privacy-preserving connection errors.
- Embedded Neovim input, managed-buffer lifecycle and explicit-write fixes, plus
  clearer TUI status/layout. The built-in editor remains the default.
- Native release build shortcuts: `make tui`, `make desktop`, `make server`, or
  `make all`. Downloads still include all fourteen checksum-verified packages.

Sync is experimental and has not had an independent security audit. Local working
files and comparison baselines remain plaintext. **Upgrade the server and all clients
before using permanent folder deletion.** The server upgrades SQLite schema 1 to 2;
older server binaries then refuse to open it. After erasure, legacy/stale clients
fail closed instead of resurrecting data. Existing vault passphrases and recovery
keys remain valid. Deletion retains encrypted catalog metadata and content-free
history receipts; independent server backups and previously downloaded copies are
not erased. See [encrypted sync setup and security boundaries](https://github.com/maennlse/paper-trail/blob/v0.6.0/docs/encrypted-sync.md).

Embedded Neovim requires an installed `nvim` 0.9+ executable and is not bundled.
Press F2, type `set editor neovim`, and Enter to enable it; F6 changes panes.
Personal configuration/plugins are trusted local code. Switching notes currently
resets the managed buffer's undo history. Browser policy may prevent the successful
login callback tab from closing automatically; manual close remains available.

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
  is needed for built-in TUI editing; external editing and optional embedded Neovim
  require a separate editor executable.
- **Sync server:** choose `paper-trail-server-<target>.tar.gz` (Linux/macOS) or
  `.zip` (Windows), for the same six x86_64/ARM64 targets as the TUI. Linux builds
  are fully static; Windows uses the static CRT and system DLLs; macOS uses only
  Apple system libraries dynamically. SQLite and cryptography are bundled, but an
  OIDC provider and HTTPS reverse proxy must be configured separately. Each archive
  includes deployment guidance and no credentials or database.

Documentation is included in the archives. `SHA256SUMS` and per-asset `.sha256`
files are attached. macOS/Windows binaries are not signed or notarized.

## Changes
