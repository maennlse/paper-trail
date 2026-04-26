# Paper Trail

Paper Trail is a local plain-text note app built as a fresh GNOME application.

It is completely vibe coded and a shameless copy of Folio, GNOME Text Editor, and Iotas.

It is intentionally narrow in scope:

- local `.txt` files only
- no Markdown
- no SQLite
- no sync
- a compact note list and GNOME-style shell
- a `GtkSourceView` editor configured for plain text, borrowing the editor direction from GNOME Text Editor

## Run

```bash
PYTHONPATH=. python3 -m papertrail
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

The Flatpak manifest is [io.github.maennlse.paper-trail.json](io.github.maennlse.paper-trail.json). It uses the GNOME runtime, stores notes inside the app sandbox by default, and relies on the file chooser portal for user-selected folders outside the sandbox.

## Build

```bash
meson setup _build
meson compile -C _build
meson install -C _build
```

This is a Python GNOME app, so `meson compile` mostly validates/configures packaging and `meson install` installs the launcher, desktop file, metainfo, Python package, and UI files.

## Runtime dependencies

You need these GNOME Python bindings and libraries available on your system:

- `python3`
- `python3-gobject`
- `gtk4`
- `libadwaita`
- `gtksourceview5`

On Fedora:

```bash
sudo dnf install python3-gobject gtk4 libadwaita gtksourceview5
```

On Debian/Ubuntu:

```bash
sudo apt install python3-gi gir1.2-gtk-4.0 gir1.2-adw-1 gir1.2-gtksource-5
```

## Current behavior

- stores notes as UTF-8 `.txt` files in `~/Documents/Paper Trail` by default
- lets you switch to another local folder from the window menu
- creates, opens, searches, edits, autosaves, and deletes notes
- keeps line-number, wrap, and monospace preferences in a small JSON config under `~/.config/paper-trail/`

## Upstream references

- Folio: <https://gitlab.gnome.org/World/folio>
- Iotas: <https://gitlab.gnome.org/World/iotas>
- GNOME Text Editor: <https://gitlab.gnome.org/GNOME/gnome-text-editor>
