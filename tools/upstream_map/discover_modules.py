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


def java_rust_map() -> int:
    """Cross-reference Java upstream modules with Rust server files."""
    import tomllib

    data = tomllib.loads(MAP_PATH.read_text(encoding="utf-8"))
    modules = data.get("module", [])

    rust_server_dir = ROOT / "substrate" / "src" / "server"
    rust_files = {f.stem for f in rust_server_dir.glob("*.rs")} if rust_server_dir.exists() else set()

    mapped_java: list[str] = []
    unmapped_java: list[str] = []
    rust_without_java: list[str] = []

    for module in modules:
        path = module.get("path", "")
        if not path:
            continue
        upstream = module.get("upstream_paths", [])
        if not upstream:
            continue

        # Derive expected Rust file name from module path
        # e.g. "platforms/core-execution/hashing" -> "hash"
        rust_name = path.split("/")[-1].replace("-", "_")
        # Try common mappings
        candidates = [
            rust_name,
            rust_name.replace("execution", "exec"),
            rust_name.replace("file_watching", "file_watch"),
            rust_name.replace("persistent_cache", "cache"),
            rust_name.replace("configuration_cache", "config_cache"),
        ]

        has_rust = any(c in rust_files for c in candidates)
        if has_rust:
            mapped_java.append(path)
        else:
            unmapped_java.append(path)

    # Rust files without a Java module mapping
    java_names = set()
    for module in modules:
        path = module.get("path", "")
        java_names.add(path.split("/")[-1].replace("-", "_"))

    for rf in sorted(rust_files):
        if rf in ("mod", "scopes", "authoritative", "control", "bootstrap",
                   "build_script_types", "groovy_parser", "task_executor"):
            continue  # Internal/non-service files
        # Check if any module maps to this Rust file
        found = False
        for module in modules:
            path = module.get("path", "")
            rust_name = path.split("/")[-1].replace("-", "_")
            if rust_name == rf or rust_name.replace("execution", "exec") == rf:
                found = True
                break
        if not found:
            rust_without_java.append(rf)

    print("Java-to-Rust mapping:")
    print(f"  Mapped:   {len(mapped_java)} modules have Rust equivalents")
    print(f"  Unmapped: {len(unmapped_java)} modules lack Rust equivalents")
    print(f"  Orphaned: {len(rust_without_java)} Rust files have no module mapping")

    if unmapped_java:
        print("\nUnmapped Java modules:")
        for p in unmapped_java:
            print(f"  - {p}")

    if rust_without_java:
        print("\nRust files without module mapping:")
        for f in rust_without_java:
            print(f"  - {f}.rs")

    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--toml-stubs",
        action="store_true",
        help="Print TOML stubs for discovered unmapped directories.",
    )
    parser.add_argument(
        "--java-rust-map",
        action="store_true",
        help="Cross-reference Java upstream modules with Rust server files.",
    )
    args = parser.parse_args()

    if args.java_rust_map:
        return java_rust_map()

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
