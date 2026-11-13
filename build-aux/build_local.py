#!/usr/bin/env python3
"""Interactively build local archives; never install tools, publish, or touch notes."""

from __future__ import annotations

import argparse
import hashlib
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from package_desktop import package as package_desktop
from package_tui import create_archive
from package_tui import package as package_standalone
from prepare_release import NATIVE_TARGETS

COMPONENTS = {
    "gtk": ("paper-trail", "desktop-release.md"),
    "tui": ("paper-trail-tui", "tui.md"),
    "server": ("paper-trail-server", "server-release.md"),
}


@dataclass(frozen=True)
class Build:
    """Public build choices only; do not store or print the process environment."""

    root: Path
    host: str
    target: str
    mode: str
    components: tuple[str, ...]
    output: Path
    target_dir: Path

    def command(self) -> list[str]:
        """Always select an explicit target, ignoring implicit Cargo build.target."""
        command = [
            "cargo",
            "build",
            "--release",
            "--locked",
            "--target",
            self.target,
            "--target-dir",
            str(self.target_dir),
        ]
        for component in self.components:
            command.extend(("--bin", COMPONENTS[component][0]))
        return command

    def binary(self, component: str) -> Path:
        """Locate the selected target's executable, including custom Cargo caches."""
        suffix = ".exe" if "windows" in self.target else ""
        return (
            self.target_dir
            / self.target
            / "release"
            / f"{COMPONENTS[component][0]}{suffix}"
        )


def host_target() -> str:
    """Read the installed compiler's host; rustup is not required for native builds."""
    for tool in ("cargo", "rustc"):
        require(tool)
    output = subprocess.check_output(["rustc", "-vV"], text=True)
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise ValueError("Could not determine the installed Rust host target")


def standalone_target(host: str) -> str:
    """Choose only an executable target for this OS/CPU, not a cross-build matrix."""
    target = f"{host.split('-')[0]}-unknown-linux-musl" if "linux" in host else host
    if target not in NATIVE_TARGETS:
        raise ValueError(
            f"No verified standalone target for host {host}; choose native mode"
        )
    return target


def available(host: str, mode: str) -> tuple[str, ...]:
    """GTK archives currently support Linux x86_64 and always require shared GNOME libs."""
    if mode == "native" and host.startswith("x86_64-") and "linux" in host:
        return tuple(COMPONENTS)
    return ("tui", "server")


def choices(args: argparse.Namespace, root: Path, host: str) -> Build:
    """Flags override individual prompts; unattended runs must explicitly opt in."""
    interactive = not (args.non_interactive or args.dry_run)
    if interactive and not sys.stdin.isatty():
        raise ValueError(
            "No interactive terminal; use --non-interactive (or --dry-run)"
        )
    mode = (
        args.mode
        or (
            input("Build type: native or standalone [native]: ").strip()
            if interactive
            else ""
        )
        or "native"
    )
    if mode not in ("native", "standalone"):
        raise ValueError("Build type must be native or standalone")
    supported = available(host, mode)
    selected = args.components
    if selected is None and interactive:
        selected = input(
            f"Components (comma separated, or all) [{','.join(supported)}]: "
        ).strip()
    selected = selected or "all"
    components = (
        supported
        if selected == "all"
        else tuple(dict.fromkeys(part.strip() for part in selected.split(",")))
    )
    if any(component not in supported for component in components):
        raise ValueError(
            f"Supported {mode} components for {host}: {', '.join(supported)}"
        )
    target = args.target or (standalone_target(host) if mode == "standalone" else host)
    expected = standalone_target(host) if mode == "standalone" else host
    if target != expected:
        raise ValueError(
            f"Use target {expected} on this machine; other OS/CPU builds need "
            "their native CI runners (packaging runs the executable)"
        )
    output = args.output_dir
    if output is None and interactive:
        output = input("Output directory [dist/local]: ").strip()
    output = Path(output or "dist/local").expanduser()
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", "target")).expanduser()
    return Build(
        root,
        host,
        target,
        mode,
        components,
        output if output.is_absolute() else root / output,
        target_dir if target_dir.is_absolute() else root / target_dir,
    )


def require(tool: str) -> None:
    """Report missing tools without installing or changing anything."""
    if shutil.which(tool) is None:
        raise ValueError(
            f"Missing prerequisite: {tool}. Install it yourself, then rerun; "
            "no tools are installed automatically"
        )


def preflight(build: Build) -> None:
    """Validate target support and local dependencies before creating output files."""
    libdir = Path(
        subprocess.check_output(
            ["rustc", "--print", "target-libdir", "--target", build.target], text=True
        ).strip()
    )
    if not libdir.is_dir() or not any(libdir.glob("libstd-*")):
        raise ValueError(
            f"Rust standard library for {build.target} is missing. "
            f"If using rustup, run: rustup target add {build.target}"
        )
    if "gtk" in build.components:
        for tool in ("pkg-config", "glib-compile-resources", "ldd"):
            require(tool)
        for library, version in (
            ("gtk4", "4.16"),
            ("libadwaita-1", "1.5"),
            ("gtksourceview-5", "5.10"),
        ):
            if subprocess.run(
                ["pkg-config", f"--atleast-version={version}", library], check=False
            ).returncode:
                raise ValueError(
                    f"GTK build needs {library} >= {version} development files; "
                    "install them or choose --components tui,server"
                )
    if build.mode == "standalone":
        if "linux" in build.target:
            require("musl-gcc")
            require("readelf")
        elif "apple" in build.target:
            require("otool")


