# Building Paper Trail

Run commands from the repository root. Paper Trail uses Rust; Python is only
needed for packaging helpers, not to run either client.

## Terminal client

Requires Rust 1.88+, Cargo, and a C linker. No GTK or display is required.

```sh
cargo run --locked --bin paper-trail-tui
cargo build --release --locked --bin paper-trail-tui
```

The executable is `target/release/paper-trail-tui` (`.exe` on Windows). See the
[terminal guide](tui.md) for configuration, keybindings, and static/portable
standalone build targets.

## Desktop prerequisites

Requires Linux, Rust 1.88+, Cargo, a C linker, pkg-config, the GLib resource
compiler, and development headers for:

- GTK 4.16+
- libadwaita 1.5+
- GtkSourceView 5.10+

On Fedora:

```sh
sudo dnf install rust cargo gcc pkgconf-pkg-config gtk4-devel libadwaita-devel gtksourceview5-devel
```

On Debian/Ubuntu:

```sh
sudo apt install cargo rustc build-essential pkg-config libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev
```

Use a distribution providing the minimum Rust and library versions, or the GNOME
Flatpak SDK. Older releases such as Ubuntu 24.04 provide an older GTK version.

## Run and build the desktop app

```sh
cargo run --locked --bin paper-trail
cargo build --release --locked --bin paper-trail
```

The executable is `target/release/paper-trail`. UI, CSS, and the icon are embedded,
so no loose application assets are needed. GNOME libraries are dynamically
linked: this is not a static executable. See the
[desktop download guide](desktop-release.md) for runtime requirements.

To install the executable, desktop entry, icon, and AppStream metadata, install
Meson and Ninja, then run:

```sh
meson setup _build
meson compile -C _build
meson install -C _build
```

Meson delegates compilation to Cargo.

## Flatpak

Install Flatpak, flatpak-builder, and the GNOME 50 SDK/runtime from Flathub.
The manifest also requires `org.freedesktop.Sdk.Extension.rust-stable` for the
matching SDK branch.

```sh
flatpak-builder --user --install --force-clean flatpak-build io.github.maennlse.paper-trail.json
flatpak run io.github.maennlse.paper-trail
```

The [manifest](../io.github.maennlse.paper-trail.json) uses the GNOME runtime and
file chooser portals for external folders. Notes default to the app sandbox.
Cargo dependencies are checksum-pinned in `cargo-sources.json`; compilation
works offline inside the sandbox. After changing `Cargo.lock`, regenerate it:

```sh
python3 build-aux/generate_flatpak_sources.py
```

Dependency-update PRs must include this regenerated file before merging.

## Tests and lint

Portable tests need no display or GNOME libraries and run on Linux, Windows,
and macOS:

```sh
cargo test --locked -p papertrail-core -p papertrail-storage -p papertrail-tui
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 -m unittest discover -s build-aux -p test_release.py
python3 build-aux/generate_flatpak_sources.py --check
```

Workspace Clippy includes the desktop crate and requires its development
libraries. Without GTK, select only the three portable packages with `-p`.

GTK tests require a display and isolated temporary notes/settings:

```sh
cargo test --locked -p papertrail-desktop -- --test-threads=1
```

For headless testing, install Xvfb, xdotool, and D-Bus, then run:

```sh
dbus-run-session -- sh build-aux/test-desktop.sh
```

This also tests search-button clicks and shortcuts-dialog dismissal through X11
input events. Set `PAPERTRAIL_SCREENSHOT` to a PNG path to capture the rendered UI.
Compatibility fixtures protect existing notes and legacy settings.

## Workspace layout

- `papertrail-core`: portable models, search, organization, and editing sessions.
- `papertrail-storage`: filesystem/JSON adapters, atomic writes, and safe moves.
- `papertrail-desktop`: GTK shell, editor, sidebar, dialogs, and native integration.
- `papertrail-tui`: Ratatui terminal client with no GTK dependencies.

Core and storage have no GTK/GLib dependency. Native desktop applications for
Windows/macOS and mobile platforms are not currently supported.
