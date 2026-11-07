# Repository Guidelines

## Project Structure & Module Organization

`crates/papertrail-core/` contains portable behavior, `crates/papertrail-storage/`
contains filesystem/settings adapters, and `crates/papertrail-desktop/` contains
the GTK application split into desktop, sidebar, editor, actions, and dialogs.
UI/CSS assets in `crates/papertrail-desktop/resources/` are embedded into the
Rust binary. The legacy Python app and custom editor schemes have been removed;
`tests/fixtures/` retains migration compatibility data. Packaging
metadata is split between the repository root `meson.build`, `data/` for
desktop/metainfo assets, and `io.github.maennlse.paper-trail.json` for
Flatpak. Treat `_build/`, `flatpak-build/`, and `.flatpak-builder/` as
generated output.

## Build, Test, and Development Commands

Run with `cargo run --locked --bin paper-trail`. Build the release executable
with `cargo build --release --locked --bin paper-trail`. Configure and validate
the Meson installation wrapper with `meson setup _build`,
then rebuild with `meson compile -C _build`. Install the local build with
`meson install -C _build` when you need launcher, desktop file, or packaged
asset checks. Build the Flatpak with `flatpak-builder --user --install
--force-clean flatpak-build io.github.maennlse.paper-trail.json`, then run it
using `flatpak run io.github.maennlse.paper-trail`.

## Coding Style & Naming Conventions

For Rust, run `cargo fmt --all` and
`cargo clippy --workspace --all-targets --locked -- -D warnings`. Keep GTK/GLib
out of core/storage, use injected paths and clocks in tests, and keep widget
callbacks weak to avoid reference cycles.

For the Python build helper, use 4-space indentation, type hints, and
`from __future__ import annotations`. Python is not an application runtime dependency.

For UI work, prefer GNOME-native widgets, menus, dialogs, and interaction patterns over custom-built replacements. Use standard GTK/libadwaita components first, and only add custom UI when the native element cannot express the required behavior.

## Testing Guidelines

Run `cargo test --locked -p papertrail-core -p papertrail-storage`. GTK tests run
with `cargo test --locked -p papertrail-desktop -- --test-threads=1` on a display,
or `dbus-run-session -- sh build-aux/test-desktop.sh` with Xvfb and xdotool. Test notes and
settings must be isolated from real user data. Manually check portals, printing,
Wayland drag behavior, and visual parity for native integration changes.
Run `meson compile -C _build` for packaging regressions. Regenerate
`cargo-sources.json` after lockfile changes using
`python3 build-aux/generate_flatpak_sources.py`.

## Commit & Pull Request Guidelines

This repository does not have established commit history yet, so use short imperative commit subjects, for example `Add note filename normalization`. Keep commits scoped to one concern. PRs should describe user-visible changes, list local verification steps, and include screenshots for UI adjustments. Link related issues when applicable and call out any Flatpak or packaging impact explicitly.
