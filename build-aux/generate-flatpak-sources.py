#!/usr/bin/env python3
"""Generate checksum-pinned, offline Cargo sources from Cargo.lock (Python 3.11+)."""
import json
import sys
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parent.parent
lock = tomllib.loads((root / "Cargo.lock").read_text())
sources = []
for package in lock["package"]:
    if package.get("source", "").startswith("registry+"):
        name, version, checksum = (package[key] for key in ("name", "version", "checksum"))
        dest = f"cargo/vendor/{name}-{version}"
        sources.extend([
            {"type": "archive", "archive-type": "tar-gzip", "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate", "sha256": checksum, "dest": dest},
            {"type": "inline", "contents": json.dumps({"package": checksum, "files": {}}), "dest": dest, "dest-filename": ".cargo-checksum.json"},
        ])
sources.append({"type": "inline", "contents": '[source.crates-io]\nreplace-with = "vendored-sources"\n[source.vendored-sources]\ndirectory = "cargo/vendor"\n', "dest": ".cargo", "dest-filename": "config.toml"})
output = json.dumps(sources, indent=2) + "\n"
destination = root / "cargo-sources.json"
if "--check" in sys.argv:
    if not destination.exists() or destination.read_text() != output:
        raise SystemExit("cargo-sources.json is stale; run build-aux/generate-flatpak-sources.py")
else:
    destination.write_text(output)
