from __future__ import annotations

import unittest

from papertrail.folder_colors import (
    DEFAULT_FOLDER_COLOR,
    custom_folder_colors,
    folder_badge_text,
    folder_color_css_class,
    is_custom_folder_color,
    normalize_folder_color,
)


class FolderColorsTest(unittest.TestCase):
    def test_normalize_folder_color_clamps_preset_indices(self) -> None:
        self.assertEqual(normalize_folder_color(-1), "preset-0")
        self.assertEqual(normalize_folder_color(999), "preset-11")

    def test_normalize_folder_color_expands_short_hex(self) -> None:
        self.assertEqual(normalize_folder_color(" #AbC "), "#aabbcc")

    def test_normalize_folder_color_rejects_invalid_values(self) -> None:
        self.assertEqual(normalize_folder_color("preset-nope"), DEFAULT_FOLDER_COLOR)
        self.assertEqual(normalize_folder_color("not-a-color"), DEFAULT_FOLDER_COLOR)

    def test_custom_color_helpers(self) -> None:
        self.assertTrue(is_custom_folder_color("#123456"))
        self.assertFalse(is_custom_folder_color("preset-3"))
        self.assertEqual(
            folder_color_css_class("#123456"),
            folder_color_css_class("#123456"),
        )
        self.assertEqual(folder_color_css_class("preset-3"), "folder-color-3")

    def test_folder_badge_text_prefers_two_words(self) -> None:
        self.assertEqual(folder_badge_text("Paper Trail"), "PT")
        self.assertEqual(folder_badge_text("project_notes"), "PN")
        self.assertEqual(folder_badge_text("42"), "42")
        self.assertEqual(folder_badge_text("---"), "F")

    def test_custom_folder_colors_returns_only_hex_values(self) -> None:
        colors = custom_folder_colors(["preset-1", "#abcdef", "#ABCDEF", "invalid"])
        self.assertEqual(colors, {"#abcdef"})


if __name__ == "__main__":
    unittest.main()
