#!/usr/bin/env python3
"""Offline coverage of interactive local builds and isolated artifact packaging."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build_local
from capture_screenshots import demo
from package_desktop import package as package_desktop


class LocalBuildTests(unittest.TestCase):
    """No test runs Cargo, installs dependencies, publishes, or reads user data."""

    def setUp(self) -> None:
        # pylint: disable-next=consider-using-with
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.enterContext(contextlib.redirect_stdout(io.StringIO()))
        self.errors = self.enterContext(contextlib.redirect_stderr(io.StringIO()))

    def build(self, mode: str = "native") -> build_local.Build:
        """Use explicit fixture directories, never the repository's real target/dist."""
        target = (
            "x86_64-unknown-linux-gnu"
            if mode == "native"
            else "x86_64-unknown-linux-musl"
        )
        return build_local.Build(
            self.root,
            "x86_64-unknown-linux-gnu",
            target,
            mode,
            ("tui", "server"),
            self.root / "dist/local",
            self.root / "target",
        )

    def binaries(self, build: build_local.Build) -> None:
        """Only selected binary/doc files may enter archives, not adjacent secrets."""
        (self.root / "docs").mkdir()
        for component in build.components:
            binary = build.binary(component)
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b"synthetic executable")
            binary.chmod(0o755)
            (self.root / "docs" / build_local.COMPONENTS[component][1]).write_text(
                f"Public {component} documentation", encoding="utf-8"
            )
        (self.root / "introspection-secret").write_text("PRIVATE", encoding="utf-8")
        (self.root / "papertrail.sqlite3").write_bytes(b"PRIVATE DATABASE")

    def test_interactive_choices_require_confirmation_and_cancel_without_writes(
        self,
    ) -> None:
        """Declining the final prompt cannot create an output directory."""
        output = self.root / "artifacts"
        with (
            patch("build_local.host_target", return_value="x86_64-unknown-linux-gnu"),
            patch("build_local.sys.stdin.isatty", return_value=True),
            patch(
                "builtins.input", side_effect=["native", "tui,server", str(output), "n"]
            ) as prompt,
            patch("build_local.execute") as execute,
        ):
            self.assertEqual(build_local.main([]), 0)
        self.assertEqual(prompt.call_count, 4)
        execute.assert_not_called()
        self.assertFalse(output.exists())

    def test_approved_interactive_choices_build_selected_components(self) -> None:
        """Only an explicit yes reaches the build executor."""
        with (
            patch("build_local.host_target", return_value="aarch64-apple-darwin"),
            patch("build_local.sys.stdin.isatty", return_value=True),
            patch(
                "builtins.input",
                side_effect=["native", "server", str(self.root), "yes"],
            ),
            patch("build_local.execute") as execute,
        ):
            self.assertEqual(build_local.main([]), 0)
        self.assertEqual(execute.call_args.args[0].components, ("server",))

    def test_dry_run_is_unattended_and_creates_no_output(self) -> None:
        """Preview works without a TTY or any filesystem mutation."""
        output = self.root / "artifacts"
        with (
            patch("build_local.host_target", return_value="x86_64-unknown-linux-gnu"),
            patch("builtins.input", side_effect=AssertionError("Unexpected prompt")),
            patch("build_local.execute") as execute,
        ):
            self.assertEqual(
                build_local.main(
                    [
                        "--components",
                        "tui,server",
                        "--mode",
                        "standalone",
                        "--output-dir",
                        str(output),
                        "--dry-run",
                    ]
                ),
                0,
            )
        execute.assert_not_called()
        self.assertFalse(output.exists())

    def test_noninteractive_flags_do_not_prompt(self) -> None:
        """Automation never blocks on component or confirmation prompts."""
        with (
            patch("build_local.host_target", return_value="x86_64-pc-windows-msvc"),
            patch("builtins.input", side_effect=AssertionError("Unexpected prompt")),
            patch("build_local.execute") as execute,
        ):
            self.assertEqual(
                build_local.main(["--components", "server", "--non-interactive"]), 0
            )
        self.assertEqual(execute.call_args.args[0].components, ("server",))

    def test_redirected_input_without_explicit_flags_fails_clearly(self) -> None:
        """Piped input must not accidentally approve a build."""
        with (
            patch("build_local.host_target", return_value="x86_64-unknown-linux-gnu"),
            patch("build_local.sys.stdin.isatty", return_value=False),
        ):
            self.assertEqual(build_local.main([]), 1)
        self.assertIn("--non-interactive", self.errors.getvalue())

    def test_platform_defaults_and_standalone_target_selection(self) -> None:
        """Component and standalone target choices follow the host platform."""
        for host, expected in [
            ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"),
            ("aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"),
            ("aarch64-apple-darwin", "aarch64-apple-darwin"),
            ("aarch64-pc-windows-msvc", "aarch64-pc-windows-msvc"),
        ]:
            self.assertEqual(build_local.standalone_target(host), expected)
            self.assertEqual(
                build_local.available(host, "standalone"), ("tui", "server")
            )
        self.assertIn(
            "gtk", build_local.available("x86_64-unknown-linux-gnu", "native")
        )
        self.assertNotIn(
            "gtk", build_local.available("x86_64-pc-windows-msvc", "native")
        )

    def test_invalid_components_and_cross_platform_targets_are_rejected(self) -> None:
        """Smoke-test execution requires the host OS and CPU architecture."""
        for arguments in [
            ["--components", "unknown"],
            ["--components", "gtk", "--mode", "standalone"],
            ["--target", "aarch64-apple-darwin"],
        ]:
            args = build_local.parser().parse_args([*arguments, "--non-interactive"])
            with self.assertRaises(ValueError):
                build_local.choices(args, self.root, "x86_64-unknown-linux-gnu")

    def test_custom_cargo_target_directory_is_honored(self) -> None:
        """Cargo caches with spaces stay one argument and determine binary paths."""
        args = build_local.parser().parse_args(
            ["--components", "server", "--non-interactive"]
        )
        with patch.dict(
            "build_local.os.environ", {"CARGO_TARGET_DIR": "cache with spaces"}
        ):
            build = build_local.choices(args, self.root, "x86_64-unknown-linux-gnu")
        self.assertEqual(build.target_dir, self.root / "cache with spaces")
        self.assertIn("--locked", build.command())
        self.assertIn(str(build.target_dir), build.command())
        self.assertEqual(
            build.binary("server"),
            build.target_dir / build.target / "release/paper-trail-server",
        )

    def test_standalone_flags_preserve_user_flags_without_changing_parent_environment(
        self,
    ) -> None:
        """Static CRT flags affect only child builds, preserving encoded user flags."""
        with patch.dict(
            "build_local.os.environ",
            {"CARGO_ENCODED_RUSTFLAGS": "-C\x1fdebuginfo=0", "CC": "cc"},
            clear=True,
        ):
            env = build_local.environment(self.build("standalone"))
            self.assertEqual(
                env["CARGO_ENCODED_RUSTFLAGS"],
                "-C\x1fdebuginfo=0\x1f-C\x1ftarget-feature=+crt-static",
            )
            self.assertEqual(env["CC"], "musl-gcc")
            self.assertEqual(env["AWS_LC_SYS_USE_SYSTEM"], "0")
            self.assertEqual(build_local.os.environ["CC"], "cc")
            self.assertEqual(build_local.environment(self.build())["CC"], "cc")

    def test_missing_target_or_tool_does_not_create_output(self) -> None:
        """Missing prerequisites are reported before output is reserved."""
        build = self.build("standalone")
        with (
            patch(
                "build_local.subprocess.check_output",
                return_value=str(self.root / "missing-stdlib"),
            ),
            self.assertRaisesRegex(ValueError, "rustup target add"),
        ):
            build_local.execute(build)
        self.assertFalse(build.output.exists())
        with (
            patch("build_local.shutil.which", return_value=None),
            self.assertRaisesRegex(ValueError, "musl-gcc"),
        ):
            build_local.require("musl-gcc")

    def test_native_archives_are_labeled_and_exclude_user_data(self) -> None:
        """Native artifacts are labeled, checksummed, isolated and allowlisted."""
        build = self.build()
        self.binaries(build)
        with patch("build_local.preflight"), patch("build_local.subprocess.run") as run:
            first = build_local.execute(build)
            second = build_local.execute(build)
        self.assertNotEqual(first, second)
        self.assertTrue(first.is_dir())
        self.assertEqual(run.call_args_list[0].args[0], build.command())
        self.assertEqual(len(list(first.iterdir())), 5)
        for archive in first.glob("*.tar.gz"):
            self.assertIn("-native.tar.gz", archive.name)
            with tarfile.open(archive) as bundle:
                self.assertEqual(len(bundle.getnames()), 3)
                self.assertIn("README.md", bundle.getnames())
                info = bundle.extractfile("BUILD-INFO.txt").read()
                self.assertIn(b"not a verified standalone/static release", info)
                self.assertNotIn(b"PRIVATE", info)
        for line in (first / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
            digest, name = line.split()
            self.assertEqual(
                hashlib.sha256((first / name).read_bytes()).hexdigest(), digest
            )

    def test_standalone_archives_reuse_existing_linkage_verification(self) -> None:
        """Release-like archives retain strict linkage verification."""
        build = self.build("standalone")
        self.binaries(build)
        with (
            patch("build_local.preflight"),
            patch("build_local.subprocess.run"),
            patch("package_tui.verify") as verify,
        ):
            output = build_local.execute(build)
        self.assertEqual(verify.call_count, 2)
        verify.assert_any_call(
            build.binary("server"), build.target, check_version=False
        )
        for archive in output.glob("*.tar.gz"):
            self.assertNotIn("-native", archive.name)
            with tarfile.open(archive) as bundle:
                self.assertNotIn("BUILD-INFO.txt", bundle.getnames())

    def test_desktop_packager_uses_the_selected_binary_and_output_directory(
        self,
    ) -> None:
        """Explicit target/cache paths must not package a stale default GTK binary."""
        binary = self.root / "custom-cache/x86_64-unknown-linux-gnu/release/paper-trail"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"\x7fELF\x02\x01" + bytes(12) + b"\x3e\x00")
        docs = self.root / "docs"
        docs.mkdir()
        (docs / "desktop-release.md").write_text("Public GTK docs", encoding="utf-8")
        distribution = self.root / "os-release"
        distribution.write_text('NAME="Local test"\n', encoding="utf-8")
        output = self.root / "local-artifacts"
        libraries = "libgtk-4.so.1\nlibadwaita-1.so.0\nlibgtksourceview-5.so.0\n"
        with patch(
            "package_desktop.subprocess.check_output",
            side_effect=[libraries, "versions"],
        ):
            archive = package_desktop(
                self.root, distribution, binary=binary, destination=output
            )
        self.assertEqual(archive.parent, output)
        self.assertFalse((self.root / "dist").exists())

    def test_screenshot_demo_is_isolated_and_has_no_provider_credentials(self) -> None:
        """Screenshot data is synthetic and cannot inherit real Paper Trail settings."""
        with patch.dict(
            "os.environ",
            {
                "PAPERTRAIL_OIDC_ISSUER": "PRIVATE_ISSUER",
                "WAYLAND_DISPLAY": "real-display",
            },
        ):
            config, env = demo(self.root)
        self.assertEqual(env["HOME"], str(self.root))
        self.assertEqual(env["GDK_BACKEND"], "x11")
        self.assertNotIn("WAYLAND_DISPLAY", env)
        self.assertNotIn("PAPERTRAIL_OIDC_ISSUER", env)
        settings = json.loads(config.read_text(encoding="utf-8"))
        self.assertEqual(len(settings["note_folders"]), 3)
        for folder in settings["note_folders"]:
            self.assertTrue(Path(folder).is_relative_to(self.root))
        sync = json.loads(
            (config.parent / "sync/config.json").read_text(encoding="utf-8")
        )
        self.assertEqual(sync["server"], "https://sync.example.com")
        self.assertFalse(any("secret" in key or "token" in key for key in sync))

    def test_archive_inputs_cannot_recursively_include_directories(self) -> None:
        """A bad binary path must fail before archiving unrelated files."""
        documentation = self.root / "README.md"
        documentation.write_text("Public", encoding="utf-8")
        output = self.root / "artifacts"
        with self.assertRaises(ValueError):
            build_local.create_archive(self.root, documentation, output, "invalid")
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
