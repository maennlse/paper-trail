# Paper Trail GTK desktop downloads

## Recommended: Flatpak (Linux x86_64)

Install `paper-trail-gtk-linux-x86_64.flatpak` with Flatpak:

```sh
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user ./paper-trail-gtk-linux-x86_64.flatpak
flatpak run io.github.maennlse.paper-trail
```

The bundle includes the app, not the GNOME runtime itself. Flatpak downloads the
required GNOME 50 runtime from Flathub if it is not already installed. An initial
installation therefore needs network access. External folders use the file
chooser portal. The executable does not need Python or a Rust installation.

## Native executable (Linux x86_64)

Extract `paper-trail-gtk-linux-x86_64.tar.gz` and run `./paper-trail` in the
extracted directory. Executable permissions are preserved in the archive. UI,
CSS, and the app icon are embedded, so there are no loose application resources
to install alongside the executable.

This build is **dynamically linked**, not a standalone/static Linux executable.
It requires compatible system libraries, including:

- GTK 4.16 or newer
- libadwaita 1.5 or newer
- GtkSourceView 5.10 or newer
- Compatible GLib, glibc, and their runtime dependencies

Release builds currently use Fedora 44. `BUILD-INFO.txt` in the archive records
the actual distribution, GNOME library versions, and dynamic dependencies used
for that build. Those versions may imply newer requirements than the source
minimums above. Older distributions may not run this binary even if they provide
some GTK libraries; prefer Flatpak rather than replacing system libraries.

On a compatible Fedora release, install the runtime libraries with:

```sh
sudo dnf install gtk4 libadwaita gtksourceview5
```

The native archive does not install a launcher or shell icon. Build and install
with Meson if system desktop integration is needed. Printing, portals, and
Wayland/X11 require the corresponding desktop services.

## Verify downloads

Download `SHA256SUMS` and the desired assets from the same release, then check
their entries with a SHA-256 tool. Each archive/bundle also has an individual
`.sha256` sidecar, so one download can be checked without the other assets:

```sh
sha256sum --check paper-trail-gtk-linux-x86_64.tar.gz.sha256
```

These checksums detect corruption; they are not signatures. Releases are not
currently signed or notarized. Existing notes and settings need no conversion.
