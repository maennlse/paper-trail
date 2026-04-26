# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from dataclasses import dataclass, asdict
import os
from pathlib import Path
import json

from gi.repository import GLib

from .folder_colors import DEFAULT_FOLDER_COLOR, PRESET_FOLDER_COLORS, normalize_folder_color


def _default_notes_dir() -> Path:
    if os.environ.get("FLATPAK_ID"):
        return Path(GLib.get_user_data_dir()) / "notes"
    documents = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_DOCUMENTS)
    base = Path(documents) if documents else Path.home() / "Documents"
    return base / "Paper Trail"


@dataclass(slots=True)
class SettingsData:
    notes_dir: str
    note_folders: list[str] | None = None
    folder_colors: dict[str, str] | None = None
    pinned_notes: list[str] | None = None
    show_line_numbers: bool = True
    wrap_text: bool = True
    use_monospace: bool = False
    show_ruler: bool = True
    theme_mode: str = "system"
    editor_style_scheme: str = "Adwaita"
    font_scale: float = 1.0
    use_custom_font: bool = False
    custom_font: str = "Monospace 11"
    note_language_overrides: dict[str, str] | None = None
    sidebar_width: int = 260
    width: int = 1320
    height: int = 860


class Settings:
    def __init__(self) -> None:
        config_dir = Path(GLib.get_user_config_dir()) / "paper-trail"
        self._path = config_dir / "settings.json"
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self.data = self._load()

    def _load(self) -> SettingsData:
        default_dir = str(_default_notes_dir())
        defaults = SettingsData(notes_dir=default_dir, note_folders=[default_dir])
        if not self._path.exists():
            return defaults

        try:
            raw = json.loads(self._path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError, TypeError, ValueError):
            return defaults

        merged = asdict(defaults)
        if isinstance(raw, dict):
            merged.update({key: value for key, value in raw.items() if key in merged})
        data = SettingsData(**merged)
        folders = self._normalise_folders(data.note_folders, defaults.notes_dir)
        active_dir = str(Path(data.notes_dir).expanduser())
        if active_dir not in folders:
            active_dir = folders[0]
        data.note_folders = folders
        data.notes_dir = active_dir
        colors = data.folder_colors if isinstance(data.folder_colors, dict) else {}
        normalised_colors: dict[str, str] = {}
        for folder in folders:
            normalised_colors[folder] = normalize_folder_color(colors.get(folder, DEFAULT_FOLDER_COLOR))
        data.folder_colors = normalised_colors
        pins = data.pinned_notes if isinstance(data.pinned_notes, list) else []
        unique_pins: list[str] = []
        for pin in pins:
            if not isinstance(pin, str):
                continue
            normalised = str(Path(pin).expanduser())
            if normalised not in unique_pins:
                unique_pins.append(normalised)
        data.pinned_notes = unique_pins
        if data.theme_mode not in {"system", "light", "dark"}:
            data.theme_mode = defaults.theme_mode
        if not isinstance(data.editor_style_scheme, str) or not data.editor_style_scheme.strip():
            data.editor_style_scheme = defaults.editor_style_scheme
        elif data.editor_style_scheme == "automatic":
            data.editor_style_scheme = defaults.editor_style_scheme
        try:
            data.font_scale = float(data.font_scale)
        except (TypeError, ValueError):
            data.font_scale = defaults.font_scale
        data.font_scale = max(0.7, min(2.5, data.font_scale))
        data.use_custom_font = bool(data.use_custom_font)
        if not isinstance(data.custom_font, str) or not data.custom_font.strip():
            data.custom_font = defaults.custom_font
        data.show_ruler = bool(data.show_ruler)
        return data

    def _normalise_folders(
        self,
        folders: list[str] | None,
        fallback: str,
    ) -> list[str]:
        unique: list[str] = []
        for value in folders or [fallback]:
            path = str(Path(value).expanduser())
            if path not in unique:
                unique.append(path)
        if not unique:
            unique.append(str(Path(fallback).expanduser()))
        return unique

    def save(self) -> None:
        self._path.write_text(
            json.dumps(asdict(self.data), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    @property
    def notes_dir(self) -> Path:
        return Path(self.data.notes_dir).expanduser()

    @property
    def note_folders(self) -> list[Path]:
        return [Path(folder).expanduser() for folder in self.data.note_folders or []]

    def set_notes_dir(self, path: Path) -> None:
        normalised = str(path.expanduser())
        folders = self._normalise_folders(self.data.note_folders, normalised)
        if normalised not in folders:
            folders.append(normalised)
        self.data.note_folders = folders
        self.data.notes_dir = normalised
        self.save()

    def add_notes_dir(self, path: Path, activate: bool = False) -> None:
        normalised = str(path.expanduser())
        folders = self._normalise_folders(self.data.note_folders, normalised)
        if normalised not in folders:
            folders.append(normalised)
        self.data.note_folders = folders
        colors = dict(self.data.folder_colors or {})
        colors.setdefault(normalised, f"preset-{len(folders) % len(PRESET_FOLDER_COLORS)}")
        self.data.folder_colors = colors
        if activate:
            self.data.notes_dir = normalised
        elif not self.data.notes_dir:
            self.data.notes_dir = folders[0]
        self.save()

    def remove_notes_dir(self, path: Path) -> None:
        normalised = str(path.expanduser())
        folders = [folder for folder in self._normalise_folders(self.data.note_folders, normalised) if folder != normalised]
        if not folders:
            return
        self.data.note_folders = folders
        colors = dict(self.data.folder_colors or {})
        colors.pop(normalised, None)
        self.data.folder_colors = colors
        if self.data.notes_dir == normalised:
            self.data.notes_dir = folders[0]
        self.save()

    def reorder_notes_dirs(self, folders: list[Path]) -> None:
        ordered = [str(folder.expanduser()) for folder in folders]
        current = self._normalise_folders(self.data.note_folders, self.data.notes_dir)
        if len(ordered) != len(current) or set(ordered) != set(current):
            return
        self.data.note_folders = ordered
        self.save()

    def is_note_pinned(self, path: Path) -> bool:
        normalised = str(path.expanduser())
        return normalised in (self.data.pinned_notes or [])

    def set_note_pinned(self, path: Path, pinned: bool) -> None:
        normalised = str(path.expanduser())
        pins = list(self.data.pinned_notes or [])
        if pinned:
            if normalised not in pins:
                pins.append(normalised)
        else:
            pins = [pin for pin in pins if pin != normalised]
        self.data.pinned_notes = pins
        self.save()

    def rename_pinned_note(self, old_path: Path, new_path: Path) -> None:
        old_key = str(old_path.expanduser())
        new_key = str(new_path.expanduser())
        pins = [new_key if pin == old_key else pin for pin in (self.data.pinned_notes or [])]
        self.data.pinned_notes = pins
        self.save()

    def delete_pinned_note(self, path: Path) -> None:
        normalised = str(path.expanduser())
        pins = [pin for pin in (self.data.pinned_notes or []) if pin != normalised]
        self.data.pinned_notes = pins
        self.save()

    def get_folder_color(self, path: Path) -> str:
        normalised = str(path.expanduser())
        colors = self.data.folder_colors or {}
        return normalize_folder_color(colors.get(normalised, DEFAULT_FOLDER_COLOR))

    def set_folder_color(self, path: Path, color: str) -> None:
        normalised = str(path.expanduser())
        colors = dict(self.data.folder_colors or {})
        colors[normalised] = normalize_folder_color(color)
        self.data.folder_colors = colors
        self.save()

    def rename_folder(self, old_path: Path, new_path: Path) -> None:
        old_key = str(old_path.expanduser())
        new_key = str(new_path.expanduser())
        folders = [new_key if folder == old_key else folder for folder in self._normalise_folders(self.data.note_folders, new_key)]
        self.data.note_folders = folders
        if self.data.notes_dir == old_key:
            self.data.notes_dir = new_key

        colors = dict(self.data.folder_colors or {})
        if old_key in colors:
            colors[new_key] = colors.pop(old_key)
        self.data.folder_colors = colors

        overrides = dict(self.data.note_language_overrides or {})
        updated_overrides: dict[str, str] = {}
        old_prefix = f"{old_key.rstrip('/')}/"
        for key, value in overrides.items():
            if key == old_key:
                updated_overrides[new_key] = value
            elif key.startswith(old_prefix):
                suffix = key[len(old_prefix):]
                updated_overrides[f"{new_key}/{suffix}"] = value
            else:
                updated_overrides[key] = value
        self.data.note_language_overrides = updated_overrides
        self.save()

    def get_language_override(self, path: Path) -> str | None:
        overrides = self.data.note_language_overrides or {}
        return overrides.get(str(path))

    def set_language_override(self, path: Path, language_id: str | None) -> None:
        overrides = dict(self.data.note_language_overrides or {})
        key = str(path)
        if language_id:
            overrides[key] = language_id
        else:
            overrides.pop(key, None)
        self.data.note_language_overrides = overrides
        self.save()

    def rename_language_override(self, old_path: Path, new_path: Path) -> None:
        overrides = dict(self.data.note_language_overrides or {})
        old_key = str(old_path)
        if old_key in overrides:
            overrides[str(new_path)] = overrides.pop(old_key)
            self.data.note_language_overrides = overrides
            self.save()

    def delete_language_override(self, path: Path) -> None:
        overrides = dict(self.data.note_language_overrides or {})
        if overrides.pop(str(path), None) is not None:
            self.data.note_language_overrides = overrides
            self.save()
