# Paper Trail 0.5.0

## Highlights

- Optional public OIDC configuration discovery in GTK and TUI, with explicit
  issuer confirmation and pinning. Manual setup remains supported; existing
  profiles do not silently change authentication providers or vault identities.
- Real Neovim embedded in the TUI editor pane, using your normal configuration
  or clean defaults. The built-in editor remains the default. Paper Trail keeps
  ownership of managed-note saving, autosave, and external-edit conflict checks.
- Interactive TUI sync-folder selection, an All notes folder row, improved folder
  badges, and shared GTK/TUI RGB accents.
- GTK sync-dialog lifecycle, folder-editing, and footer layout fixes.
- Standalone sync-server downloads for all six TUI targets, checksum-verified
  fourteen-package releases, and interactive local artifact builds.

Sync is experimental and has not had an independent security audit. Local working
files and comparison baselines remain plaintext. Encrypted formats, account binding,
and token validation are unchanged. Update the server to use public authentication
discovery; older servers can still use manual configuration.
See [encrypted sync setup and security boundaries](https://github.com/maennlse/paper-trail/blob/v0.5.0/docs/encrypted-sync.md).

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
