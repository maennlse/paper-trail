#!/usr/bin/env python3
"""Offline tests for release validation and native GTK archive contents."""

from __future__ import annotations

import hashlib
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from package_desktop import package
from package_tui import verify
from prepare_release import PACKAGES, collect, validate_tag


class ReleaseTests(unittest.TestCase):
    """Publishing must never receive an incomplete or mislabeled asset set."""

    def setUp(self) -> None:
        # unittest owns the context until test cleanup, rather than setUp return.
        # pylint: disable-next=consider-using-with
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def artifacts(self) -> Path:
        """Create synthetic artifacts; no network, real notes, or compiled code."""
        source = self.root / "artifacts"
        for index, name in enumerate(PACKAGES):
            folder = source / str(index)
            folder.mkdir(parents=True)
            data = f"synthetic package {index}".encode()
            (folder / name).write_bytes(data)
            digest = hashlib.sha256(data).hexdigest()
            (folder / f"{name}.sha256").write_text(
                f"{digest}  {name}\n", encoding="utf-8"
            )
        return source

    def test_tag_must_match_workspace_version(self) -> None:
        """Stable and prerelease tags must match exactly; no shell evaluation."""
        manifest = self.root / "Cargo.toml"
        for version in ("1.2.3", "1.2.3-rc.1"):
            manifest.write_text(
                f'[workspace.package]\nversion = "{version}"\n', encoding="utf-8"
            )
            validate_tag(f"v{version}", manifest)
            for wrong in ("v9.0.0", version, "v$(touch unwanted)"):
                with self.assertRaises(ValueError):
                    validate_tag(wrong, manifest)

    def test_windows_allows_system_crypto_but_rejects_dynamic_crt(self) -> None:
        """System randomness DLLs are allowed, but the C runtime stays static."""
        binary = self.root / "paper-trail-tui.exe"
        with (
            patch("package_tui.windows_imports", return_value=["bcryptprimitives.dll"]),
            patch("package_tui.subprocess.run") as run,
        ):
            verify(binary, "x86_64-pc-windows-msvc")
            self.assertEqual(run.call_count, 2)
        for library in ("vcruntime140.dll", "msvcp140.dll", "third-party.dll"):
            with (
                patch("package_tui.windows_imports", return_value=[library]),
                self.assertRaises(ValueError),
            ):
                verify(binary, "aarch64-pc-windows-msvc")

    def test_collect_only_expected_assets_and_checksum_manifest(self) -> None:
        """Debug binaries/screenshots are not accidentally published."""
        source = self.artifacts()
        (source / "debug.png").write_bytes(b"not a release asset")
        destination = self.root / "release"
        collect(source, destination)
        self.assertEqual(len(list(destination.iterdir())), 17)
        self.assertFalse((destination / "debug.png").exists())
        lines = (destination / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(lines), 8)
        for line in lines:
            digest, name = line.split()
            self.assertEqual(
                hashlib.sha256((destination / name).read_bytes()).hexdigest(), digest
            )

    def test_missing_package_prevents_collection(self) -> None:
        """No publication stage is created when one target is missing."""
        source = self.artifacts()
        next(source.rglob(PACKAGES[0])).unlink()
        destination = self.root / "release"
        with self.assertRaises(ValueError):
            collect(source, destination)
        self.assertFalse(destination.exists())

    def test_corrupt_package_prevents_collection(self) -> None:
        """Checksums must verify before staging any files."""
        source = self.artifacts()
        next(source.rglob(PACKAGES[0])).write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            collect(source, self.root / "release")
        self.assertFalse((self.root / "release").exists())

    def test_duplicate_package_prevents_collection(self) -> None:
        """Artifact flattening must not silently overwrite an asset."""
        source = self.artifacts()
        (source / PACKAGES[0]).write_bytes(b"duplicate")
        with self.assertRaises(ValueError):
            collect(source, self.root / "release")

    def test_checksum_path_traversal_is_rejected(self) -> None:
        """Checksum filenames cannot redirect staging outside the output folder."""
        source = self.artifacts()
        path = next(source.rglob(f"{PACKAGES[0]}.sha256"))
        path.write_text("0" * 64 + "  ../unwanted\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            collect(source, self.root / "release")

    def test_existing_destination_is_not_overwritten(self) -> None:
        """Never delete or replace unrelated local files."""
        source = self.artifacts()
        destination = self.root / "release"
        destination.mkdir()
        (destination / "keep").write_text("keep", encoding="utf-8")
        with self.assertRaises(ValueError):
            collect(source, destination)
        self.assertEqual((destination / "keep").read_text(encoding="utf-8"), "keep")

    def test_native_gtk_archive_has_executable_documentation_and_build_info(
        self,
    ) -> None:
        """The dynamic GTK download is explicit about its system dependencies."""
        binary = self.root / "target/release/paper-trail"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"\x7fELF\x02\x01" + bytes(12) + b"\x3e\x00")
        binary.chmod(0o755)
        docs = self.root / "docs"
        docs.mkdir()
        (docs / "desktop-release.md").write_text("GTK requirements", encoding="utf-8")
        distribution = self.root / "os-release"
        distribution.write_text('NAME="Test Linux"\n', encoding="utf-8")
        libraries = "libgtk-4.so.1\nlibadwaita-1.so.0\nlibgtksourceview-5.so.0\n"
        with patch(
            "package_desktop.subprocess.check_output",
            side_effect=[libraries, "versions"],
        ):
            archive = package(self.root, distribution)
        with tarfile.open(archive) as bundle:
            self.assertEqual(
                bundle.getnames(), ["paper-trail", "README.md", "BUILD-INFO.txt"]
            )
            self.assertTrue(bundle.getmember("paper-trail").mode & 0o111)
            info = bundle.extractfile("BUILD-INFO.txt")
            self.assertIsNotNone(info)
            self.assertIn(b"not a standalone/static binary", info.read())
        self.assertTrue(archive.with_suffix(".gz.sha256").exists())


if __name__ == "__main__":
    unittest.main()
