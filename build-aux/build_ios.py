#!/usr/bin/env python3
"""Build unsigned iOS artifacts on macOS. Never sign, install tools, or publish."""

from __future__ import annotations

import argparse
import hashlib
import os
import platform
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

TARGETS = {
    "iphoneos": "aarch64-apple-ios",
    "iphonesimulator": "aarch64-apple-ios-sim",
}


def cargo_command(root: Path, *arguments: str) -> list[str]:
    """Use an explicit cache, independent of a user's default build target."""
    return [
        "cargo",
        *arguments,
        "--locked",
        "--target-dir",
        str(root / "target/ios"),
    ]


def xcode_command(root: Path, sdk: str, action: str = "build") -> list[str]:
    """Signing is explicitly disabled for both the app and its test bundles."""
    return [
        "xcodebuild",
        "-project",
        str(root / "ios/PaperTrail.xcodeproj"),
        "-scheme",
        "PaperTrail",
        "-configuration",
        "Release",
        "-sdk",
        sdk,
        "-destination",
        f"generic/platform={'iOS' if sdk == 'iphoneos' else 'iOS Simulator'}",
        "-derivedDataPath",
        str(root / "ios/build/DerivedData"),
        action,
        "CODE_SIGNING_ALLOWED=NO",
        "CODE_SIGNING_REQUIRED=NO",
        "CODE_SIGN_IDENTITY=",
        "DEVELOPMENT_TEAM=",
        "ARCHS=arm64",
    ]


def build_commands(root: Path) -> list[list[str]]:
    """Plan Rust/binding generation, project generation, and unsigned Xcode builds."""
    host = "aarch64-apple-darwin"
    commands = [
        cargo_command(
            root,
            "build",
            "--release",
            "-p",
            "papertrail-ffi",
            "--features",
            "bindgen",
            "--target",
            host,
        ),
        cargo_command(
            root,
            "run",
            "--release",
            "-p",
            "papertrail-ffi",
            "--features",
            "bindgen",
            "--bin",
            "papertrail-bindgen",
            "--target",
            host,
        )
        + [
            "--",
            "generate",
            "--library",
            str(root / f"target/ios/{host}/release/libpapertrail_ffi.dylib"),
            "--language",
            "swift",
            "--out-dir",
            str(root / "ios/Generated"),
            "--config",
            str(root / "crates/papertrail-ffi/uniffi.toml"),
        ],
    ]
    for target in TARGETS.values():
        commands.append(
            cargo_command(
                root,
                "build",
                "--release",
                "-p",
                "papertrail-ffi",
                "--lib",
                "--target",
                target,
            )
        )
    commands.append(["xcodegen", "generate", "--spec", str(root / "ios/project.yml")])
    commands.extend(xcode_command(root, sdk) for sdk in TARGETS)
    return commands


def build(root: Path, output: Path, dry_run: bool = False) -> Path | None:
    """Build bindings, device/simulator libraries, and app bundles reproducibly."""
    commands = build_commands(root)
    if dry_run:
        for command in commands:
            print(shlex.join(command))
        print(f"Package unsigned IPA, simulator app, and SHA256SUMS under {output}")
        return None
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ValueError(
            "iOS packaging requires an Apple Silicon Mac with Xcode (or the macOS GitHub workflow)"
        )
    for tool in ("cargo", "rustc", "xcrun", "xcodebuild", "xcodegen", "ditto"):
        if shutil.which(tool) is None:
            raise ValueError(
                f"Missing prerequisite: {tool}; no tools are installed automatically"
            )
    for sdk in TARGETS:
        subprocess.run(
            ["xcrun", "--sdk", sdk, "--show-sdk-path"],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    env = os.environ.copy()
    env["IPHONEOS_DEPLOYMENT_TARGET"] = "17.0"
    # Do not inherit a Linux compiler or static-CRT flags from unrelated builds.
    for key in ("CC", "CXX", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "SDKROOT"):
        env.pop(key, None)
    for command in commands[:4]:
        subprocess.run(command, cwd=root, env=env, check=True)
    for sdk, target in TARGETS.items():
        destination = root / "ios/build/rust" / sdk
        destination.mkdir(parents=True, exist_ok=True)
        shutil.copy2(
            root / f"target/ios/{target}/release/libpapertrail_ffi.a", destination
        )
    for command in commands[4:]:
        subprocess.run(command, cwd=root, env=env, check=True)
    return package_artifacts(root, output)


def package_artifacts(root: Path, output: Path) -> Path:
    """Package app bundles without adding signatures or replacing previous output."""
    products = root / "ios/build/DerivedData/Build/Products"
    for sdk in TARGETS:
        app = products / f"Release-{sdk}/PaperTrail.app"
        if not app.is_dir():
            raise ValueError(f"Missing built application: {app}")
        if (app / "_CodeSignature").exists() or (
            app / "embedded.mobileprovision"
        ).exists():
            raise ValueError(
                "Refusing to package a previously signed app; "
                "use a clean DerivedData directory"
            )
    output.mkdir(parents=True, exist_ok=True)
    destination = Path(tempfile.mkdtemp(prefix="ios-unsigned-", dir=output))
    with tempfile.TemporaryDirectory(prefix="papertrail-ios-") as temporary:
        staging = Path(temporary)
        payload = staging / "Payload"
        payload.mkdir()
        shutil.copytree(
            products / "Release-iphoneos/PaperTrail.app",
            payload / "PaperTrail.app",
            symlinks=True,
        )
        subprocess.run(
            [
                "ditto",
                "-c",
                "-k",
                "--keepParent",
                str(payload),
                str(destination / "paper-trail-ios-arm64-unsigned.ipa"),
            ],
            check=True,
        )
    subprocess.run(
        [
            "ditto",
            "-c",
            "-k",
            "--keepParent",
            str(products / "Release-iphonesimulator/PaperTrail.app"),
            str(destination / "Paper-Trail-simulator.zip"),
        ],
        check=True,
    )
    checksums = []
    for artifact in sorted(destination.iterdir()):
        with artifact.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        checksum = f"{digest}  {artifact.name}\n"
        checksums.append(checksum)
        artifact.with_name(f"{artifact.name}.sha256").write_text(
            checksum, encoding="utf-8"
        )
    (destination / "SHA256SUMS").write_text("".join(checksums), encoding="utf-8")
    print(f"Unsigned artifacts: {destination}")
    return destination


def main() -> int:
    """Parse local build flags and report actionable failures without installing tools."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path("dist/local"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        build(root, args.output_dir.resolve(), args.dry_run)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"iOS build stopped: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
