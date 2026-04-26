from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from papertrail.note_repository import NoteRepository


class NoteRepositoryTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmpdir.cleanup)
        self.notes_dir = Path(self._tmpdir.name)
        self.repository = NoteRepository(self.notes_dir)

    def test_load_note_uses_filename_when_first_line_is_blank(self) -> None:
        path = self.notes_dir / "my-note.txt"
        path.write_text("\n\nsecond line\n", encoding="utf-8")

        note = self.repository.load_note(path)

        self.assertEqual(note.title, "my note")
        self.assertEqual(note.preview, "second line")

    def test_load_note_uses_default_preview_for_empty_note(self) -> None:
        path = self.notes_dir / "empty.txt"
        path.write_text("", encoding="utf-8")

        note = self.repository.load_note(path)

        self.assertEqual(note.preview, "Plain text note")

    def test_list_notes_returns_newest_first(self) -> None:
        older = self.notes_dir / "older.txt"
        newer = self.notes_dir / "newer.txt"
        older.write_text("old", encoding="utf-8")
        newer.write_text("new", encoding="utf-8")
        older.touch()
        newer.touch()

        notes = self.repository.list_notes()

        self.assertEqual([note.path.name for note in notes], ["newer.txt", "older.txt"])

    def test_rename_note_sanitizes_filename_and_preserves_extensionless_name(self) -> None:
        path = self.notes_dir / "source.txt"
        path.write_text("hello", encoding="utf-8")

        renamed = self.repository.rename_note(path, ' bad:/\\name?.txt. ')

        self.assertEqual(renamed.path.name, "bad--name.txt")
        self.assertTrue(renamed.path.exists())

    def test_rename_note_resolves_name_collisions(self) -> None:
        path = self.notes_dir / "source.txt"
        path.write_text("hello", encoding="utf-8")
        (self.notes_dir / "taken.txt").write_text("existing", encoding="utf-8")

        renamed = self.repository.rename_note(path, "taken.txt")

        self.assertEqual(renamed.path.name, "taken-1.txt")

    def test_move_note_resolves_name_collisions(self) -> None:
        source = self.notes_dir / "note.txt"
        source.write_text("hello", encoding="utf-8")
        target_dir = self.notes_dir / "nested"
        target_dir.mkdir()
        (target_dir / "note.txt").write_text("existing", encoding="utf-8")

        moved = self.repository.move_note(source, target_dir)

        self.assertEqual(moved.path, target_dir / "note-1.txt")
        self.assertTrue(moved.path.exists())
        self.assertFalse(source.exists())

    def test_delete_note_removes_existing_file(self) -> None:
        path = self.notes_dir / "gone.txt"
        path.write_text("bye", encoding="utf-8")

        self.repository.delete_note(path)

        self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main()
