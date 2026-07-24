"""Tests for persisted settings normalization."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from papertrail.settings import Settings


class SettingsTest(unittest.TestCase):
    """Exercises recovery from malformed persisted settings."""

    def test_invalid_window_geometry_uses_safe_values(self) -> None:
        """Window geometry must not make GTK startup fail."""

        with tempfile.TemporaryDirectory() as tempdir:
            config_dir = Path(tempdir) / "paper-trail"
            config_dir.mkdir()
            (config_dir / "settings.json").write_text(
                json.dumps(
                    {
                        "notes_dir": 42,
                        "note_folders": "not-a-list",
                        "width": "not-a-number",
                        "height": 99_999,
                        "sidebar_width": False,
                        "wrap_text": "false",
                    }
                ),
                encoding="utf-8",
            )

            with patch("papertrail.settings.call_nullary", return_value=tempdir):
                settings = Settings()

        self.assertEqual(settings.data.width, 1320)
        self.assertEqual(settings.data.height, 4320)
        self.assertEqual(settings.data.sidebar_width, 260)
        self.assertTrue(settings.data.wrap_text)
        self.assertIsInstance(settings.data.notes_dir, str)
        self.assertIsInstance(settings.data.note_folders, list)


if __name__ == "__main__":
    unittest.main()
