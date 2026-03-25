#!/usr/bin/env python3
"""Validate Rust migration upstream map and module parity metadata."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = ROOT / "architecture" / "upstream-map" / "modules.toml"

REQUIRED_UPSTREAM_SECTIONS = [
    "# Upstream Mapping",
    "## Module",
    "## Upstream Source Paths",
    "## Last Synced Upstream Commit",
    "## Sync Notes",
]

REQUIRED_PARITY_SECTIONS = [
    "# Parity Status",
    "## Module",
    "## Supported",
    "## Gaps",
    "## Validation",
    "## Next Sync Actions",
]

VALID_STATUSES = {
    "planned",
    "jvm-compat",
    "mixed",
    "native-kernel",
    "native",
}


def load_map() -> dict:
    if not MAP_PATH.exists():
        raise FileNotFoundError(f"Missing map file: {MAP_PATH}")
    return tomllib.loads(MAP_PATH.read_text(encoding="utf-8"))


def check_required_sections(file_path: Path, required_sections: list[str]) -> list[str]:
    text = file_path.read_text(encoding="utf-8")
    return [section for section in required_sections if section not in text]


def main() -> int:
    errors: list[str] = []

    try:
        data = load_map()
    except Exception as exc:  # noqa: BLE001
        print(f"ERROR: failed to parse {MAP_PATH}: {exc}", file=sys.stderr)
        return 1

    schema_version = data.get("schema_version")
    if schema_version != 1:
        errors.append(
            f"schema_version must be 1, found {schema_version!r} in {MAP_PATH}"
        )

    modules = data.get("module")
    if not isinstance(modules, list) or not modules:
        errors.append(f"{MAP_PATH} must define at least one [[module]] entry")
        modules = []
    else:
        module_paths = [m.get("path") for m in modules if isinstance(m, dict)]
        sorted_paths = sorted(module_paths)
        if module_paths != sorted_paths:
            errors.append(
                f"{MAP_PATH} module entries must be sorted by 'path'. "
                "Run a sort pass before committing."
            )

    ids: set[str] = set()
    paths: set[str] = set()
    upstream_paths: dict[str, str] = {}
    mapped_paths: set[str] = set()

    for idx, module in enumerate(modules, start=1):
        label = f"module[{idx}]"
        if not isinstance(module, dict):
            errors.append(f"{label} is not a table")
            continue

        module_id = module.get("id")
        module_path = module.get("path")
        owner = module.get("owner")
        status = module.get("status")
        module_upstream_paths = module.get("upstream_paths")
        parity_targets = module.get("parity_test_targets")

        for key, value in {
            "id": module_id,
            "path": module_path,
            "owner": owner,
            "status": status,
        }.items():
            if not isinstance(value, str) or not value.strip():
                errors.append(f"{label}.{key} must be a non-empty string")

        if isinstance(module_id, str):
            if module_id in ids:
                errors.append(f"duplicate module id: {module_id}")
            ids.add(module_id)

        if isinstance(module_path, str):
            if module_path in paths:
                errors.append(f"duplicate module path: {module_path}")
            paths.add(module_path)
            mapped_paths.add(module_path)

        if isinstance(status, str) and status not in VALID_STATUSES:
            errors.append(
                f"{label}.status '{status}' is invalid; expected one of {sorted(VALID_STATUSES)}"
            )

        if not isinstance(module_upstream_paths, list) or not module_upstream_paths:
            errors.append(f"{label}.upstream_paths must be a non-empty array of strings")
        else:
            for upstream_path in module_upstream_paths:
                if not isinstance(upstream_path, str) or not upstream_path.strip():
                    errors.append(f"{label}.upstream_paths contains a non-string value")
                    continue
                previous = upstream_paths.get(upstream_path)
                if previous and previous != module_id:
                    errors.append(
                        f"upstream path '{upstream_path}' is mapped by both {previous} and {module_id}"
                    )
                else:
                    upstream_paths[upstream_path] = module_id

        if not isinstance(parity_targets, list) or not parity_targets:
            errors.append(f"{label}.parity_test_targets must be a non-empty array")
        else:
            for target in parity_targets:
                if not isinstance(target, str) or not target.strip():
                    errors.append(f"{label}.parity_test_targets contains an invalid item")

        if isinstance(module_path, str):
            module_dir = ROOT / module_path
            if not module_dir.exists() or not module_dir.is_dir():
                errors.append(f"{label}.path does not exist or is not a directory: {module_path}")
                continue

            upstream_file = module_dir / "UPSTREAM.md"
            parity_file = module_dir / "PARITY.md"

            if not upstream_file.exists():
                errors.append(f"missing {upstream_file}")
            else:
                missing_sections = check_required_sections(
                    upstream_file, REQUIRED_UPSTREAM_SECTIONS
                )
                for section in missing_sections:
                    errors.append(f"{upstream_file} is missing section: {section}")
                upstream_text = upstream_file.read_text(encoding="utf-8")
                if isinstance(module_id, str) and module_id not in upstream_text:
                    errors.append(f"{upstream_file} does not mention module id '{module_id}'")
                if isinstance(module_path, str) and module_path not in upstream_text:
                    errors.append(f"{upstream_file} does not mention module path '{module_path}'")

            if not parity_file.exists():
                errors.append(f"missing {parity_file}")
            else:
                missing_sections = check_required_sections(
                    parity_file, REQUIRED_PARITY_SECTIONS
                )
                for section in missing_sections:
                    errors.append(f"{parity_file} is missing section: {section}")
                parity_text = parity_file.read_text(encoding="utf-8")
                if isinstance(module_id, str) and module_id not in parity_text:
                    errors.append(f"{parity_file} does not mention module id '{module_id}'")
                if isinstance(module_path, str) and module_path not in parity_text:
                    errors.append(f"{parity_file} does not mention module path '{module_path}'")

    # Detect tracked metadata pairs that are not represented in modules.toml.
    for upstream_file in ROOT.rglob("UPSTREAM.md"):
        if "architecture/upstream-map/templates" in upstream_file.as_posix():
            continue
        module_dir = upstream_file.parent
        parity_file = module_dir / "PARITY.md"
        rel_module_dir = module_dir.relative_to(ROOT).as_posix()
        if parity_file.exists() and rel_module_dir not in mapped_paths:
            errors.append(
                f"metadata drift: {rel_module_dir} has UPSTREAM/PARITY docs but no modules.toml entry"
            )

    if errors:
        print("Upstream map validation failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(
        f"Upstream map validation passed: {len(modules)} modules, "
        f"{len(upstream_paths)} upstream path mappings."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
