# SPDX-License-Identifier: GPL-3.0-or-later

"""Small helpers for common popover interactions."""

from __future__ import annotations

from collections.abc import Callable
from typing import cast

from .gi_helpers import load_modules

_GI_MODULES = cast(tuple[object, object], load_modules(("GLib", "2.0"), ("Gtk", "4.0")))
GLib = _GI_MODULES[0]
Gtk = _GI_MODULES[1]


def popdown(popover: Gtk.Popover | None) -> None:
    """Close a popover when it exists."""

    if popover is not None:
        popover.popdown()


def popup_with_delayed_prelight(
    popover: Gtk.Popover,
    enable_callback: Callable[[Gtk.Popover], bool],
    *,
    suppress_first_prelight: bool,
) -> None:
    """Open a popover and defer button sensitivity until the first idle tick."""

    popover.popup()
    if suppress_first_prelight:
        GLib.idle_add(enable_callback, popover)
