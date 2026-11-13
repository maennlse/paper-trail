#!/usr/bin/env python3
"""Capture real GTK/VTE windows with isolated synthetic notes (no generated UI)."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


def demo(home: Path, editor: str = "builtin") -> tuple[Path, dict[str, str]]:
    """Create only demonstration data in the caller-owned temporary home."""
    folders = [home / "Notes" / name for name in ("Personal", "Work", "Ideas")]
    notes = [
        (
            0,
            "Welcome to Paper Trail.txt",
            (
                "Welcome to Paper Trail\n\n"
                "Your notes. Plain text. Desktop or terminal.\n\n"
                "A little room for the things on your mind:\n\n"
                "  - Capture an idea before it disappears\n"
                "  - Keep work and personal notes in their own folders\n"
                "  - Find a thought by its title or any word inside\n"
                "  - Open the same files in your favourite editor\n\n"
                "No special format. No account required for local notes.\n"
                "Just a quiet place to think.\n"
            ),
        ),
        (
            0,
            "Weekend plans.txt",
            (
                "Weekend plans\n\nVisit the bookshop. Take the scenic route home.\n"
                "Leave room for a long lunch and a good conversation.\n"
            ),
        ),
        (
            0,
            "Reading list.txt",
            "Reading list\n\nA book for the train and a notebook for the journey.\n",
        ),
        (
            1,
            "Release checklist.txt",
            (
                "Release checklist\n\nRun the tests. Review the changes. Check the downloads.\n"
                "Write a short note about what is new.\n"
            ),
        ),
        (
            1,
            "Meeting notes.txt",
            "Meeting notes\n\nKeep the plan small. Make the next step clear.\n",
        ),
        (1, "Today.txt", "Today\n\nOne useful improvement, finished well.\n"),
        (
            2,
            "A thought for later.txt",
            "A thought for later\n\nSimple tools leave more space for the work itself.\n",
        ),
        (
            2,
            "Things to try.txt",
            "Things to try\n\nA small experiment is a good place to start.\n",
        ),
    ]
    stamp = time.time()
    paths = []
    for index, (folder, name, body) in enumerate(notes):
        folders[folder].mkdir(parents=True, exist_ok=True)
        path = folders[folder] / name
        path.write_text(body, encoding="utf-8")
        os.utime(path, (stamp - index * 60, stamp - index * 60))
        paths.append(path)
    config = home / "config/paper-trail/settings.json"
    config.parent.mkdir(parents=True)
    config.write_text(
        json.dumps(
            {
                "notes_dir": str(folders[0]),
                "note_folders": list(map(str, folders)),
                "folder_colors": {
                    str(folder): token
                    for folder, token in zip(
                        folders, ("preset-0", "preset-4", "preset-2")
                    )
                },
                "pinned_notes": [str(paths[0]), str(paths[3])],
                "theme_mode": "light",
                "width": 1200,
                "height": 780,
                "sidebar_width": 330,
                "show_line_numbers": False,
                "font_scale": 1.1,
            }
        ),
        encoding="utf-8",
    )
    config.with_name("tui.json").write_text(
        json.dumps(
            {
                "folder_width": 22,
                "hide_numbers": True,
                "editor": editor,
                "nvim_clean": True,
            }
        ),
        encoding="utf-8",
    )
    sync = config.parent / "sync/config.json"
    sync.parent.mkdir()
    sync.write_text(
        json.dumps(
            {
                "version": 2,
                "server": "https://sync.example.com",
                "issuer": "https://login.example.com",
                "client_id": "paper-trail-demo",
                "scopes": "openid offline_access",
                "folders": list(map(str, folders[:2])),
                "download_dir": str(home / "Notes/Synced"),
                "auto_download": False,
            }
        ),
        encoding="utf-8",
    )
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("PAPERTRAIL_")
    }
    env.update(
        {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / "config"),
            "XDG_DATA_HOME": str(home / "data"),
            "XDG_CACHE_HOME": str(home / "cache"),
            "GSETTINGS_BACKEND": "memory",
            "GDK_BACKEND": "x11",
            "GSK_RENDERER": "cairo",
            "GTK_A11Y": "none",
            "TERM": "xterm-256color",
            "COLORTERM": "truecolor",
            "SSH_CONNECTION": "screenshot-demo",
            "RUST_LOG": "warn",
            "XDG_RUNTIME_DIR": str(home / "runtime"),
            "NVIM_LOG_FILE": str(home / "nvim.log"),
            "NVIM_APPNAME": "nvim",
        }
    )
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)
    env.pop("NVIM", None)
    env.pop("NVIM_LISTEN_ADDRESS", None)
    (home / "runtime").mkdir(mode=0o700)
    return config, env


def window_for(pid: int) -> str | None:
    """Wait for this demo process's own X11 window, never another user's window."""
    result = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--pid", str(pid)],
        capture_output=True,
        text=True,
        check=False,
    )
    return (
        next(iter(result.stdout.splitlines()), None) if result.returncode == 0 else None
    )


def xdo(*arguments: str) -> None:
    """Send ordinary input to an explicitly identified demo window."""
    subprocess.run(["xdotool", *arguments], check=True)


# The capture harness keeps both live UIs and their event wiring in one scope.
# pylint: disable-next=too-many-locals,too-many-statements
def capture(root: Path, home: Path, output: Path, editor: str = "builtin") -> None:
    """Run real binaries and snapshot actual windows in an isolated Xvfb display."""
    # Set isolation before importing GTK/GDK: they can open a display on import.
    config, env = demo(home, editor)
    command_prefix = b"\x1bOQ" if editor == "neovim" else b":"
    for key in list(os.environ):
        if key.startswith("PAPERTRAIL_") or key in (
            "WAYLAND_DISPLAY",
            "WAYLAND_SOCKET",
        ):
            del os.environ[key]
    os.environ.update(env)
    # Optional screenshot-only dependencies, never application runtime dependencies.
    # pylint: disable=import-outside-toplevel
    import gi

    gi.require_version("Gtk", "4.0")
    gi.require_version("Gdk", "4.0")
    gi.require_version("GdkX11", "4.0")
    gi.require_version("Vte", "3.91")
    from gi.repository import Gdk, GdkX11, Gio, GLib, Gtk, Pango, Vte

    output.mkdir(parents=True, exist_ok=True)
    app = Gtk.Application(
        application_id="io.github.maennlse.PaperTrailScreenshots",
        flags=Gio.ApplicationFlags.NON_UNIQUE,
    )
    state = {
        "phase": "startup",
        "until": time.monotonic() + 30,
        "after": 0.0,
        "gtk": None,
        "error": None,
    }

    # Keep the optional GTK/VTE setup together, rather than altering application code.
    # pylint: disable-next=too-many-statements
    def activate(application: Gtk.Application) -> None:
        terminal = Vte.Terminal()
        terminal.set_font(Pango.FontDescription.from_string("Monospace 12"))
        foreground, background = Gdk.RGBA(), Gdk.RGBA()
        foreground.parse("#deddda")
        background.parse("#1e1e1e")
        # A normal terminal palette with legible secondary text; app RGB hues stay exact.
        palette = []
        for color in (
            "#171421",
            "#c01c28",
            "#26a269",
            "#a2734c",
            "#12488b",
            "#a347ba",
            "#2aa1b3",
            "#d0cfcc",
            "#8a8a8a",
            "#f66151",
            "#33d17a",
            "#e9ad0c",
            "#2a7bde",
            "#c061cb",
            "#33c7de",
            "#ffffff",
        ):
            value = Gdk.RGBA()
            value.parse(color)
            palette.append(value)
        terminal.set_colors(foreground, background, palette)
        terminal.set_cursor_blink_mode(Vte.CursorBlinkMode.OFF)
        window = Gtk.ApplicationWindow(
            application=application, title="Paper Trail · Terminal"
        )
        window.set_decorated(False)
        window.set_default_size(1200, 750)
        window.set_child(terminal)

        def spawned(
            _terminal: Vte.Terminal, _pid: int, error: object, *_data: object
        ) -> None:
            if error is not None:
                state["error"] = f"Could not launch the demo terminal: {error}"
                application.quit()

        terminal.spawn_async(
            pty_flags=Vte.PtyFlags.DEFAULT,
            working_directory=str(home),
            argv=[
                str(root / "target/release/paper-trail-tui"),
                "--config",
                str(config),
            ],
            envv=[f"{key}={value}" for key, value in env.items()],
            spawn_flags=GLib.SpawnFlags.DEFAULT,
            child_setup=None,
            timeout=-1,
            cancellable=None,
            callback=spawned,
            user_data=None,
        )
        window.present()

        def exited(_terminal: Vte.Terminal, _status: int) -> None:
            if state["phase"] != "quitting":
                state["error"] = "Demo terminal exited before capture completed"
            application.quit()

        terminal.connect("child-exited", exited)

        # Explicit screen states make real-window capture failures easy to diagnose.
        # pylint: disable-next=too-many-branches
        def tick() -> bool:
            try:
                now = time.monotonic()
                text = terminal.get_text_format(Vte.Format.TEXT) or ""
                if now > state["until"] or desktop.poll() is not None:
                    lines = text.splitlines()
                    raise RuntimeError(
                        f"Screenshot demo stopped during {state['phase']}; "
                        f"demo header={lines[:1]!r}; demo status={lines[-3:]!r}"
                    )
                if now < state["after"]:
                    return True
                phase = state["phase"]
                if phase == "startup":
                    native = window_for(desktop.pid)
                    if native is None or "Welcome to Paper Trail" not in text:
                        return True
                    state["gtk"] = native
                    xdo(
                        "windowraise",
                        native,
                        "windowfocus",
                        native,
                        "mousemove",
                        "--window",
                        native,
                        "25",
                        "85",
                        "click",
                        "1",
                    )
                    state.update(phase="open-note", after=now + 0.3)
                elif phase == "open-note":
                    xdo(
                        "mousemove",
                        "--window",
                        state["gtk"],
                        "190",
                        "105",
                        "click",
                        "1",
                    )
                    state.update(phase="gtk", after=now + 0.5)
                elif phase == "gtk":
                    subprocess.run(
                        [
                            "import",
                            "-silent",
                            "-window",
                            state["gtk"],
                            "-strip",
                            str(output / "gtk-editor.png"),
                        ],
                        check=True,
                    )
                    surface = window.get_surface()
                    if surface is None:
                        raise RuntimeError("Demo terminal window is unavailable")
                    native = str(GdkX11.X11Surface.get_xid(surface))
                    xdo("windowraise", native, "windowfocus", native)
                    terminal.feed_child(command_prefix + b"all\r")
                    state.update(phase="all", until=now + 30)
                elif (
                    phase == "all"
                    and "8 notes" in text
                    and (editor == "builtin" or "· Neovim" in text)
                ):
                    state.update(phase="tui", after=now + 0.4)
                elif phase == "tui":
                    subprocess.run(
                        [
                            "import",
                            "-silent",
                            "-window",
                            str(GdkX11.X11Surface.get_xid(window.get_surface())),
                            "-strip",
                            str(
                                output
                                / (
                                    "tui-neovim.png"
                                    if editor == "neovim"
                                    else "tui.png"
                                )
                            ),
                        ],
                        check=True,
                    )
                    terminal.feed_child(command_prefix + b"sync-folders\r")
                    state.update(phase="picker", until=now + 30)
                elif (
                    phase == "picker"
                    and "Sync folders" in text
                    and "Enter save" in text
                ):
                    state.update(phase="sync", after=now + 0.3)
                elif phase == "sync":
                    subprocess.run(
                        [
                            "import",
                            "-silent",
                            "-window",
                            str(GdkX11.X11Surface.get_xid(window.get_surface())),
                            "-strip",
                            str(
                                output
                                / (
                                    "tui-sync-folders-neovim.png"
                                    if editor == "neovim"
                                    else "tui-sync-folders.png"
                                )
                            ),
                        ],
                        check=True,
                    )
                    terminal.feed_child(b"\x1b")
                    state.update(phase="cancel", after=now + 0.3)
                elif phase == "cancel":
                    state["phase"] = "quitting"
                    terminal.feed_child(command_prefix + b"q\r")
                return True
            except (
                OSError,
                RuntimeError,
                TypeError,
                AttributeError,
                subprocess.CalledProcessError,
            ) as error:
                state["error"] = str(error)
                application.quit()
                return False

        GLib.timeout_add(100, tick)

    app.connect("activate", activate)

    def expired() -> bool:
        state["error"] = "Screenshot capture exceeded 45 seconds"
        app.quit()
        return False

    GLib.timeout_add_seconds(45, expired)
    with subprocess.Popen(
        [str(root / "target/release/paper-trail")], cwd=home, env=env
    ) as desktop:
        try:
            app.run([])
        finally:
            if desktop.poll() is None:
                desktop.terminate()
            desktop.wait(timeout=5)
    if state["error"]:
        raise RuntimeError(state["error"])
    if state["phase"] != "quitting":
        raise RuntimeError("Screenshot capture did not reach all three screens")
    print(f"Captured actual GTK/TUI demo windows in {output}")


def main() -> None:
    """Require an isolated display and prebuilt binaries; install nothing."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--editor", choices=("builtin", "neovim"), default="builtin")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    for tool in ("xdotool", "import"):
        if shutil.which(tool) is None:
            parser.error(f"Screenshot capture requires {tool}")
    if args.editor == "neovim" and shutil.which("nvim") is None:
        parser.error("Embedded Neovim screenshots require nvim 0.9+ on PATH")
    for binary in ("paper-trail", "paper-trail-tui"):
        if not (root / "target/release" / binary).is_file():
            parser.error(f"Build --release --bin {binary} first")
    if not os.environ.get("DISPLAY"):
        parser.error(
            "Use dbus-run-session -- xvfb-run -a python3 build-aux/capture_screenshots.py"
        )
    with tempfile.TemporaryDirectory(prefix="papertrail-demo-") as directory:
        capture(
            root,
            Path(directory),
            args.output_dir or root / "data/screenshots",
            args.editor,
        )


if __name__ == "__main__":
    main()
