#!/usr/bin/env python3
"""Scaffold missing UPSTREAM.md/PARITY.md from upstream-map/modules.toml."""

from __future__ import annotations

import argparse
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = ROOT / "architecture" / "upstream-map" / "modules.toml"


def load_modules() -> list[dict]:
    data = tomllib.loads(MAP_PATH.read_text(encoding="utf-8"))
    modules = data.get("module", [])
    if not isinstance(modules, list):
        raise ValueError(f"{MAP_PATH} has invalid 'module' entries")
    return modules


def render_upstream(module: dict) -> str:
    upstream_paths = module.get("upstream_paths", [])
    upstream_list = "\n".join(f"- `{path}`" for path in upstream_paths)
    return f"""# Upstream Mapping

## Module

- `module_id`: `{module["id"]}`
- `module_path`: `{module["path"]}`
- `owner`: `{module["owner"]}`

## Upstream Source Paths

{upstream_list}

## Last Synced Upstream Commit

- `commit`: _pending_
- `sync_date_utc`: _pending_

## Sync Notes

- Auto-generated from `architecture/upstream-map/modules.toml`.
- Record upstream behavior changes and how they were ported.
"""


def render_parity(module: dict) -> str:
    parity_targets = module.get("parity_test_targets", [])
    parity_list = "\n".join(f"- `{target}`" for target in parity_targets)
    return f"""# Parity Status

## Module

- `module_id`: `{module["id"]}`
- `module_path`: `{module["path"]}`
- `status`: `{module["status"]}`

## Supported

- Document behavior that currently matches upstream.

## Gaps

- Document known behavior gaps against upstream.

## Validation

{parity_list}

## Next Sync Actions

1. Fill in concrete parity updates and upstream sync commits.
2. Add module-specific differential tests where applicable.
3. Keep this file current on every sync PR.
"""


def write_if_missing(path: Path, content: str, check: bool) -> bool:
    if path.exists():
        return False
    if check:
        return True
    path.write_text(content, encoding="utf-8")
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="Fail if scaffolding would create missing metadata files.",
    )
    args = parser.parse_args()

    modules = load_modules()
    created_or_missing: list[Path] = []

    for module in modules:
        module_dir = ROOT / module["path"]
        if not module_dir.exists() or not module_dir.is_dir():
            continue

        upstream_file = module_dir / "UPSTREAM.md"
        parity_file = module_dir / "PARITY.md"

        if write_if_missing(upstream_file, render_upstream(module), args.check):
            created_or_missing.append(upstream_file)
        if write_if_missing(parity_file, render_parity(module), args.check):
            created_or_missing.append(parity_file)

    if args.check and created_or_missing:
        print("Missing metadata files detected:")
        for file_path in created_or_missing:
            print(f"  - {file_path}")
        print("Run: python3 tools/upstream_map/sync_metadata.py")
        return 1

    if created_or_missing:
        action = "Missing (check mode)" if args.check else "Created"
        print(f"{action} {len(created_or_missing)} metadata files.")
    else:
        print("Metadata scaffolding already up to date.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
