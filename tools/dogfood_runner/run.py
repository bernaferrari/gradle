#!/usr/bin/env python3
"""Validate and enumerate Rust substrate dogfood manifests.

The dogfood runner is intentionally thin at first: it defines a stable manifest
contract that can be consumed by heavier upstream-vs-Rust execution later.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "testing" / "dogfood" / "manifest.json"
SUPPORTED_MODES = {"strict", "native-ready-default"}
SUPPORTED_EXPECTATIONS = {"supported", "fail-closed"}


@dataclass(frozen=True)
class DogfoodProject:
    name: str
    path: Path
    tasks: list[str]
    mode: str
    expectation: str
    timeout_seconds: int
    reason: str
    checks: dict[str, Any]
    source: dict[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "path": str(self.path),
            "tasks": self.tasks,
            "mode": self.mode,
            "expectation": self.expectation,
            "timeout_seconds": self.timeout_seconds,
            "reason": self.reason,
            "checks": self.checks,
            "source": self.source,
        }


def load_manifest(manifest_path: Path) -> tuple[dict[str, Any], list[DogfoodProject]]:
    manifest_path = manifest_path.resolve()
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    root = Path(data.get("root", "."))
    if not root.is_absolute():
        root = (manifest_path.parent / root).resolve()

    projects = []
    for entry in data.get("projects", []):
        project_path = Path(entry["path"])
        if not project_path.is_absolute():
            project_path = root / project_path
        projects.append(
            DogfoodProject(
                name=entry["name"],
                path=project_path.resolve(),
                tasks=list(entry.get("tasks", [])),
                mode=entry.get("mode", ""),
                expectation=entry.get("expectation", ""),
                timeout_seconds=int(entry.get("timeout_seconds", 0)),
                reason=entry.get("reason", ""),
                checks=dict(entry.get("checks", {})),
                source=dict(entry.get("source", {})),
            )
        )
    return data, projects


def validate_manifest(manifest_path: Path) -> list[str]:
    data, projects = load_manifest(manifest_path)
    errors: list[str] = []
    if data.get("schema") != "gradle-substrate.dogfood-manifest.v1":
        errors.append("schema must be gradle-substrate.dogfood-manifest.v1")
    if len(projects) < 5:
        errors.append(f"manifest must list at least 5 projects, got {len(projects)}")

    names: set[str] = set()
    supported_count = 0
    fail_closed_count = 0
    for project in projects:
        prefix = f"{project.name}: "
        if project.name in names:
            errors.append(prefix + "duplicate project name")
        names.add(project.name)
        if not project.path.exists():
            errors.append(prefix + f"path does not exist: {project.path}")
        if not project.tasks:
            errors.append(prefix + "tasks must be non-empty")
        if project.mode not in SUPPORTED_MODES:
            errors.append(prefix + f"unsupported mode {project.mode!r}")
        if project.expectation not in SUPPORTED_EXPECTATIONS:
            errors.append(prefix + f"unsupported expectation {project.expectation!r}")
        if project.timeout_seconds <= 0:
            errors.append(prefix + "timeout_seconds must be positive")
        if not project.reason:
            errors.append(prefix + "reason must be non-empty")
        if not project.source.get("kind") or not project.source.get("description"):
            errors.append(prefix + "source.kind and source.description are required")
        if project.expectation == "supported":
            supported_count += 1
            if not project.checks.get("task_parity"):
                errors.append(prefix + "supported entries must require task_parity")
            if not project.checks.get("no_jvm_forwards"):
                errors.append(prefix + "supported entries must require no_jvm_forwards")
        if project.expectation == "fail-closed":
            fail_closed_count += 1
            if not project.checks.get("fail_closed_diagnostic"):
                errors.append(prefix + "fail-closed entries must name fail_closed_diagnostic")

    if supported_count < 3:
        errors.append(f"manifest must list at least 3 supported projects, got {supported_count}")
    if fail_closed_count < 1:
        errors.append("manifest must list at least 1 fail-closed project")
    return errors


def enumerate_manifest(manifest_path: Path) -> dict[str, Any]:
    data, projects = load_manifest(manifest_path)
    errors = validate_manifest(manifest_path)
    return {
        "schema": data.get("schema"),
        "manifest": str(manifest_path.resolve()),
        "valid": not errors,
        "errors": errors,
        "project_count": len(projects),
        "supported_count": sum(1 for project in projects if project.expectation == "supported"),
        "fail_closed_count": sum(1 for project in projects if project.expectation == "fail-closed"),
        "projects": [project.to_dict() for project in projects],
    }


def create_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST), help="Dogfood manifest path")
    parser.add_argument("--list", action="store_true", help="List projects as a table")
    parser.add_argument("--json", action="store_true", help="Emit JSON")
    parser.add_argument("--validate-only", action="store_true", help="Validate the manifest and exit")
    return parser


def print_table(summary: dict[str, Any]) -> None:
    print(f"Manifest: {summary['manifest']}")
    print(
        f"Projects: {summary['project_count']} "
        f"({summary['supported_count']} supported, {summary['fail_closed_count']} fail-closed)"
    )
    print()
    print(f"{'name':32} {'mode':20} {'expectation':12} tasks")
    print("-" * 86)
    for project in summary["projects"]:
        print(
            f"{project['name'][:32]:32} "
            f"{project['mode'][:20]:20} "
            f"{project['expectation'][:12]:12} "
            f"{' '.join(project['tasks'])}"
        )


def main(argv: list[str] | None = None) -> int:
    args = create_arg_parser().parse_args(argv)
    summary = enumerate_manifest(Path(args.manifest))
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print_table(summary)
    if summary["errors"]:
        for error in summary["errors"]:
            print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
