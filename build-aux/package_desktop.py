#!/usr/bin/env python3
"""Package the dynamic Linux x86_64 GTK executable and its runtime requirements."""

from __future__ import annotations

import hashlib
import io
import struct
import subprocess
import tarfile
from pathlib import Path


def package(
    root: Path,
    distribution_file: Path = Path("/etc/os-release"),
    *,
    binary: Path | None = None,
    destination: Path | None = None,
) -> Path:
    """Validate a trusted local build, then archive it without launching the GUI."""
    binary = binary or root / "target/release/paper-trail"
    with binary.open("rb") as executable:
        header = executable.read(20)
    if (
        header[:6] != b"\x7fELF\x02\x01"
        or struct.unpack_from("<H", header, 18)[0] != 62
    ):
        raise ValueError("Expected a Linux x86_64 ELF executable")
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
        f"Build distribution:\n{distribution}\n"
        f"GTK / libadwaita / GtkSourceView build versions:\n{versions}\n"
        f"Dynamic library dependencies:\n{libraries}"
    ).encode()
    destination = destination or root / "dist"
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / "paper-trail-gtk-linux-x86_64.tar.gz"
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
    print(f"Packaged {package(Path(__file__).resolve().parent.parent)}")
