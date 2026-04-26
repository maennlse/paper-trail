#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path
import os
import shutil
import subprocess
import sys


def main() -> int:
    if os.environ.get("DESTDIR"):
        return 0

    datadir = Path(sys.argv[1])
    icon_dir = datadir / "icons" / "hicolor"
    applications_dir = datadir / "applications"

    gtk_update_icon_cache = shutil.which("gtk-update-icon-cache")
    if gtk_update_icon_cache and icon_dir.exists():
        subprocess.run([gtk_update_icon_cache, "-q", "-t", str(icon_dir)], check=False)

    update_desktop_database = shutil.which("update-desktop-database")
    if update_desktop_database and applications_dir.exists():
        subprocess.run([update_desktop_database, str(applications_dir)], check=False)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
