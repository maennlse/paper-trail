#!/usr/bin/env python3
"""Verify standalone linkage and package a native TUI build (no Python at runtime)."""

from __future__ import annotations

import hashlib
import io
import struct
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path


def windows_imports(binary: Path) -> list[str]:
    """Read the PE import directory without requiring Visual Studio tools on PATH."""
    data = binary.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if struct.unpack_from("4s", data, pe)[0] != b"PE\0\0":
        raise ValueError("Not a PE executable")
    sections = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directory = optional + (112 if magic == 0x20B else 96)
    imports_rva = struct.unpack_from("<I", data, directory + 8)[0]

    def offset(rva: int) -> int:
        for index in range(sections):
            header = optional + optional_size + index * 40
            size, address, raw_size, raw_offset = struct.unpack_from(
                "<IIII", data, header + 8
            )
            if address <= rva < address + max(size, raw_size):
                return raw_offset + rva - address
        raise ValueError(f"Invalid PE address: {rva}")

    if not imports_rva:
        return []
    descriptor = offset(imports_rva)
    libraries = []
    while any(struct.unpack_from("<IIIII", data, descriptor)):
        name_rva = struct.unpack_from("<I", data, descriptor + 12)[0]
        start = offset(name_rva)
        libraries.append(data[start:].split(b"\0", 1)[0].decode("ascii"))
        descriptor += 20
    return libraries


def verify(binary: Path, target: str, *, check_version: bool = True) -> None:
    """Reject dynamic Linux builds and third-party runtime dependencies elsewhere."""
    if "linux" in target:
        program = subprocess.check_output(
            ["readelf", "--program-headers", str(binary)], text=True
        )
        dynamic = subprocess.check_output(
            ["readelf", "--dynamic", str(binary)], text=True
        )
        if "INTERP" in program or "NEEDED" in dynamic:
            raise ValueError("Linux artifact is not fully static")
    elif "windows" in target:
        libraries = windows_imports(binary)
        system_dlls = {
            "advapi32.dll",
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "combase.dll",
            "comdlg32.dll",
            "crypt32.dll",
            "dbghelp.dll",
            "gdi32.dll",
            "iphlpapi.dll",
            "kernel32.dll",
            "ntdll.dll",
            "ole32.dll",
            "oleaut32.dll",
            "rpcrt4.dll",
            "secur32.dll",
            "shell32.dll",
            "shlwapi.dll",
            "user32.dll",
            "userenv.dll",
            "ws2_32.dll",
        }
        print("Windows system DLL imports:", ", ".join(libraries))
        for name in libraries:
            lower = name.lower()
            if lower not in system_dlls and not lower.startswith(
                ("api-ms-win-core-", "api-ms-win-security-", "api-ms-win-eventing-")
            ):
                raise ValueError(f"Non-system dependency or dynamic C runtime: {name}")
    elif "apple" in target:
        output = subprocess.check_output(["otool", "-L", str(binary)], text=True)
        for line in output.splitlines()[1:]:
            library = line.strip().split(" (", 1)[0]
            if not library.startswith(("/usr/lib/", "/System/Library/")):
                raise ValueError(f"Non-system macOS dependency: {library}")
    else:
        raise ValueError(f"Unsupported standalone target: {target}")
    if check_version:
        subprocess.run([str(binary), "--version"], check=True)
    subprocess.run([str(binary), "--help"], check=True, stdout=subprocess.DEVNULL)


# Explicit keyword-only source/output overrides keep local builds out of CI's dist/.
# pylint: disable-next=too-many-arguments
def package(
    root: Path,
    target: str,
    *,
    executable_name: str,
    documentation: Path,
    check_version: bool = True,
    binary: Path | None = None,
    destination: Path | None = None,
) -> Path:
    """Package a verified standalone executable and its documentation, never data."""
    executable = f"{executable_name}.exe" if "windows" in target else executable_name
    binary = binary or root / "target" / target / "release" / executable
    verify(binary, target, check_version=check_version)
    return create_archive(
        binary,
        documentation,
        destination or root / "dist",
        f"{executable_name}-{target}",
    )


def create_archive(
    binary: Path,
    documentation: Path,
    destination: Path,
    stem: str,
    build_info: str | None = None,
) -> Path:
    """Archive only explicit public files; native builds include a dependency notice."""
    for source in (binary, documentation):
        if not source.is_file() or source.is_symlink():
            raise ValueError(
                f"Archive input must be a regular file, not a directory/link: {source}"
            )
    destination.mkdir(parents=True, exist_ok=True)
    executable = binary.name
    if executable.endswith(".exe"):
        archive = destination / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            bundle.write(binary, executable)
            bundle.write(documentation, "README.md")
            if build_info is not None:
                bundle.writestr("BUILD-INFO.txt", build_info)
    else:
        archive = destination / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(binary, arcname=executable)
            bundle.add(documentation, arcname="README.md")
            if build_info is not None:
                data = build_info.encode("utf-8")
                entry = tarfile.TarInfo("BUILD-INFO.txt")
                entry.size = len(data)
                entry.mode = 0o644
                bundle.addfile(entry, io.BytesIO(data))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8"
    )
    print(f"Packaged {archive}")
    return archive


def main() -> None:
    """Package one of the six native CI target builds, with a checksum."""
    root = Path(__file__).resolve().parent.parent
    package(
        root,
        sys.argv[1],
        executable_name="paper-trail-tui",
        documentation=root / "docs/tui.md",
    )


if __name__ == "__main__":
    main()
