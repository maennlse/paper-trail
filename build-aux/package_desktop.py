#!/usr/bin/env python3
"""Package a dynamic Linux GTK executable and its native runtime requirements."""

from __future__ import annotations

import argparse
import hashlib
import io
import struct
import subprocess
import tarfile
from pathlib import Path

ARCHITECTURES = {"x86_64": 62, "aarch64": 183}


def executable_architecture(binary: Path, expected: str | None = None) -> str:
    """Read ELF identity before inspecting native dependencies or writing outputs."""
    with binary.open("rb") as executable:
        header = executable.read(20)
    if len(header) < 20 or header[:6] != b"\x7fELF\x02\x01":
        raise ValueError("Expected a 64-bit little-endian Linux ELF executable")
    machine = struct.unpack_from("<H", header, 18)[0]
    detected = next(
        (arch for arch, value in ARCHITECTURES.items() if value == machine), None
    )
    if detected is None:
        raise ValueError("Unsupported GTK executable architecture")
    if expected is not None and expected != detected:
        raise ValueError(
            f"Expected a Linux {expected} ELF executable, found {detected}"
        )
    return detected


def package(
    root: Path,
    distribution_file: Path = Path("/etc/os-release"),
    *,
    binary: Path | None = None,
    destination: Path | None = None,
    architecture: str | None = None,
) -> Path:
    """Validate a trusted local build, then archive it without launching the GUI."""
    binary = binary or root / "target/release/paper-trail"
    architecture = executable_architecture(binary, architecture)
    libraries = subprocess.check_output(["ldd", str(binary)], text=True)
    required = ("libgtk-4.so", "libadwaita-1.so", "libgtksourceview-5.so")
    if "not found" in libraries or not all(name in libraries for name in required):
        raise ValueError(f"Missing GNOME runtime libraries:\n{libraries}")
    distribution = distribution_file.read_text(encoding="utf-8")
    versions = subprocess.check_output(
        ["pkg-config", "--modversion", "gtk4", "libadwaita-1", "gtksourceview-5"],
        text=True,
    )
    info = (
        "This is a dynamically linked GTK build, not a standalone/static binary.\n\n"
        f"Architecture: {architecture}\n"
        f"Build distribution:\n{distribution}\n"
        f"GTK / libadwaita / GtkSourceView build versions:\n{versions}\n"
        f"Dynamic library dependencies:\n{libraries}"
    ).encode()
    destination = destination or root / "dist"
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f"paper-trail-gtk-linux-{architecture}.tar.gz"
    with tarfile.open(archive, "w:gz") as bundle:
        bundle.add(binary, arcname="paper-trail")
        bundle.add(root / "docs/desktop-release.md", arcname="README.md")
        entry = tarfile.TarInfo("BUILD-INFO.txt")
        entry.size = len(info)
        entry.mode = 0o644
        bundle.addfile(entry, io.BytesIO(info))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8"
    )
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--arch",
        choices=ARCHITECTURES,
        help="Require this executable architecture (otherwise detect from ELF)",
    )
    args = parser.parse_args()
    print(
        f"Packaged {package(Path(__file__).resolve().parent.parent, architecture=args.arch)}"
    )
