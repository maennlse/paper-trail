# SPDX-License-Identifier: GPL-3.0-or-later

"""Application entry point and top-level window lifecycle."""

from __future__ import annotations

import os
from pathlib import Path
from typing import cast

from . import APP_ID
from .gi_helpers import call_nullary, load_modules
from .window import PaperTrailWindow

_GTK_MODULES = cast(
    tuple[object, object, object, object],
    load_modules(
        ("Adw", "1"),
        ("Gdk", "4.0"),
        ("Gio", "2.0"),
        ("Gtk", "4.0"),
    ),
)
Adw = _GTK_MODULES[0]
Gdk = _GTK_MODULES[1]
Gio = _GTK_MODULES[2]
Gtk = _GTK_MODULES[3]

ICON_SEARCH_PATH = Path(__file__).resolve().parent.parent / "data" / "icons"
DEV_APP_ID = f"{APP_ID}.Devel"


def _application_id() -> str:
    """Return the runtime application ID.

    Source checkouts use a separate ID so they do not remote-activate an
    installed Flatpak instance with the release application ID.
    """

    if os.environ.get("FLATPAK_ID"):
        return APP_ID
    if (Path(__file__).resolve().parent.parent / ".git").exists():
        return DEV_APP_ID
    return APP_ID


class PaperTrailApplication(Adw.Application):
    """Main Adwaita application object for Paper Trail."""

    def __init__(self) -> None:
        super().__init__(
            application_id=_application_id(), flags=Gio.ApplicationFlags(0)
        )
        self._window: PaperTrailWindow | None = None

    def do_startup(self, *args, **kwargs) -> None:
        """Complete startup and register application-wide actions."""

        del args, kwargs
        _startup_application(self)
        display = call_nullary(Gdk.Display.get_default)
        if display is not None and ICON_SEARCH_PATH.exists():
            Gtk.IconTheme.get_for_display(display).add_search_path(
                str(ICON_SEARCH_PATH)
            )
        Gtk.Window.set_default_icon_name(APP_ID)
        self._setup_actions()

        self.set_accels_for_action("win.new-note", ["<Control>n"])
        self.set_accels_for_action("win.close-note", ["<Control>w"])
        self.set_accels_for_action("win.choose-folder", ["<Control>o"])
        self.set_accels_for_action("win.print-note", ["<Control>p"])
        self.set_accels_for_action("win.show-preferences", ["<Control>comma"])
        self.set_accels_for_action("win.show-shortcuts", ["<Control>question"])
        self.set_accels_for_action("win.delete-note", ["<Control>Delete"])
        self.set_accels_for_action("win.toggle-fullscreen", ["F11"])
        self.set_accels_for_action("win.toggle-sidebar", ["F9", "<Control>b"])
        self.set_accels_for_action("win.toggle-info", ["F10", "<Control>i"])
        self.set_accels_for_action("win.focus-sidebar-search", ["<Control>k"])
        self.set_accels_for_action("win.toggle-editor-search", ["<Control>f"])
        self.set_accels_for_action("win.focus-replace", ["<Control>h"])
        self.set_accels_for_action("win.find-next", ["F3", "<Control>g"])
        self.set_accels_for_action(
            "win.find-previous", ["<Shift>F3", "<Control><Shift>g"]
        )
        self.set_accels_for_action("win.zoom-in", ["<Control>plus", "<Control>equal"])
        self.set_accels_for_action("win.zoom-out", ["<Control>minus"])
        self.set_accels_for_action("win.zoom-reset", ["<Control>0"])
        self.set_accels_for_action("win.replace-current", ["<Control>Return"])
        self.set_accels_for_action("win.replace-all", ["<Control><Shift>Return"])
        self.set_accels_for_action("win.toggle-line-numbers", ["F7"])
        self.set_accels_for_action("win.toggle-wrap", ["<Alt>z"])
        self.set_accels_for_action("win.toggle-monospace", ["<Control>m"])
        self.set_accels_for_action("win.close-search", ["Escape"])
        self.set_accels_for_action("app.about", ["F1"])
        self.set_accels_for_action("app.quit", ["<Control>q"])

    def do_activate(self, *args, **kwargs) -> None:
        """Present the main window, creating it on first activation."""

        del args, kwargs
        if self._window is None:
            self._window = PaperTrailWindow(application=self)
        self._window.present()

    def _setup_actions(self) -> None:
        """Install application-scoped actions."""

        quit_action = Gio.SimpleAction.new("quit", None)
        quit_action.connect("activate", lambda *_args: self.quit())
        self.add_action(quit_action)

        about_action = Gio.SimpleAction.new("about", None)
        about_action.connect("activate", self._show_about)
        self.add_action(about_action)

    def _show_about(self, *_args) -> None:
        """Present the about dialog."""

        if self._window is None:
            return

        about = Adw.AboutDialog(
            application_name="Paper Trail",
            application_icon=APP_ID,
            developer_name="sm",
            version="0.1.0",
            website="https://github.com/maennlse/paper-trail",
            issue_url="https://github.com/maennlse/paper-trail/issues",
            license_type=Gtk.License.GPL_3_0,
        )
        about.add_css_class("papertrail-dialog")
        about.set_presentation_mode(Adw.DialogPresentationMode.FLOATING)
        about.set_comments(
            "Local plain-text notes with an Iotas-like shell and a GtkSourceView editor."
        )
        about.present(self._window)


def main() -> int:
    """Run the application and return its exit status."""

    call_nullary(Adw.init)
    app = PaperTrailApplication()
    return app.run(None)


def _startup_application(app: "PaperTrailApplication") -> None:
    """Chain up to the GTK application startup implementation."""

    Gtk.Application.do_startup(app)
