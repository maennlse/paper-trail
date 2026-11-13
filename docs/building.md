# Building Paper Trail

Run commands from the repository root. Paper Trail uses Rust; Python is only
needed for packaging helpers, not to run either client.

## Local build artifacts (interactive)

```sh
python3 build-aux/build_local.py
```

Requires Python 3.11+, Rust 1.88+, Cargo and the selected components' C/build
dependencies. The script asks for **native or standalone**, **Desktop/TUI/Server
components**, and an output directory. It shows prerequisites and the exact Cargo
command, then requires confirmation before building. Nothing is installed
automatically, no tags/releases are created, and no applications are installed.

Each successful run writes archives, individual `.sha256` files and `SHA256SUMS`
to a **fresh timestamped subdirectory of `dist/local/`**. Existing output is never
cleared or overwritten. Archives contain only executables, public documentation
and build information, never notes, settings, databases or credentials.

For automation or a preview:

```sh
python3 build-aux/build_local.py --components tui,server --non-interactive
python3 build-aux/build_local.py --components gtk --non-interactive
python3 build-aux/build_local.py --mode standalone --components tui,server --non-interactive
python3 build-aux/build_local.py --mode standalone --components server --dry-run
```

- **Native** is the default: local TUI/server archives have `-native` in their
  names and a `BUILD-INFO.txt` warning that compatible OS/runtime libraries may
  be needed. They are **not** silently advertised as static/portable releases.
  GTK packaging currently supports Linux x86_64 and requires the GNOME development
  libraries below; other hosts can build TUI/server without GTK.
- **Standalone** builds TUI/server using the same target and linkage checks as CI:
  Linux musl, Windows MSVC with static CRT, or Apple system libraries on macOS.
  Install the matching Rust target and native compiler/tooling beforehand. Linux
  needs `musl-gcc` and `readelf`; Windows needs MSVC build tools; macOS needs `otool`.
  A missing target/tool fails clearly; the script never silently downgrades to a
  dynamic build. GTK is not a standalone component.
- Targets must match the current machine's OS/CPU because packaging smoke-tests
  the executable. `--target` can specify that matching target explicitly; use the
  GitHub Actions matrix to produce other platform/architecture artifacts.
- `--output-dir PATH` changes the output parent. `CARGO_TARGET_DIR` is honored;
  explicit `--target` and `--locked` are always passed to Cargo. `--dry-run` creates
  no output and skips dependency probes/builds. Without a TTY, use
  `--non-interactive` to avoid prompts.

## Terminal client

Requires Rust 1.88+, Cargo, and a C linker. No GTK or display is required.

```sh
cargo run --locked --bin paper-trail-tui
cargo build --release --locked --bin paper-trail-tui
```

The executable is `target/release/paper-trail-tui` (`.exe` on Windows). See the
[terminal guide](tui.md) for configuration, keybindings, and static/portable
standalone build targets.

The TUI's optional embedded Neovim backend uses private stdio RPC and needs `nvim`
0.9+ only when selected; it adds no GTK dependency and is not bundled in archives.
Tests use isolated Neovim homes and clean configuration. Linux standalone CI installs
Neovim and sets `PAPERTRAIL_NVIM_TESTS=1` so missing integration coverage fails rather
than silently skipping; other hosts skip those tests when Neovim is not installed.

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
python3 -m unittest discover -s build-aux -p test_build_local.py
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

### Refresh readme screenshots

The screenshots are captures of real GTK and VTE terminal windows, not mockups.
Build both release clients first. On Linux, install screenshot-only developer
tools: Python PyGObject with GTK 4 and VTE 3.91 bindings, Xvfb, xdotool, ImageMagick,
and D-Bus. These are not application runtime requirements.

```sh
cargo build --release --locked --bin paper-trail --bin paper-trail-tui
dbus-run-session -- xvfb-run -a -s '-screen 0 1600x1000x24' python3 build-aux/capture_screenshots.py
```

The helper creates a temporary home/settings profile with synthetic notes and
public example sync settings; it does not authenticate, contact a sync server,
or read your real notes. It captures the desktop editor, terminal All view, and
interactive sync-folder checklist into `data/screenshots/`. `--output-dir PATH`
can redirect captures without changing the readme assets.
`--editor neovim` captures the optional backend using clean, isolated Neovim
configuration and names the terminal images `tui-neovim.png` and
`tui-sync-folders-neovim.png`; it requires an installed Neovim runtime.

## Workspace layout

- `papertrail-core`: portable models, search, organization, and editing sessions.
- `papertrail-storage`: filesystem/JSON adapters, atomic writes, and safe moves.
- `papertrail-desktop`: GTK shell, editor, sidebar, dialogs, and native integration.
- `papertrail-tui`: Ratatui terminal client with no GTK dependencies.

Core and storage have no GTK/GLib dependency. Native desktop applications for
Windows/macOS and mobile platforms are not currently supported.
