#!/usr/bin/env python3
"""Verify standalone server linkage and package only the executable and public docs."""

from __future__ import annotations

import sys
from pathlib import Path

from package_tui import package as package_standalone


def package(root: Path, target: str) -> Path:
    """Create a checksummed server archive without configuration or credentials."""
    return package_standalone(
        root,
        target,
        executable_name="paper-trail-server",
        documentation=root / "docs/server-release.md",
        # Server --help exits without OIDC discovery, database creation or listening.
        # The server does not currently offer --version.
        check_version=False,
    )


def main() -> None:
    """Package a native server built for the selected CI target."""
    package(Path(__file__).resolve().parent.parent, sys.argv[1])


if __name__ == "__main__":
    main()
