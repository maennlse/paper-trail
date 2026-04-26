# Repository Guidelines

## Project Structure & Module Organization
`papertrail/` contains the application code. `application.py` wires up the Adwaita app, `window.py` owns most UI behavior, and `note_repository.py`, `settings.py`, and `note_row.py` hold supporting logic. UI assets live in `papertrail/ui/` and editor style schemes in `papertrail/styles/`. Packaging metadata is split between the repo root `meson.build`, `data/` for desktop/metainfo assets, and `io.github.maennlse.paper-trail.json` for Flatpak. Treat `_build/`, `flatpak-build/`, and `.flatpak-builder/` as generated output.

## Build, Test, and Development Commands
Run the app directly during development with `PYTHONPATH=. python3 -m papertrail`. Configure and validate the Meson build with `meson setup _build`, then rebuild with `meson compile -C _build`. Install the local build with `meson install -C _build` when you need launcher, desktop file, or packaged asset checks. Build the Flatpak with `flatpak-builder --user --install --force-clean flatpak-build io.github.maennlse.paper-trail.json`, then run it using `flatpak run io.github.maennlse.paper-trail`.

## Coding Style & Naming Conventions
Follow the existing Python style: 4-space indentation, type hints, `from __future__ import annotations`, and small focused classes. Use `snake_case` for functions, methods, and module names; `PascalCase` for classes; and `UPPER_SNAKE_CASE` for UI constants. Keep GTK signal handlers and action names explicit, matching current patterns such as `_on_sidebar_search_changed` and `"win.new-note"`. No formatter or linter is configured yet, so keep changes PEP 8-aligned and consistent with nearby code.

For UI work, prefer GNOME-native widgets, menus, dialogs, and interaction patterns over custom-built replacements. Use standard GTK/libadwaita components first, and only add custom UI when the native element cannot express the required behavior.

## Testing Guidelines
There is no automated test suite yet. Before opening a PR, run the app locally and verify note creation, rename/delete, search, autosave, folder switching, and preferences changes. At minimum, run `meson compile -C _build` to catch packaging or install path regressions. If you add test coverage later, prefer `tests/test_*.py` and keep UI-independent logic isolated in small modules.

## Commit & Pull Request Guidelines
This repository does not have established commit history yet, so use short imperative commit subjects, for example `Add note filename normalization`. Keep commits scoped to one concern. PRs should describe user-visible changes, list local verification steps, and include screenshots for UI adjustments. Link related issues when applicable and call out any Flatpak or packaging impact explicitly.
