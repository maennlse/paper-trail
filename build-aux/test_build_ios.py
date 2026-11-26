"""Unit tests for unsigned iOS build planning; no Xcode or note files required."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build_ios


class IOSBuildTests(unittest.TestCase):
    """Exercise build plans without invoking Cargo or Apple tooling."""

    def test_app_icon_is_wired_to_the_asset_catalog(self) -> None:
        """Xcode must compile the checked-in universal icon into the iOS app."""
        root = Path(__file__).resolve().parent.parent
        assets = root / "ios/Resources/Assets.xcassets"
        manifest = json.loads(
            (assets / "AppIcon.appiconset/Contents.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            manifest["images"],
            [
                {
                    "filename": "AppIcon.png",
                    "idiom": "universal",
                    "platform": "ios",
                    "size": "1024x1024",
                }
            ],
        )
        project = (root / "ios/project.yml").read_text(encoding="utf-8")
        self.assertIn("ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon", project)
        self.assertIn("      - Resources\n", project)

    def test_app_icon_is_opaque_rgb_at_full_resolution(self) -> None:
        """iOS icons require a square 1024px image with no alpha/transparency."""
        icon = (
            Path(__file__).resolve().parent.parent
            / "ios/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
        ).read_bytes()
        self.assertEqual(icon[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(icon[12:16], b"IHDR")
        self.assertEqual(
            struct.unpack(">IIBBBBB", icon[16:29]), (1024, 1024, 8, 2, 0, 0, 0)
        )
        offset = 8
        chunks = []
        while offset < len(icon):
            size = int.from_bytes(icon[offset : offset + 4], "big")
            chunks.append(icon[offset + 4 : offset + 8])
            offset += size + 12
        self.assertIn(b"IDAT", chunks)
        self.assertEqual(chunks[-1], b"IEND")
        self.assertNotIn(b"tRNS", chunks)

    def test_dry_run_creates_nothing_and_disables_signing(self) -> None:
        """A build preview is non-mutating and never requests provisioning."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                build_ios.build(root, root / "dist", dry_run=True)
            self.assertEqual(list(root.iterdir()), [])
            commands = output.getvalue()
            self.assertIn("aarch64-apple-ios-sim", commands)
            self.assertIn("CODE_SIGNING_ALLOWED=NO", commands)
            self.assertIn("ARCHS=arm64", commands)
            self.assertNotIn("-allowProvisioningUpdates", commands)

    def test_linux_fails_before_writing_outputs(self) -> None:
        """Unsupported hosts fail before creating build or artifact directories."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (
                patch("build_ios.platform.system", return_value="Linux"),
                self.assertRaisesRegex(ValueError, "requires an Apple Silicon Mac"),
            ):
                build_ios.build(root, root / "dist")
            self.assertEqual(list(root.iterdir()), [])

    def test_explicit_rust_target_and_cache(self) -> None:
        """User Cargo defaults must not redirect the library search paths."""
        command = build_ios.cargo_command(
            Path("/repo"), "build", "--target", "aarch64-apple-ios"
        )
        self.assertIn("--locked", command)
        self.assertEqual(command[command.index("--target-dir") + 1], "/repo/target/ios")

    def test_packaging_preserves_outputs_and_checksums_both_artifacts(self) -> None:
        """Only the new unsigned bundle copies enter the package."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            products = root / "ios/build/DerivedData/Build/Products"
            for sdk in build_ios.TARGETS:
                app = products / f"Release-{sdk}/PaperTrail.app"
                app.mkdir(parents=True)
                (app / "PaperTrail").write_bytes(b"executable")
            output = root / "dist"
            output.mkdir()
            previous = output / "previous.ipa"
            previous.write_bytes(b"keep")

            def fake_ditto(command: list[str], **_: object) -> None:
                source = Path(command[-2])
                self.assertTrue(source.exists())
                if source.name == "Payload":
                    self.assertTrue((source / "PaperTrail.app/PaperTrail").is_file())
                Path(command[-1]).write_bytes(b"unsigned archive")

            with patch("build_ios.subprocess.run", side_effect=fake_ditto):
                destination = build_ios.package_artifacts(root, output)
            self.assertEqual(previous.read_bytes(), b"keep")
            self.assertEqual(len(list(destination.glob("*.ipa"))), 1)
            digest = hashlib.sha256(b"unsigned archive").hexdigest()
            checksums = (destination / "SHA256SUMS").read_text()
            self.assertEqual(checksums.count(digest), 2)

    def test_packaging_rejects_existing_signing_material(self) -> None:
        """A developer's old signed build must never leak into an unsigned artifact."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            app = (
                root
                / "ios/build/DerivedData/Build/Products/Release-iphoneos/PaperTrail.app"
            )
            app.mkdir(parents=True)
            (app / "embedded.mobileprovision").write_bytes(b"private profile")
            with self.assertRaisesRegex(ValueError, "previously signed"):
                build_ios.package_artifacts(root, root / "dist")
            self.assertFalse((root / "dist").exists())


if __name__ == "__main__":
    unittest.main()
