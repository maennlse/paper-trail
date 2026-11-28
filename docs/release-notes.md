# Paper Trail 0.8.0

## Highlights

- Native aarch64 GTK archives and Flatpak bundles join the x86_64 desktop builds.
  Every platform and build variant uses the unified version 0.8.0.
- Improved iOS note browsing with bottom search, a `+` button, folder badges,
  upper-right timestamps, and native full-swipe actions with delete confirmation.
- Floating Runestone-powered Find with match counts, navigation, case/matching
  options, and regular expressions; share note text as well as files.
- Bash/shell, HTML, CSS, and SQL highlighting alongside existing languages.
- Fixed remote-folder downloads, sync after successful note close, and pull-to-sync.
- Independent server-folder deletion, clearer folder discovery and mappings,
  persistent synced folder renames, and simpler URL-only account setup.
- Unified TUI command names and server `--help`/`--version` support.
- All seventeen release packages require verified checksums and successful builds;
  iOS simulator UI, state, and grammar tests now gate release publication.

Sync is experimental and has not had an independent security audit. Local working
files and comparison baselines remain plaintext. **Upgrade the server and all clients
before using permanent folder deletion.** The server upgrades SQLite schema 1 to 2;
older server binaries then refuse to open it. After erasure, legacy/stale clients
fail closed instead of resurrecting data. Existing vault passphrases and recovery
keys remain valid. Deletion retains encrypted catalog metadata and content-free
history receipts; independent server backups and previously downloaded copies are
not erased. See [encrypted sync setup and security boundaries](https://github.com/maennlse/paper-trail/blob/v0.8.0/docs/encrypted-sync.md).

Embedded Neovim requires an installed `nvim` 0.9+ executable and is not bundled.
Press F2, type `set editor neovim`, and Enter to enable it; F6 changes panes.
Personal configuration/plugins are trusted local code. Switching notes currently
resets the managed buffer's undo history. Browser policy may prevent the successful
login callback tab from closing automatically; manual close remains available.

## Downloads

- **iOS 17+, ARM64 (experimental):** `paper-trail-ios-arm64-unsigned.ipa` requires
  signing with your chosen sideloading tool before installation. It is not an
  App Store or TestFlight build. No Apple credentials are used in CI. See the
  [iOS build and sideloading guide](https://github.com/maennlse/paper-trail/blob/v0.8.0/docs/ios.md).
- **GTK desktop, Linux x86_64/aarch64:** `paper-trail-gtk-linux-ARCH.flatpak` is the
  recommended download (replace `ARCH` with `x86_64` or `aarch64`). Flatpak installs the separate GNOME runtime from Flathub
  as needed; the bundle does not include the runtime for offline installation.
- **Native GTK, Linux x86_64/aarch64:** `paper-trail-gtk-linux-ARCH.tar.gz` requires
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
