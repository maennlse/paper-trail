# SPDX-License-Identifier: GPL-3.0-or-later

"""Helpers for loading GI modules without tripping import-order linters."""

from __future__ import annotations

import importlib
from collections.abc import Callable
from typing import TypeVar

import gi

T = TypeVar("T")
REQUIRED_GI_VERSIONS: tuple[tuple[str, str], ...] = (
    ("Adw", "1"),
    ("Gdk", "4.0"),
    ("Gio", "2.0"),
    ("GLib", "2.0"),
    ("GObject", "2.0"),
    ("Gtk", "4.0"),
    ("GtkSource", "5"),
    ("Pango", "1.0"),
)


def require_versions() -> None:
    """Register the GI versions used across the application."""

    for namespace, version in REQUIRED_GI_VERSIONS:
        gi.require_version(namespace, version)


require_versions()


def load_modules(*modules: tuple[str, str]) -> tuple[object, ...]:
    """Require and import GI repository modules in order."""

    imported: list[object] = []
    for namespace, version in modules:
        gi.require_version(namespace, version)
        imported.append(importlib.import_module(f"gi.repository.{namespace}"))
    return tuple(imported)


def call_nullary(func: Callable[[], T]) -> T:
    """Call a GI function through a typed wrapper for static analysis."""

    return func()
