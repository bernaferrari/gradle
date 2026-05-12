#!/usr/bin/env python3
"""Validate and enumerate Rust substrate dogfood manifests.

The dogfood runner is intentionally thin at first: it defines a stable manifest
contract that can be consumed by heavier upstream-vs-Rust execution later.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "testing" / "dogfood" / "manifest.json"
CORPUS_RUNNER_PATH = REPO_ROOT / "tools" / "corpus_runner" / "run.py"
SUPPORTED_MODES = {"strict", "native-ready-default"}
SUPPORTED_EXPECTATIONS = {"supported", "fail-closed"}


def load_corpus_runner():
    spec = importlib.util.spec_from_file_location("dogfood_corpus_runner", CORPUS_RUNNER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load corpus runner from {CORPUS_RUNNER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


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


def parse_jvm_forwards(output: str) -> int:
    match = re.search(r"jvmForwarded=(\d+)", output)
    if match:
        return int(match.group(1))
    if "JVM forwarding disabled" in output or "JVM fallback disabled" in output:
        return 0
    return -1


def run_project(
    project: DogfoodProject,
    output_dir: Path,
    gradle_command: str | None,
    daemon_binary: str | None,
    verbose: bool = False,
) -> dict[str, Any]:
    corpus = load_corpus_runner()
    runbuild_authoritative = project.mode == "strict"
    runbuild_native_ready_default = project.mode == "native-ready-default"
    project_output_dir = output_dir / project.name
    project_output_dir.mkdir(parents=True, exist_ok=True)

    upstream = corpus.run_build(
        str(project.path),
        substrate=False,
        timeout=project.timeout_seconds,
        tasks=project.tasks,
        gradle_command=gradle_command,
    )
    substrate = corpus.run_build(
        str(project.path),
        substrate=True,
        timeout=project.timeout_seconds,
        tasks=project.tasks,
        daemon_binary=daemon_binary,
        runbuild_authoritative=runbuild_authoritative,
        runbuild_native_ready_default=runbuild_native_ready_default,
        gradle_command=gradle_command,
    )

    if project.expectation == "fail-closed":
        checks = corpus.compare_expected_fail_closed(upstream, substrate)
        expected_marker = str(project.checks.get("fail_closed_diagnostic", ""))
        if expected_marker:
            checks["expected_diagnostic_present"] = expected_marker in substrate.output
            checks["match"] = checks["match"] and checks["expected_diagnostic_present"]
    else:
        checks = corpus.compare_run_pair(upstream, substrate)
        jvm_forwards = parse_jvm_forwards(substrate.output)
        checks["jvm_forward_count"] = jvm_forwards
        if project.checks.get("no_jvm_forwards"):
            checks["no_jvm_forwards"] = jvm_forwards == 0
            checks["match"] = checks["match"] and checks["no_jvm_forwards"]

    result = {
        "name": project.name,
        "path": str(project.path),
        "mode": project.mode,
        "expectation": project.expectation,
        "tasks": project.tasks,
        "reason": project.reason,
        "upstream": upstream.to_dict(),
        "substrate": substrate.to_dict(),
        "checks": checks,
        "match": checks["match"],
    }
    (project_output_dir / "result.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if verbose:
        print(f"{project.name}: {'PASS' if result['match'] else 'FAIL'}")
    return result


def summarize_execution(results: list[dict[str, Any]]) -> dict[str, Any]:
    supported = [result for result in results if result["expectation"] == "supported"]
    fail_closed = [result for result in results if result["expectation"] == "fail-closed"]
    return {
        "schema": "gradle-substrate.dogfood-summary.v1",
        "project_count": len(results),
        "matched_project_count": sum(1 for result in results if result["match"]),
        "supported_project_count": len(supported),
        "supported_matched_count": sum(1 for result in supported if result["match"]),
        "fail_closed_project_count": len(fail_closed),
        "fail_closed_matched_count": sum(1 for result in fail_closed if result["match"]),
        "zero_jvm_forward_supported_count": sum(
            1
            for result in supported
            if result.get("checks", {}).get("jvm_forward_count") == 0
        ),
        "upstream_duration_ms": sum(result["upstream"].get("duration_ms", 0) for result in results),
        "substrate_duration_ms": sum(result["substrate"].get("duration_ms", 0) for result in results),
        "upstream_task_total": sum(result["upstream"].get("task_count", 0) for result in results),
        "substrate_task_total": sum(result["substrate"].get("task_count", 0) for result in results),
        "failed_projects": [result["name"] for result in results if not result["match"]],
    }


def write_markdown_report(output_dir: Path, summary: dict[str, Any], results: list[dict[str, Any]]) -> Path:
    path = output_dir / "dogfood-summary.md"
    lines = [
        "# Rust Substrate Dogfood Summary",
        "",
        f"Projects matched: {summary['matched_project_count']}/{summary['project_count']}",
        f"Supported projects matched: {summary['supported_matched_count']}/{summary['supported_project_count']}",
        f"Fail-closed projects matched: {summary['fail_closed_matched_count']}/{summary['fail_closed_project_count']}",
        f"Supported projects with zero JVM forwards: {summary['zero_jvm_forward_supported_count']}/{summary['supported_project_count']}",
        f"Observed wall time: upstream={summary['upstream_duration_ms']}ms, substrate={summary['substrate_duration_ms']}ms",
        f"Task totals: upstream={summary['upstream_task_total']}, substrate={summary['substrate_task_total']}",
        "",
        "| Project | Expectation | Mode | Result | Upstream ms | Substrate ms | JVM forwards |",
        "| --- | --- | --- | --- | ---: | ---: | ---: |",
    ]
    for result in results:
        checks = result.get("checks", {})
        jvm_forwards = checks.get("jvm_forward_count", "")
        lines.append(
            "| {name} | {expectation} | {mode} | {status} | {upstream_ms} | {substrate_ms} | {jvm_forwards} |".format(
                name=result["name"],
                expectation=result["expectation"],
                mode=result["mode"],
                status="PASS" if result["match"] else "FAIL",
                upstream_ms=result["upstream"].get("duration_ms", 0),
                substrate_ms=result["substrate"].get("duration_ms", 0),
                jvm_forwards=jvm_forwards,
            )
        )
    lines.extend([
        "",
        "This report is dogfood evidence for the documented preview surface, not a 100% Gradle compatibility claim.",
    ])
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def execute_manifest(
    manifest_path: Path,
    output_dir: Path,
    gradle_command: str | None,
    daemon_binary: str | None,
    verbose: bool = False,
) -> dict[str, Any]:
    errors = validate_manifest(manifest_path)
    if errors:
        return {"valid": False, "errors": errors, "results": [], "summary": {}}
    output_dir.mkdir(parents=True, exist_ok=True)
    _data, projects = load_manifest(manifest_path)
    results = [
        run_project(project, output_dir, gradle_command, daemon_binary, verbose=verbose)
        for project in projects
    ]
    summary = summarize_execution(results)
    report_path = write_markdown_report(output_dir, summary, results)
    payload = {
        "valid": True,
        "errors": [],
        "manifest": str(manifest_path.resolve()),
        "output_dir": str(output_dir.resolve()),
        "report": str(report_path.resolve()),
        "summary": summary,
        "results": results,
    }
    (output_dir / "dogfood-results.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return payload


def create_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST), help="Dogfood manifest path")
    parser.add_argument("--list", action="store_true", help="List projects as a table")
    parser.add_argument("--json", action="store_true", help="Emit JSON")
    parser.add_argument("--validate-only", action="store_true", help="Validate the manifest and exit")
    parser.add_argument("--execute", action="store_true", help="Execute upstream and Rust substrate dogfood runs")
    parser.add_argument("--output-dir", default="build/dogfood", help="Directory for execution results")
    parser.add_argument("--gradle-command", default=None, help="Gradle-under-test executable")
    parser.add_argument("--daemon-binary", default="target/debug/gradle-substrate-daemon", help="Rust daemon binary")
    parser.add_argument("--verbose", action="store_true", help="Print per-project execution status")
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
    if args.execute:
        payload = execute_manifest(
            Path(args.manifest),
            Path(args.output_dir),
            gradle_command=args.gradle_command,
            daemon_binary=args.daemon_binary,
            verbose=args.verbose,
        )
        if args.json:
            print(json.dumps(payload, indent=2, sort_keys=True))
        else:
            summary = payload.get("summary", {})
            print(f"Results: {summary.get('matched_project_count', 0)}/{summary.get('project_count', 0)} matched")
            if payload.get("report"):
                print(f"Report: {payload['report']}")
        return 0 if payload.get("valid") and not payload.get("summary", {}).get("failed_projects") else 1

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
