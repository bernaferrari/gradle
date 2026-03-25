#!/usr/bin/env python3
"""Discover unmapped platform/subproject module directories."""

from __future__ import annotations

import argparse
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = ROOT / "architecture" / "upstream-map" / "modules.toml"


def load_mapped_paths() -> set[str]:
    data = tomllib.loads(MAP_PATH.read_text(encoding="utf-8"))
    modules = data.get("module", [])
    mapped: set[str] = set()
    for module in modules:
        path = module.get("path")
        if isinstance(path, str):
            mapped.add(path)
    return mapped


def discover_candidates() -> list[str]:
    candidates: list[str] = []

    for root in [ROOT / "platforms", ROOT / "subprojects"]:
        if not root.exists():
            continue
        for child in sorted(root.iterdir()):
            if not child.is_dir():
                continue
            rel = child.relative_to(ROOT).as_posix()
            candidates.append(rel)

    return candidates


def to_module_id(path: str) -> str:
    return path.replace("/", "-")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--toml-stubs",
        action="store_true",
        help="Print TOML stubs for discovered unmapped directories.",
    )
    args = parser.parse_args()

    mapped = load_mapped_paths()
    candidates = discover_candidates()
    missing = [path for path in candidates if path not in mapped]

    if not missing:
        print("No unmapped top-level platform/subproject directories.")
        return 0

    if args.toml_stubs:
        for path in missing:
            module_id = to_module_id(path)
            print("[[module]]")
            print(f'id = "{module_id}"')
            print(f'path = "{path}"')
            print('owner = "unassigned"')
            print('status = "planned"')
            print(f'upstream_paths = ["{path}"]')
            print('last_synced_commit = ""')
            print('parity_test_targets = ["./gradlew compileAll"]')
            print()
    else:
        print("Unmapped module candidates:")
        for path in missing:
            print(f"  - {path}")
        print(
            f"\nTotal unmapped: {len(missing)}. "
            "Use --toml-stubs to print module entry templates."
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
