#!/usr/bin/env python3
import base64, json, sys
from pathlib import Path

staging = Path(__file__).resolve().parent
manifest = json.loads((staging / "manifest.json").read_text())
root = staging.parents[1]  # repo root (assets/.icon-staging -> assets -> root)
# Wait: staging is assets/.icon-staging, parents[0]=assets, parents[1]=root. Good.
for entry in manifest:
    dest = root / entry["dest"]
    dest.parent.mkdir(parents=True, exist_ok=True)
    raw = base64.b64decode((staging / entry["b64"]).read_text().strip())
    dest.write_bytes(raw)
    if len(raw) != entry["bytes"]:
        sys.exit(f"size mismatch {entry['dest']}: {len(raw)} != {entry['bytes']}")
    print(f"wrote {dest} ({len(raw)} bytes)")
