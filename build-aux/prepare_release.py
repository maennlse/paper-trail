#!/usr/bin/env python3
"""Validate version tags and collect only complete, checksum-verified release assets."""

from __future__ import annotations

import hashlib
import re
import shutil
import sys
import tomllib
from pathlib import Path

NATIVE_TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)
GTK_ARCHITECTURES = ("x86_64", "aarch64")
PACKAGES = (
    tuple(
        f"paper-trail-{component}-{target}.{'zip' if 'windows' in target else 'tar.gz'}"
        for component in ("tui", "server")
        for target in NATIVE_TARGETS
    )
    + tuple(
        f"paper-trail-gtk-linux-{arch}.{extension}"
        for arch in GTK_ARCHITECTURES
        for extension in ("tar.gz", "flatpak")
    )
    + ("paper-trail-ios-arm64-unsigned.ipa",)
)


def validate_tag(tag: str, manifest: Path) -> None:
    """Refuse a tag that would label binaries with the wrong Cargo version."""
    version = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"][
        "package"
    ]["version"]
    if tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} must match Cargo workspace version v{version}")


def validate_versions(root: Path) -> None:
    """Reject platform, packaging, or lockfile version drift before release builds."""
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = workspace["workspace"]["package"]["version"]
    for relative, pattern in (
        ("meson.build", r"version:\s*'([^']+)'"),
        ("ios/project.yml", r'MARKETING_VERSION:\s*"([^"]+)"'),
        ("docs/release-notes.md", r"^# Paper Trail (\S+)"),
    ):
        match = re.search(pattern, (root / relative).read_text(encoding="utf-8"))
        if match is None or match[1] != version:
            raise ValueError(f"{relative} must use release version {version}")
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    locked = {entry["name"]: entry["version"] for entry in lock["package"]}
    for member in workspace["workspace"]["members"]:
        crate = tomllib.loads(
            (root / member / "Cargo.toml").read_text(encoding="utf-8")
        )["package"]
        declared = crate["version"]
        actual = version if declared == {"workspace": True} else declared
        if actual != version or locked.get(crate["name"]) != version:
            raise ValueError(f"{member} and Cargo.lock must use version {version}")
    metadata = (root / "data/io.github.maennlse.paper-trail.metainfo.xml.in").read_text(
        encoding="utf-8"
    )
    match = re.search(r'<release version="([^"]+)"', metadata)
    if match is None or match[1] != "@APP_VERSION@":
        raise ValueError("AppStream must inherit the Meson release version")


def collect(artifacts: Path, destination: Path) -> None:
    """Fail closed on missing, duplicate, malformed, or corrupt packages."""
    verified = []
    lines = []
    for name in PACKAGES:
        matches = list(artifacts.rglob(name))
        if len(matches) != 1 or not matches[0].is_file() or matches[0].is_symlink():
            raise ValueError(f"Expected exactly one regular package: {name}")
        package = matches[0]
        sidecar = package.with_name(f"{name}.sha256")
        if sidecar.is_symlink():
            raise ValueError(f"Checksum must be a regular file: {sidecar}")
        fields = sidecar.read_text(encoding="utf-8").split()
        with package.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if fields != [digest, name]:
            raise ValueError(f"Checksum mismatch or malformed checksum: {name}")
        verified.extend((package, sidecar))
        lines.append(f"{digest}  {name}\n")
    if destination.exists() and any(destination.iterdir()):
        raise ValueError(
            f"Refusing to mix release assets with existing files: {destination}"
        )
    destination.mkdir(parents=True, exist_ok=True)
    for source in verified:
        shutil.copyfile(source, destination / source.name)
    (destination / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    print(
        f"Verified {len(PACKAGES)} packages; collected release assets in {destination}"
    )


def main() -> None:
    """Run a local validation/collection step; never create a tag or release."""
    if len(sys.argv) == 3 and sys.argv[1] == "validate-tag":
        validate_tag(sys.argv[2], Path(__file__).resolve().parent.parent / "Cargo.toml")
        validate_versions(Path(__file__).resolve().parent.parent)
    elif len(sys.argv) == 4 and sys.argv[1] == "collect":
        collect(Path(sys.argv[2]), Path(sys.argv[3]))
    else:
        raise SystemExit(
            "Usage: prepare_release.py validate-tag TAG | collect ARTIFACTS DESTINATION"
        )


if __name__ == "__main__":
    main()
