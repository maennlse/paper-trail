# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from hashlib import sha1
import re


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
    if isinstance(value, int):
        index = max(0, min(len(PRESET_FOLDER_COLORS) - 1, value))
        return f"preset-{index}"

    if isinstance(value, str):
        candidate = value.strip().lower()
        if candidate.startswith("preset-"):
            try:
                index = int(candidate.removeprefix("preset-"))
            except ValueError:
                return DEFAULT_FOLDER_COLOR
            if 0 <= index < len(PRESET_FOLDER_COLORS):
                return candidate
            return DEFAULT_FOLDER_COLOR
        if _HEX_COLOR_RE.fullmatch(candidate):
            if len(candidate) == 4:
                return "#" + "".join(ch * 2 for ch in candidate[1:])
            return candidate

    return DEFAULT_FOLDER_COLOR


def is_custom_folder_color(value: object) -> bool:
    return normalize_folder_color(value).startswith("#")


def folder_color_css_class(value: object) -> str:
    color = normalize_folder_color(value)
    if color.startswith("preset-"):
        return f"folder-color-{color.removeprefix('preset-')}"
    return f"folder-color-custom-{sha1(color.encode('utf-8')).hexdigest()[:10]}"


def custom_folder_colors(values: list[object] | tuple[object, ...] | set[object]) -> set[str]:
    return {color for value in values if (color := normalize_folder_color(value)).startswith("#")}
