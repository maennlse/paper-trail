# Paper Trail

Paper Trail is a local plain-text note app for GNOME, written in Rust with GTK4,
libadwaita, and GtkSourceView 5.

> [!IMPORTANT]
> This project is completely vibe coded.
> It is also a shameless copy of Folio, GNOME Text Editor, and Iotas.

It is intentionally narrow in scope:

- local `.txt` files only
- no Markdown
- no SQLite
- no sync
- a compact note list and GNOME-style shell
- a `GtkSourceView` editor configured for plain text, borrowing the editor direction from GNOME Text Editor

## Run

```bash
cargo run --locked --bin paper-trail
```

## Flatpak

Build the app as a Flatpak from this checkout with:

```bash
flatpak-builder --user --install --force-clean flatpak-build io.github.maennlse.paper-trail.json
```

Then run it with:

```bash
flatpak run io.github.maennlse.paper-trail
```

The Flatpak manifest is [io.github.maennlse.paper-trail.json](io.github.maennlse.paper-trail.json).
It uses the GNOME runtime, stores notes inside the app sandbox by default, and
relies on the file chooser portal for user-selected folders outside the sandbox.
Cargo dependencies are checksum-pinned in `cargo-sources.json` and builds work
offline inside the sandbox. After changing `Cargo.lock`, regenerate sources with
`python3 build-aux/generate-flatpak-sources.py` (build tooling only).

## Build

```bash
cargo build --release --locked --bin paper-trail
```

The executable is `target/release/paper-trail`. Its UI, CSS, and application icon
are embedded: it can run outside the checkout without Python or loose app assets.
It dynamically links GNOME libraries; it is **not** a fully static executable.
Use Flatpak for a self-contained Linux distribution with the required runtime.

For installation with desktop entry, shell icon, and AppStream metadata:

```bash
meson setup _build
meson compile -C _build
meson install -C _build
```

Meson delegates compilation to Cargo and installs the binary and desktop metadata.

## Tests

Run the test suite from the repository root with:

```bash
cargo test --locked -p papertrail-core -p papertrail-storage
cargo test --locked -p papertrail-desktop -- --test-threads=1
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

GTK tests require a display and use isolated temporary notes/settings. On a
headless Linux machine with Xvfb and xdotool installed, run
`dbus-run-session -- sh build-aux/test-desktop.sh`. This also tests real search-button
clicks and shortcuts-dialog dismissal through X11 input events. Set `PAPERTRAIL_SCREENSHOT`
to a PNG path for an optional rendered UI capture. Core/storage tests need no
display or GNOME libraries and also run on Windows/macOS in CI.

The legacy Python app and its tests have been retired after user acceptance.
Compatibility fixtures remain to protect existing notes and settings. Python is
only used by the optional Flatpak dependency-generation helper.

## Runtime dependencies

Build requirements: Rust 1.85+, Cargo, a C linker, pkg-config, GLib resource
compiler, and development headers for these libraries. Runtime requirements:

- GTK 4.16+ (the existing CSS uses custom properties)
- libadwaita 1.5+
- GtkSourceView 5.10+

On Fedora:

```bash
sudo dnf install rust cargo gcc pkgconf-pkg-config gtk4-devel libadwaita-devel gtksourceview5-devel
```

On Debian/Ubuntu:

```bash
sudo apt install cargo rustc build-essential pkg-config libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev
```

Use a distribution release providing the minimum library versions above, or the
GNOME Flatpak SDK. Older releases such as Ubuntu 24.04 have an older GTK version.

## Current behavior

- stores notes as UTF-8 `.txt` files in `~/Documents/Paper Trail` by default
- lets you switch to another local folder from the window menu
- creates, opens, searches, edits, autosaves, and deletes notes
- renames and moves notes without overwriting existing files, including across filesystems
- supports multiple folders, folder colors/reordering, pinning, and cross-folder search
- provides syntax detection/overrides, find/replace, undo/redo, current-line highlighting, custom fonts, zoom, and printing
- keeps preferences in the existing JSON config under `~/.config/paper-trail/`
- follows Adwaita light/dark/system appearance; legacy custom editor schemes are ignored

New notes use `.txt`; existing regular files are also listed, including
extensionless notes. Existing notes and Python settings need no conversion.

If a saved folder is unavailable (for example, a stale Flatpak document-portal
bookmark), startup uses the default notes folder and offers **Choose Folder…**
to restore access. The original bookmark, colors, pins, and language overrides
are retained; no notes are moved or deleted. Unavailable folders are skipped in
cross-folder views and identified in their sidebar tooltips.

## Architecture

- `papertrail-core`: portable models, normalization, search, organization, and
  dirty editing-session behavior; no GTK/GLib dependency.
- `papertrail-storage`: local filesystem and JSON adapter with atomic writes and
  no-clobber moves; paths are supplied by the caller.
- `papertrail-desktop`: Linux GTK shell, editor, sidebar, actions, dialogs, and
  native integration. UI/CSS in `crates/papertrail-desktop/resources/` are embedded at build time.

Future CLI/TUI clients can reuse the first two crates. Future native desktop or
mobile applications can reuse core behavior and provide their own storage and
platform integrations; GTK widgets themselves are not a mobile abstraction.
Windows/macOS desktop applications and iOS/Android are not currently supported.

## Upstream references

- Folio: <https://gitlab.gnome.org/World/folio>
- Iotas: <https://gitlab.gnome.org/World/iotas>
- GNOME Text Editor: <https://gitlab.gnome.org/GNOME/gnome-text-editor>
