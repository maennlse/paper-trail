#!/usr/bin/env python3
"""Generate checksum-pinned, offline Cargo sources from Cargo.lock (Python 3.11+)."""

from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path


def main() -> None:
    """Generate vendor sources, or check that the committed manifest is current."""
    root = Path(__file__).resolve().parent.parent
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    sources = []
    for package in lock["package"]:
        if package.get("source", "").startswith("registry+"):
            name, version, checksum = (
                package[key] for key in ("name", "version", "checksum")
            )
            dest = f"cargo/vendor/{name}-{version}"
            sources.extend(
                [
                    {
                        "type": "archive",
                        "archive-type": "tar-gzip",
                        "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                        "sha256": checksum,
                        "dest": dest,
                    },
                    {
                        "type": "inline",
                        "contents": json.dumps({"package": checksum, "files": {}}),
                        "dest": dest,
                        "dest-filename": ".cargo-checksum.json",
                    },
                ]
            )
    sources.append(
        {
            "type": "inline",
            "contents": (
                '[source.crates-io]\nreplace-with = "vendored-sources"\n'
                '[source.vendored-sources]\ndirectory = "cargo/vendor"\n'
            ),
            "dest": ".cargo",
            "dest-filename": "config.toml",
        }
    )
    output = json.dumps(sources, indent=2) + "\n"
    destination = root / "cargo-sources.json"
    if "--check" in sys.argv:
        if (
            not destination.exists()
            or destination.read_text(encoding="utf-8") != output
        ):
            raise SystemExit(
                "cargo-sources.json is stale; run build-aux/generate_flatpak_sources.py"
            )
    else:
        destination.write_text(output, encoding="utf-8")


if __name__ == "__main__":
    main()
