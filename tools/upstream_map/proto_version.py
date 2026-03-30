#!/usr/bin/env python3
"""
Track proto schema versions over time.

Compares per-file SHA-256 digests against a baseline to detect
new, removed, or modified proto files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PROTO_DIR = ROOT / "substrate" / "proto" / "v1"
VERSIONS_FILE = ROOT / "architecture" / "upstream-map" / "proto-versions.json"


def compute_per_file_hashes() -> dict[str, str]:
    """Compute SHA-256 for each proto file, keyed by relative path."""
    hashes: dict[str, str] = {}
    for proto in sorted(PROTO_DIR.glob("*.proto")):
        rel = proto.relative_to(ROOT).as_posix()
        hashes[rel] = hashlib.sha256(proto.read_bytes()).hexdigest()
    return hashes


def compute_root_hash(per_file: dict[str, str]) -> str:
    """Compute aggregate root hash from per-file hashes."""
    aggregate = hashlib.sha256()
    for path in sorted(per_file):
        aggregate.update(f"{per_file[path]}  {path}\n".encode("utf-8"))
    return aggregate.hexdigest()


def load_baseline() -> dict:
    """Load the current baseline from proto-versions.json."""
    if not VERSIONS_FILE.exists():
        return {}
    return json.loads(VERSIONS_FILE.read_text(encoding="utf-8"))


def save_baseline(baseline: dict) -> None:
    """Write the baseline to proto-versions.json."""
    VERSIONS_FILE.parent.mkdir(parents=True, exist_ok=True)
    VERSIONS_FILE.write_text(
        json.dumps(baseline, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def check() -> int:
    """Compare current proto state against baseline."""
    baseline = load_baseline()
    if not baseline:
        print("No baseline found. Run: python3 tools/upstream_map/proto_version.py --init")
        return 1

    current_hashes = compute_per_file_hashes()
    current_root = compute_root_hash(current_hashes)
    baseline_root = baseline.get("root_sha256", "")
    baseline_files = baseline.get("files", {})

    if current_root == baseline_root:
        print("Proto versions match baseline.")
        return 0

    print("Proto version changes detected:")
    added = []
    removed = []
    modified = []
    for path, sha in current_hashes.items():
        if path not in baseline_files:
            added.append(path)
        elif baseline_files[path] != sha:
            modified.append(path)
    for path in baseline_files:
        if path not in current_hashes:
            removed.append(path)

    if added:
        print(f"  Added ({len(added)}):")
        for p in added:
            print(f"    + {p}")
    if removed:
        print(f"  Removed ({len(removed)}):")
        for p in removed:
            print(f"    - {p}")
    if modified:
        print(f"  Modified ({len(modified)}):")
        for p in modified:
            print(f"    ~ {p}")

    print(f"\nRoot hash: {baseline_root[:12]}... -> {current_root[:12]}...")
    return 1


def init_baseline() -> int:
    """Create initial baseline from current proto files."""
    current_hashes = compute_per_file_hashes()
    current_root = compute_root_hash(current_hashes)
    baseline = {
        "root_sha256": current_root,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "files": current_hashes,
    }
    save_baseline(baseline)
    print(f"Baseline created with {len(current_hashes)} proto files.")
    print(f"Root hash: {current_root}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Track proto schema versions")
    parser.add_argument("--init", action="store_true", help="Create initial baseline")
    parser.add_argument("--check", action="store_true", help="Check against baseline")
    parser.add_argument("--update", action="store_true", help="Update baseline to current")
    args = parser.parse_args()

    if args.update:
        return init_baseline()
    if args.init:
        return init_baseline()
    if args.check:
        return check()

    parser.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