def environment(build: Build) -> dict[str, str]:
    """Keep user flags, adding static CRT only for the verified standalone mode."""
    env = os.environ.copy()
    if build.mode == "standalone":
        env["AWS_LC_SYS_USE_SYSTEM"] = "0"
        if "linux" in build.target or "windows" in build.target:
            if "CARGO_ENCODED_RUSTFLAGS" in env:
                existing = env["CARGO_ENCODED_RUSTFLAGS"].rstrip("\x1f")
                env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(
                    filter(None, (existing, "-C", "target-feature=+crt-static"))
                )
            else:
                env["RUSTFLAGS"] = (
                    f"{env.get('RUSTFLAGS', '')} -C target-feature=+crt-static".strip()
                )
        if "linux" in build.target:
            env["CC"] = "musl-gcc"
    return env


def summary(build: Build) -> None:
    """Show the build and prerequisites before confirmation or a dry run."""
    print(f"Host: {build.host}\nTarget: {build.target}\nMode: {build.mode}")
    print(f"Components: {', '.join(build.components)}")
    print(
        f"Output: {build.output} / new timestamped directory (existing files are kept)"
    )
    print(
        "Prerequisites: Rust 1.88+, Cargo, a C toolchain; Python 3.11+ for packaging."
    )
    if "gtk" in build.components:
        print(
            "GTK: pkg-config, glib-compile-resources, GTK 4.16+, "
            "libadwaita 1.5+, GtkSourceView 5.10+."
        )
    if build.mode == "standalone":
        print(
            "Standalone: installed Rust target; Linux needs musl-gcc/readelf; "
            "macOS needs otool; Windows needs MSVC build tools."
        )
    else:
        print(
            "Native archives are labeled native; compatible OS/runtime libraries "
            "may be required. They are not verified standalone releases."
        )
    print("Build:", shlex.join(build.command()))


def execute(build: Build) -> Path:
    """Build/package a new isolated output set; never delete outputs or publish."""
    preflight(build)
    build.output.mkdir(parents=True, exist_ok=True)
    destination = Path(
        tempfile.mkdtemp(prefix=f"{time.strftime('%Y%m%d-%H%M%S')}-", dir=build.output)
    )
    print(f"Build output directory: {destination}", flush=True)
    subprocess.run(build.command(), cwd=build.root, env=environment(build), check=True)
    archives = []
    for component in build.components:
        binary = build.binary(component)
        if component == "gtk":
            archive = package_desktop(
                build.root, binary=binary, destination=destination
            )
        elif build.mode == "standalone":
            archive = package_standalone(
                build.root,
                build.target,
                executable_name=COMPONENTS[component][0],
                documentation=build.root / "docs" / COMPONENTS[component][1],
                check_version=component == "tui",
                binary=binary,
                destination=destination,
            )
        else:
            subprocess.run(
                [str(binary), "--help"], check=True, stdout=subprocess.DEVNULL
            )
            info = (
                f"Paper Trail local native build\nTarget: {build.target}\n"
                "This is not a verified standalone/static release.\n"
                "Compatible OS, libc/C runtime, and other runtime libraries may be required.\n"
                "No configuration, credentials, notes or database are included.\n"
            )
            archive = create_archive(
                binary,
                build.root / "docs" / COMPONENTS[component][1],
                destination,
                f"{COMPONENTS[component][0]}-{build.target}-native",
                info,
            )
        archives.append(archive)
    lines = []
    for archive in archives:
        with archive.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {archive.name}\n")
    (destination / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    print(f"\nLocal artefacts ready: {destination}")
    return destination


def parser() -> argparse.ArgumentParser:
    """Keep unattended and preview invocations explicit and discoverable."""
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--components",
        help="gtk,tui,server or all (default: all supported for this host/mode)",
    )
    result.add_argument(
        "--mode",
        choices=("native", "standalone"),
        help="native (default), or verified standalone TUI/server",
    )
    result.add_argument(
        "--target",
        help="target for this OS/CPU; defaults to host or matching standalone target",
    )
    result.add_argument(
        "--output-dir",
        help="output parent (default: dist/local); every run gets a fresh subdirectory",
    )
    result.add_argument(
        "--non-interactive",
        action="store_true",
        help="use flags/defaults without prompts or confirmation",
    )
    result.add_argument(
        "--dry-run",
        action="store_true",
        help="print choices/commands without building or creating files",
    )
    return result


def main(argv: list[str] | None = None) -> int:
    """Build only after confirmation; all failures are safe and actionable."""
    args = parser().parse_args(argv)
    try:
        build = choices(args, Path(__file__).resolve().parent.parent, host_target())
        summary(build)
        if args.dry_run:
            return 0
        if not args.non_interactive and input(
            "Build these artefacts? [y/N]: "
        ).strip().lower() not in ("y", "yes"):
            print("Cancelled; no artefacts were created.")
            return 0
        execute(build)
        return 0
    except (
        OSError,
        ValueError,
        subprocess.CalledProcessError,
        EOFError,
        KeyboardInterrupt,
    ) as error:
        print(f"Build stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
