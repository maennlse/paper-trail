# SPDX-License-Identifier: GPL-3.0-or-later

"""Helpers for normalizing and presenting folder colors."""

from __future__ import annotations

import re
from hashlib import sha1

PRESET_FOLDER_COLORS: tuple[str, ...] = (
    "#d49b16",
    "#7a9a01",
    "#1ea672",
    "#148085",
    "#0f81b8",
    "#2563c9",
    "#6f5bd6",
    "#a44ab8",
    "#d6457b",
    "#c13f3f",
    "#d86a1f",
    "#8d5a3b",
)
DEFAULT_FOLDER_COLOR = "preset-0"

_HEX_COLOR_RE = re.compile(r"^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$")


def normalize_folder_color(value: object) -> str:
    """Return a normalized preset token or hex color string."""

    normalized = DEFAULT_FOLDER_COLOR
    if isinstance(value, int):
        index = max(0, min(len(PRESET_FOLDER_COLORS) - 1, value))
        normalized = f"preset-{index}"
    elif isinstance(value, str):
        candidate = value.strip().lower()
        if candidate.startswith("preset-"):
            try:
                index = int(candidate.removeprefix("preset-"))
            except ValueError:
                return normalized
            if 0 <= index < len(PRESET_FOLDER_COLORS):
                normalized = candidate
        elif _HEX_COLOR_RE.fullmatch(candidate):
            if len(candidate) == 4:
                normalized = "#" + "".join(ch * 2 for ch in candidate[1:])
            else:
                normalized = candidate

    return normalized


def is_custom_folder_color(value: object) -> bool:
    """Return whether the color token resolves to a custom hex color."""

    return normalize_folder_color(value).startswith("#")


def folder_color_css_class(value: object) -> str:
    """Return the CSS class name used for a folder color token."""

    color = normalize_folder_color(value)
    if color.startswith("preset-"):
        return f"folder-color-{color.removeprefix('preset-')}"
    return f"folder-color-custom-{sha1(color.encode('utf-8')).hexdigest()[:10]}"


def folder_badge_text(title: str) -> str:
    """Build a short badge label from a folder title."""

    words = [part for part in title.replace("_", " ").replace("-", " ").split() if part]
    if len(words) >= 2:
        return (words[0][0] + words[1][0]).upper()[:2]
    compact = "".join(ch for ch in title if ch.isalnum())
    return compact[:2].upper() or "F"


def custom_folder_colors(
    values: list[object] | tuple[object, ...] | set[object],
) -> set[str]:
    """Collect the custom hex colors present in a folder color set."""

    return {
        color
        for value in values
        if (color := normalize_folder_color(value)).startswith("#")
    }
