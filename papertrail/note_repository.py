# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
import itertools
import uuid


@dataclass(slots=True)
class NoteRecord:
    path: Path
    title: str
    preview: str
    body: str
    modified_at: datetime


class NoteRepository:
    def __init__(self, notes_dir: Path) -> None:
        self.set_notes_dir(notes_dir)

    def set_notes_dir(self, notes_dir: Path) -> None:
        self._notes_dir = notes_dir.expanduser()
        self._notes_dir.mkdir(parents=True, exist_ok=True)

    @property
    def notes_dir(self) -> Path:
        return self._notes_dir

    def list_notes(self) -> list[NoteRecord]:
        notes: list[NoteRecord] = []
        for path in sorted(self._notes_dir.iterdir()):
            if path.is_file():
                notes.append(self.load_note(path))

        notes.sort(key=lambda note: note.modified_at, reverse=True)
        return notes

    def load_note(self, path: Path) -> NoteRecord:
        body = path.read_text(encoding="utf-8", errors="replace") if path.exists() else ""
        stat = path.stat()
        title, preview = self._summarize(path, body)
        return NoteRecord(
            path=path,
            title=title,
            preview=preview,
            body=body,
            modified_at=datetime.fromtimestamp(stat.st_mtime),
        )

    def create_note(self) -> NoteRecord:
        stem = datetime.now().strftime("%Y%m%d-%H%M%S")
        for suffix in itertools.chain([""], [f"-{index}" for index in range(1, 100)]):
            path = self._notes_dir / f"{stem}{suffix}.txt"
            if not path.exists():
                path.write_text("", encoding="utf-8")
                return self.load_note(path)

        path = self._notes_dir / f"{stem}-{uuid.uuid4().hex[:8]}.txt"
        path.write_text("", encoding="utf-8")
        return self.load_note(path)

    def save_note(self, path: Path, body: str) -> NoteRecord:
        path.write_text(body, encoding="utf-8")
        return self.load_note(path)

    def rename_note(self, path: Path, requested_name: str) -> NoteRecord:
        target_name = self._normalise_filename(requested_name)
        target_path = self._unique_path(path.parent, target_name, path)
        if target_path != path:
            path.rename(target_path)
        return self.load_note(target_path)

    def move_note(self, path: Path, folder: Path) -> NoteRecord:
        target_folder = folder.expanduser()
        target_folder.mkdir(parents=True, exist_ok=True)
        target_path = self._unique_path(target_folder, path.name, path)
        if target_path != path:
            path.rename(target_path)
        return self.load_note(target_path)

    def delete_note(self, path: Path) -> None:
        if path.exists():
            path.unlink()

    def _normalise_filename(self, requested_name: str) -> str:
        cleaned = requested_name.strip().replace("/", "-").replace("\\", "-")
        cleaned = "".join(ch for ch in cleaned if ch not in '<>:"|?*').strip().rstrip(".")
        if not cleaned:
            cleaned = "untitled-note"
        return cleaned

    def _unique_path(self, folder: Path, filename: str, current_path: Path | None = None) -> Path:
        candidate = folder / filename
        if current_path is not None and candidate == current_path:
            return candidate
        if not candidate.exists():
            return candidate

        stem = candidate.stem
        suffix = candidate.suffix
        for index in range(1, 1000):
            numbered = folder / f"{stem}-{index}{suffix}"
            if current_path is not None and numbered == current_path:
                return numbered
            if not numbered.exists():
                return numbered

        return folder / f"{stem}-{uuid.uuid4().hex[:8]}{suffix}"

    def _summarize(self, path: Path, body: str) -> tuple[str, str]:
        lines = body.splitlines()
        title = lines[0].strip() if lines and lines[0].strip() else path.stem.replace("-", " ")

        preview_source = [line.rstrip() for line in lines[1:]] if lines else body.splitlines()
        while preview_source and not preview_source[0].strip():
            preview_source.pop(0)

        preview = "\n".join(preview_source[:5]).strip()
        if not preview:
            preview = "Plain text note"
        preview = preview[:500].rstrip()
        if len(preview) == 500:
            preview += "…"

        return title[:120], preview
