# Paper Trail

![Paper Trail logo](data/icons/io.github.maennlse.paper-trail.svg)

**Your notes. Plain text. Desktop or terminal.**

[![Release](https://img.shields.io/github/v/release/maennlse/paper-trail)](https://github.com/maennlse/paper-trail/releases/latest)
[![Build](https://github.com/maennlse/paper-trail/actions/workflows/rust.yml/badge.svg)](https://github.com/maennlse/paper-trail/actions/workflows/rust.yml)
[![Standalone TUI](https://github.com/maennlse/paper-trail/actions/workflows/tui.yml/badge.svg)](https://github.com/maennlse/paper-trail/actions/workflows/tui.yml)
[![Lint](https://github.com/maennlse/paper-trail/actions/workflows/super-linter.yml/badge.svg)](https://github.com/maennlse/paper-trail/actions/workflows/super-linter.yml)
[![Built with Rust](https://img.shields.io/badge/built_with-Rust-dea584?logo=rust)](https://www.rust-lang.org/)
[![TUI platforms: Linux, Windows, macOS](https://img.shields.io/badge/TUI-Linux%20%C2%B7%20Windows%20%C2%B7%20macOS-4c8eda)](docs/tui.md)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](https://www.gnu.org/licenses/gpl-3.0.html)

Paper Trail is a local-first note app written in Rust, with a native GNOME desktop
app and a portable terminal client. Your notes stay in ordinary text files.
Optional client-side end-to-end encrypted sync uses a separately deployed server;
the server does not hold the decryption key. Local-only use needs no account.

> [!IMPORTANT]
> This project is completely vibe coded.

## Make room for your thoughts

- **Keep it simple.** Create, search, edit, and autosave plain-text notes.
- **Stay organized.** Multiple folders, pinned notes, folder colors, and search
  across folders.
- **Keep control.** Safe renames and moves, atomic saves, and conflict detection
  when desktop and terminal share notes.
- **Feel at home.** Native desktop dialogs, light/dark appearance, fonts, zoom,
  find/replace, and printing.
- **Work from the terminal.** Vim-style editing, your external editor, instant
  previews, mouse navigation, and command completion—without GTK.

## Get Paper Trail

[**Download the latest release →**](https://github.com/maennlse/paper-trail/releases/latest)

| Experience            | Platforms                              | Download               |
| --------------------- | -------------------------------------- | ---------------------- |
| Desktop · recommended | Linux x86_64                           | Flatpak bundle         |
| Desktop · native      | Linux x86_64                           | GTK executable archive |
| Terminal              | Linux, Windows, macOS · x86_64 & ARM64 | Standalone archive     |

Every release includes documentation and SHA-256 checksums. Native GTK downloads
need compatible system libraries; Flatpak installs its separate GNOME runtime as
needed. TUI downloads need no GTK, Python, or Rust installation.

[Desktop installation guide](docs/desktop-release.md) ·
[Terminal guide & keybindings](docs/tui.md)

## Plain files, lasting notes

New notes are UTF-8 `.txt` files, stored in `~/Documents/Paper Trail` by default.
Choose another local folder whenever you like. Existing regular files, including
extensionless notes, work too; existing notes and settings need no conversion.

If a saved folder becomes unavailable, Paper Trail retains its bookmark and
metadata and lets you restore access. It does not move or delete your notes.

## Explore further

- [Build, run, and test from source](docs/building.md)
- [Encrypted sync, server setup, and security boundaries](docs/encrypted-sync.md)
- [Short sync architecture note](docs/sync-architecture.md)
- [Terminal usage and standalone targets](docs/tui.md)
- [Release process](docs/releases.md)
- [Report an issue](https://github.com/maennlse/paper-trail/issues)
