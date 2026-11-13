#!/usr/bin/env python3
"""Offline tests for release validation and standalone/native archive contents."""

from __future__ import annotations

import hashlib
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

from package_desktop import package
from package_server import package as package_server
from package_tui import verify
from prepare_release import NATIVE_TARGETS, PACKAGES, collect, validate_tag


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
            patch(
                "package_tui.windows_imports",
                return_value=["bcryptprimitives.dll", "combase.dll"],
            ),
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
        self.assertEqual(len(PACKAGES), 14)
        self.assertEqual(len(list(destination.iterdir())), 29)
        self.assertFalse((destination / "debug.png").exists())
        lines = (destination / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(lines), 14)
        for line in lines:
            digest, name = line.split()
            self.assertEqual(
                hashlib.sha256((destination / name).read_bytes()).hexdigest(), digest
            )

    def test_missing_package_prevents_collection(self) -> None:
        """No publication stage is created when one target is missing."""
        source = self.artifacts()
        for name in PACKAGES:
            with self.subTest(package=name):
                path = next(source.rglob(name))
                data = path.read_bytes()
                path.unlink()
                destination = self.root / "release"
                with self.assertRaises(ValueError):
                    collect(source, destination)
                self.assertFalse(destination.exists())
                path.write_bytes(data)

    def test_corrupt_package_prevents_collection(self) -> None:
        """Checksums must verify before staging any files."""
        source = self.artifacts()
        for name in PACKAGES:
            with self.subTest(package=name):
                path = next(source.rglob(name))
                data = path.read_bytes()
                path.write_bytes(b"corrupt")
                with self.assertRaises(ValueError):
                    collect(source, self.root / "release")
                self.assertFalse((self.root / "release").exists())
                path.write_bytes(data)

    def test_server_and_tui_workflows_cover_the_same_six_release_targets(self) -> None:
        """A new target cannot be published without matching both native workflows."""
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(len(NATIVE_TARGETS), 6)
        for component in ("tui", "server"):
            workflow = (root / f".github/workflows/{component}.yml").read_text(
                encoding="utf-8"
            )
            self.assertEqual(workflow.count("target: "), 6)
            for target in NATIVE_TARGETS:
                with self.subTest(component=component, target=target):
                    self.assertEqual(workflow.count(f"target: {target}\n"), 1)
                    extension = "zip" if "windows" in target else "tar.gz"
                    self.assertIn(
                        f"paper-trail-{component}-{target}.{extension}", PACKAGES
                    )

    def test_server_archives_contain_only_the_binary_and_public_documentation(
        self,
    ) -> None:
        """All six archives exclude database, config, keys and local credentials."""
        docs = self.root / "docs"
        docs.mkdir()
        (docs / "server-release.md").write_text("Server deployment", encoding="utf-8")
        (self.root / "papertrail.sqlite3").write_bytes(b"DATABASE_PRIVATE")
        (self.root / "introspection-secret").write_bytes(b"CREDENTIAL_PRIVATE")
        for target in NATIVE_TARGETS:
            with self.subTest(target=target):
                executable = "paper-trail-server" + (
                    ".exe" if "windows" in target else ""
                )
                binary = self.root / "target" / target / "release" / executable
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"synthetic executable")
                binary.chmod(0o755)
                with patch("package_tui.verify") as check:
                    archive = package_server(self.root, target)
                check.assert_called_once_with(binary, target, check_version=False)
                if "windows" in target:
                    with zipfile.ZipFile(archive) as bundle:
                        self.assertEqual(bundle.namelist(), [executable, "README.md"])
                        self.assertEqual(bundle.read("README.md"), b"Server deployment")
                else:
                    with tarfile.open(archive) as bundle:
                        self.assertEqual(bundle.getnames(), [executable, "README.md"])
                        self.assertTrue(bundle.getmember(executable).mode & 0o111)
                        self.assertEqual(
                            bundle.extractfile("README.md").read(), b"Server deployment"
                        )
                sidecar = archive.with_name(f"{archive.name}.sha256")
                self.assertEqual(
                    sidecar.read_text(encoding="utf-8"),
                    f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n",
                )

    def test_server_smoke_test_only_requests_help(self) -> None:
        """Packaging cannot start a server or require deployment credentials."""
        binary = self.root / "paper-trail-server"
        for target in NATIVE_TARGETS:
            with self.subTest(target=target):
                with (
                    patch("package_tui.windows_imports", return_value=["bcrypt.dll"]),
                    patch(
                        "package_tui.subprocess.check_output",
                        return_value="No dynamic dependencies\n",
                    ),
                    patch("package_tui.subprocess.run") as run,
                ):
                    verify(binary, target, check_version=False)
                run.assert_called_once()
                self.assertEqual(run.call_args.args[0], [str(binary), "--help"])

    def test_linux_artifacts_must_be_fully_static(self) -> None:
        """Reject an interpreter or dynamic library even if the executable runs."""
        for headers, dynamic in [("INTERP", ""), ("", "NEEDED libcrypto.so")]:
            with self.subTest(headers=headers, dynamic=dynamic):
                with (
                    patch(
                        "package_tui.subprocess.check_output",
                        side_effect=[headers, dynamic],
                    ),
                    patch("package_tui.subprocess.run") as run,
                    self.assertRaises(ValueError),
                ):
                    verify(self.root / "server", "x86_64-unknown-linux-musl")
                run.assert_not_called()

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
