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
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "testing" / "dogfood" / "manifest.json"
CORPUS_RUNNER_PATH = REPO_ROOT / "tools" / "corpus_runner" / "run.py"
SUPPORTED_MODES = {"strict", "native-ready-default"}
SUPPORTED_EXPECTATIONS = {"supported", "fail-closed", "supported-or-fail-closed"}


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
    coverage_tags: list[str] = field(default_factory=list)

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
            "coverage_tags": self.coverage_tags,
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
                coverage_tags=sorted(str(tag) for tag in entry.get("coverage_tags", [])),
            )
        )
    return data, projects


def validate_manifest(manifest_path: Path) -> list[str]:
    data, projects = load_manifest(manifest_path)
    errors: list[str] = []
    if data.get("schema") != "gradle-substrate.dogfood-manifest.v1":
        errors.append("schema must be gradle-substrate.dogfood-manifest.v1")
    minimum_projects = int(data.get("minimum_projects", 5))
    if len(projects) < minimum_projects:
        errors.append(
            f"manifest must list at least {minimum_projects} projects, got {len(projects)}"
        )

    names: set[str] = set()
    supported_count = 0
    fail_closed_count = 0
    for project in projects:
        prefix = f"{project.name}: "
        if project.name in names:
            errors.append(prefix + "duplicate project name")
        names.add(project.name)
        source_kind = project.source.get("kind")
        if source_kind == "git":
            ref = str(project.source.get("ref", ""))
            if not project.source.get("url"):
                errors.append(prefix + "git source requires url")
            if not is_immutable_git_ref(ref):
                errors.append(prefix + "git source requires immutable 40-character ref")
            if "subdir" not in project.source:
                errors.append(prefix + "git source requires subdir")
        elif not project.path.exists():
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
        if any(not tag for tag in project.coverage_tags):
            errors.append(prefix + "coverage_tags must be non-empty strings")
        if project.expectation in {"supported", "supported-or-fail-closed"}:
            supported_count += 1
            if not project.checks.get("task_parity"):
                errors.append(prefix + "supported entries must require task_parity")
            if not project.checks.get("no_jvm_forwards"):
                errors.append(prefix + "supported entries must require no_jvm_forwards")
        if project.expectation in {"fail-closed", "supported-or-fail-closed"}:
            fail_closed_count += 1
            if not project.checks.get("fail_closed_diagnostic"):
                errors.append(prefix + "fail-closed entries must name fail_closed_diagnostic")

    minimum_supported = int(data.get("minimum_supported_projects", 3))
    minimum_fail_closed = int(data.get("minimum_fail_closed_projects", 1))
    if supported_count < minimum_supported:
        errors.append(
            f"manifest must list at least {minimum_supported} supported projects, got {supported_count}"
        )
    if fail_closed_count < minimum_fail_closed:
        errors.append(
            f"manifest must list at least {minimum_fail_closed} fail-closed projects, got {fail_closed_count}"
        )
    return errors


def is_immutable_git_ref(ref: str) -> bool:
    return bool(re.fullmatch(r"[0-9a-fA-F]{40}", ref))


def git_fetch_commands(project: DogfoodProject, source_cache_dir: Path) -> list[list[str]]:
    source = project.source
    clone_dir = source_cache_dir / project.name
    url = str(source["url"])
    ref = str(source["ref"])
    return [
        ["git", "clone", "--no-checkout", url, str(clone_dir)],
        ["git", "-C", str(clone_dir), "fetch", "--depth", "1", "origin", ref],
        ["git", "-C", str(clone_dir), "checkout", "--detach", ref],
    ]


def fetch_git_project(project: DogfoodProject, source_cache_dir: Path) -> dict[str, Any]:
    if project.source.get("kind") != "git":
        return {
            "name": project.name,
            "source_kind": project.source.get("kind", ""),
            "path": str(project.path),
            "fetched": False,
            "cached": False,
            "success": True,
            "message": "checked-in source does not require fetch",
        }

    clone_dir = source_cache_dir / project.name
    ref = str(project.source["ref"])
    source_cache_dir.mkdir(parents=True, exist_ok=True)
    commands = git_fetch_commands(project, source_cache_dir)
    try:
        if not (clone_dir / ".git").exists():
            subprocess.run(commands[0], check=True, capture_output=True, text=True)
            fetched = True
        else:
            fetched = False
        subprocess.run(commands[1], check=True, capture_output=True, text=True)
        subprocess.run(commands[2], check=True, capture_output=True, text=True)
        actual_ref = subprocess.check_output(
            ["git", "-C", str(clone_dir), "rev-parse", "HEAD"],
            text=True,
        ).strip()
        if actual_ref.lower() != ref.lower():
            return {
                "name": project.name,
                "source_kind": "git",
                "path": str(clone_dir),
                "fetched": fetched,
                "cached": not fetched,
                "success": False,
                "message": f"checked out {actual_ref}, expected {ref}",
            }
        return {
            "name": project.name,
            "source_kind": "git",
            "path": str(clone_dir),
            "fetched": fetched,
            "cached": not fetched,
            "success": True,
            "message": "fetched",
        }
    except subprocess.CalledProcessError as error:
        return {
            "name": project.name,
            "source_kind": "git",
            "path": str(clone_dir),
            "fetched": False,
            "cached": False,
            "success": False,
            "message": (error.stderr or error.stdout or str(error)).strip(),
        }


def materialized_project(project: DogfoodProject, source_cache_dir: Path) -> DogfoodProject:
    if project.source.get("kind") != "git":
        return project
    subdir = str(project.source.get("subdir", "."))
    path = source_cache_dir / project.name
    if subdir and subdir != ".":
        path = path / subdir
    return DogfoodProject(
        name=project.name,
        path=path.resolve(),
        tasks=project.tasks,
        mode=project.mode,
        expectation=project.expectation,
        timeout_seconds=project.timeout_seconds,
        reason=project.reason,
        checks=project.checks,
        source=project.source,
        coverage_tags=project.coverage_tags,
    )


def fetch_manifest(manifest_path: Path, source_cache_dir: Path) -> dict[str, Any]:
    errors = validate_manifest(manifest_path)
    if errors:
        return {"valid": False, "errors": errors, "fetches": []}
    _data, projects = load_manifest(manifest_path)
    fetches = [fetch_git_project(project, source_cache_dir) for project in projects]
    return {
        "valid": True,
        "errors": [],
        "source_cache_dir": str(source_cache_dir.resolve()),
        "fetches": fetches,
        "success": all(fetch["success"] for fetch in fetches),
    }


def enumerate_manifest(manifest_path: Path) -> dict[str, Any]:
    data, projects = load_manifest(manifest_path)
    errors = validate_manifest(manifest_path)
    return {
        "schema": data.get("schema"),
        "manifest": str(manifest_path.resolve()),
        "valid": not errors,
        "errors": errors,
        "project_count": len(projects),
        "supported_count": sum(
            1 for project in projects if project.expectation in {"supported", "supported-or-fail-closed"}
        ),
        "fail_closed_count": sum(
            1 for project in projects if project.expectation in {"fail-closed", "supported-or-fail-closed"}
        ),
        "projects": [project.to_dict() for project in projects],
    }


def parse_jvm_forwards(output: str) -> int:
    match = re.search(r"jvmForwarded=(\d+)", output)
    if match:
        return int(match.group(1))
    if "JVM forwarding disabled" in output or "JVM fallback disabled" in output:
        return 0
    return -1


def parse_substrate_signals(output: str) -> dict[str, Any]:
    plan_match = re.search(r"\[substrate:run-build\].*? from ([A-Za-z0-9_.-]+)", output)
    rust_executed_match = re.search(r"\[substrate:run-build\] Rust executed (\d+)", output)
    bootstrap_durations = [
        int(match)
        for match in re.findall(
            r"\[substrate:bootstrap\] build [^ ]+ completed \([^,]+, (\d+)ms, acked=true\)",
            output,
        )
    ]
    return {
        "daemon_started": "Daemon started successfully" in output,
        "daemon_reused": "Connecting to existing daemon" in output
        and "Failed to connect to existing daemon" not in output,
        "runbuild_marker": "[substrate:run-build]" in output,
        "rust_executed_tasks": int(rust_executed_match.group(1)) if rust_executed_match else 0,
        "taskgraph_captured": "[substrate:taskgraph] captured" in output,
        "plan_source": plan_match.group(1) if plan_match else "",
        "jvm_forward_count": parse_jvm_forwards(output),
        "rust_bootstrap_duration_ms": sum(bootstrap_durations),
        "rust_bootstrap_completion_count": len(bootstrap_durations),
    }


def run_project(
    project: DogfoodProject,
    output_dir: Path,
    gradle_command: str | None,
    daemon_binary: str | None,
    shared_substrate_state_dir: Path | None = None,
    verbose: bool = False,
) -> dict[str, Any]:
    corpus = load_corpus_runner()
    runbuild_authoritative = project.mode == "strict"
    runbuild_native_ready_default = project.mode == "native-ready-default"
    project_output_dir = output_dir / project.name
    project_output_dir.mkdir(parents=True, exist_ok=True)
    upstream_project_path = project.path
    substrate_project_path = project.path
    if project.source.get("kind") == "git":
        upstream_project_path = prepare_project_run_dir(
            project.path,
            project_output_dir / "upstream-work",
        )
        substrate_project_path = prepare_project_run_dir(
            project.path,
            project_output_dir / "substrate-work",
        )

    upstream = corpus.run_build(
        str(upstream_project_path),
        substrate=False,
        timeout=project.timeout_seconds,
        tasks=project.tasks,
        gradle_command=gradle_command,
    )
    substrate = corpus.run_build(
        str(substrate_project_path),
        substrate=True,
        timeout=project.timeout_seconds,
        tasks=project.tasks,
        daemon_binary=daemon_binary,
        runbuild_authoritative=runbuild_authoritative,
        runbuild_native_ready_default=runbuild_native_ready_default,
        gradle_command=gradle_command,
        extra_gradle_args=(
            [f"-Dorg.gradle.rust.substrate.state.dir={shared_substrate_state_dir}"]
            if shared_substrate_state_dir is not None
            else None
        ),
    )

    if project.expectation == "fail-closed":
        checks = corpus.compare_expected_fail_closed(upstream, substrate)
        expected_marker = str(project.checks.get("fail_closed_diagnostic", ""))
        if expected_marker:
            checks["expected_diagnostic_present"] = expected_marker in substrate.output
            checks["match"] = checks["match"] and checks["expected_diagnostic_present"]
    elif project.expectation == "supported-or-fail-closed" and substrate.exit_code != 0:
        checks = corpus.compare_expected_fail_closed(upstream, substrate)
        expected_marker = str(project.checks.get("fail_closed_diagnostic", ""))
        if expected_marker and expected_marker != "required-if-not-supported":
            checks["expected_diagnostic_present"] = expected_marker in substrate.output
            checks["match"] = checks["match"] and checks["expected_diagnostic_present"]
        checks["resolved_expectation"] = "fail-closed"
    else:
        checks = corpus.compare_run_pair(upstream, substrate)
        ignored_hashes = ignored_hash_differences(
            upstream.output_hashes,
            substrate.output_hashes,
            project.checks.get("ignore_output_hash_patterns", []),
        )
        if ignored_hashes:
            filtered_upstream_hashes = {
                path: hash_value
                for path, hash_value in upstream.output_hashes.items()
                if path not in ignored_hashes
            }
            filtered_substrate_hashes = {
                path: hash_value
                for path, hash_value in substrate.output_hashes.items()
                if path not in ignored_hashes
            }
            checks["ignored_output_hash_differences"] = sorted(ignored_hashes)
            checks["output_hashes_match"] = filtered_upstream_hashes == filtered_substrate_hashes
            checks["match"] = (
                checks["substrate_usable"]
                and checks["successful"]
                and checks["exit_code_match"]
                and checks["task_list_match"]
                and checks["output_files_match"]
                and checks["output_hashes_match"]
                and checks["archive_entries_match"]
            )
        if (
            not checks["task_list_match"]
            and project.checks.get("task_order") is False
            and sorted(upstream.tasks) == sorted(substrate.tasks)
        ):
            checks["task_list_match"] = True
            checks["task_set_match"] = True
            checks["match"] = (
                checks["substrate_usable"]
                and checks["successful"]
                and checks["exit_code_match"]
                and checks["task_list_match"]
                and checks["output_files_match"]
                and checks["output_hashes_match"]
                and checks["archive_entries_match"]
            )
        jvm_forwards = parse_jvm_forwards(substrate.output)
        checks["jvm_forward_count"] = jvm_forwards
        if project.checks.get("no_jvm_forwards"):
            checks["no_jvm_forwards"] = jvm_forwards == 0
            checks["match"] = checks["match"] and checks["no_jvm_forwards"]
        if project.expectation == "supported-or-fail-closed":
            checks["resolved_expectation"] = "supported"

    substrate_signals = parse_substrate_signals(substrate.output)
    substrate_signals["non_rust_overhead_ms"] = max(
        0,
        substrate.duration_ms - int(substrate_signals.get("rust_bootstrap_duration_ms", 0)),
    )
    if checks.get("resolved_expectation", project.expectation) == "supported":
        checks["rust_runbuild_executed"] = substrate_signals["rust_executed_tasks"] > 0
        checks["match"] = checks["match"] and checks["rust_runbuild_executed"]
    result = {
        "name": project.name,
        "path": str(project.path),
        "mode": project.mode,
        "expectation": project.expectation,
        "tasks": project.tasks,
        "reason": project.reason,
        "coverage_tags": project.coverage_tags,
        "upstream": upstream.to_dict(),
        "substrate": substrate.to_dict(),
        "substrate_signals": substrate_signals,
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


def reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def write_endpoint_file(endpoint_file: Path, endpoint: str, daemon_path: Path) -> None:
    stat = daemon_path.stat()
    endpoint_file.parent.mkdir(parents=True, exist_ok=True)
    endpoint_file.write_text(
        "\n".join(
            [
                "# Gradle Rust substrate daemon endpoint",
                f"endpoint={endpoint}",
                f"daemonBinary={daemon_path.resolve()}",
                f"daemonBinaryLastModifiedMillis={int(stat.st_mtime * 1000)}",
                f"daemonBinarySize={stat.st_size}",
                "",
            ]
        ),
        encoding="utf-8",
    )


def prewarm_shared_substrate_daemon(output_dir: Path, daemon_binary: str | None) -> tuple[Path | None, subprocess.Popen[str] | None, dict[str, Any]]:
    if not daemon_binary:
        return None, None, {"enabled": False, "reason": "daemon-binary-not-configured"}
    daemon_path = Path(daemon_binary).expanduser()
    if not daemon_path.is_absolute():
        daemon_path = (REPO_ROOT / daemon_path).resolve()
    if not daemon_path.exists():
        return None, None, {"enabled": False, "reason": f"daemon-binary-missing:{daemon_path}"}

    state_dir = (output_dir / "shared-substrate-state").resolve()
    state_root = state_dir / "state"
    cache_dir = state_root / "cache"
    history_dir = state_root / "history"
    config_cache_dir = state_root / "config-cache"
    toolchain_dir = state_root / "toolchains"
    artifact_store_dir = state_root / "artifacts"
    for directory in [cache_dir, history_dir, config_cache_dir, toolchain_dir, artifact_store_dir]:
        directory.mkdir(parents=True, exist_ok=True)

    port = reserve_loopback_port()
    endpoint = f"tcp://127.0.0.1:{port}"
    socket_path = state_dir / "substrate.sock"
    proc = subprocess.Popen(
        [
            str(daemon_path),
            "--socket-path",
            str(socket_path),
            "--tcp-address",
            f"127.0.0.1:{port}",
            "--log-level",
            "warn",
            "--cache-dir",
            str(cache_dir),
            "--history-dir",
            str(history_dir),
            "--config-cache-dir",
            str(config_cache_dir),
            "--toolchain-dir",
            str(toolchain_dir),
            "--artifact-store-dir",
            str(artifact_store_dir),
        ],
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            return None, None, {
                "enabled": False,
                "reason": f"daemon-exited:{proc.returncode}",
                "state_dir": str(state_dir),
            }
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                write_endpoint_file(state_dir / "substrate.tcp-endpoint", endpoint, daemon_path)
                return state_dir, proc, {
                    "enabled": True,
                    "endpoint": endpoint,
                    "state_dir": str(state_dir),
                }
        except OSError:
            time.sleep(0.05)
    proc.kill()
    proc.wait(timeout=5)
    return None, None, {"enabled": False, "reason": "daemon-not-ready", "state_dir": str(state_dir)}


def stop_shared_substrate_daemon(proc: subprocess.Popen[str] | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def prepare_project_run_dir(source_dir: Path, destination_dir: Path) -> Path:
    if destination_dir.exists():
        shutil.rmtree(destination_dir)
    shutil.copytree(
        source_dir,
        destination_dir,
        ignore=shutil.ignore_patterns(
            ".git",
            ".gradle",
            "build",
            ".kotlin",
            ".idea",
        ),
    )
    return destination_dir.resolve()


def ignored_hash_differences(
    upstream_hashes: dict[str, str],
    substrate_hashes: dict[str, str],
    patterns: Any,
) -> set[str]:
    if not isinstance(patterns, list) or not patterns:
        return set()
    differing = {
        path
        for path in set(upstream_hashes) | set(substrate_hashes)
        if upstream_hashes.get(path) != substrate_hashes.get(path)
    }
    ignored: set[str] = set()
    for path in differing:
        if any(re.fullmatch(str(pattern), path) for pattern in patterns):
            ignored.add(path)
    return ignored


def summarize_execution(results: list[dict[str, Any]]) -> dict[str, Any]:
    supported = [
        result
        for result in results
        if result["expectation"] in {"supported", "supported-or-fail-closed"}
        and result.get("checks", {}).get("resolved_expectation", "supported") == "supported"
    ]
    fail_closed = [
        result
        for result in results
        if result["expectation"] == "fail-closed"
        or result.get("checks", {}).get("resolved_expectation") == "fail-closed"
    ]
    supported_coverage_counts: dict[str, int] = {}
    zero_forward_coverage_counts: dict[str, int] = {}
    for result in supported:
        for tag in result.get("coverage_tags", []):
            supported_coverage_counts[tag] = supported_coverage_counts.get(tag, 0) + 1
            if result.get("match") and result.get("checks", {}).get("jvm_forward_count") == 0:
                zero_forward_coverage_counts[tag] = zero_forward_coverage_counts.get(tag, 0) + 1
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
        "supported_coverage_counts": dict(sorted(supported_coverage_counts.items())),
        "zero_jvm_forward_coverage_counts": dict(sorted(zero_forward_coverage_counts.items())),
        "upstream_duration_ms": sum(result["upstream"].get("duration_ms", 0) for result in results),
        "substrate_duration_ms": sum(result["substrate"].get("duration_ms", 0) for result in results),
        "upstream_task_total": sum(result["upstream"].get("task_count", 0) for result in results),
        "substrate_task_total": sum(result["substrate"].get("task_count", 0) for result in results),
        "runbuild_marker_count": sum(
            1 for result in results if result.get("substrate_signals", {}).get("runbuild_marker")
        ),
        "rust_runbuild_executed_count": sum(
            1
            for result in results
            if result.get("substrate_signals", {}).get("rust_executed_tasks", 0) > 0
        ),
        "taskgraph_capture_count": sum(
            1 for result in results if result.get("substrate_signals", {}).get("taskgraph_captured")
        ),
        "daemon_started_count": sum(
            1 for result in results if result.get("substrate_signals", {}).get("daemon_started")
        ),
        "daemon_reused_count": sum(
            1 for result in results if result.get("substrate_signals", {}).get("daemon_reused")
        ),
        "rust_bootstrap_duration_ms": sum(
            result.get("substrate_signals", {}).get("rust_bootstrap_duration_ms", 0)
            for result in results
        ),
        "non_rust_overhead_ms": sum(
            result.get("substrate_signals", {}).get("non_rust_overhead_ms", 0)
            for result in results
        ),
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
    ]
    if summary.get("zero_jvm_forward_coverage_counts"):
        coverage = ", ".join(
            f"{tag}={count}/{summary.get('supported_coverage_counts', {}).get(tag, count)}"
            for tag, count in summary["zero_jvm_forward_coverage_counts"].items()
        )
        lines.append(f"Zero-forward supported coverage: {coverage}")
    lines.extend([
        f"Observed wall time: upstream={summary['upstream_duration_ms']}ms, substrate={summary['substrate_duration_ms']}ms",
        f"Task totals: upstream={summary['upstream_task_total']}, substrate={summary['substrate_task_total']}",
        f"Rust RunBuild markers: {summary['runbuild_marker_count']}/{summary['project_count']}",
        f"Rust RunBuild executions: {summary['rust_runbuild_executed_count']}/{summary['project_count']}",
        f"Task-graph captures: {summary['taskgraph_capture_count']}/{summary['project_count']}",
        f"Daemon signals: started={summary['daemon_started_count']}, reused={summary['daemon_reused_count']}",
        f"Rust bootstrap duration total: {summary['rust_bootstrap_duration_ms']}ms",
        f"Non-Rust/Gradle overhead estimate: {summary['non_rust_overhead_ms']}ms",
    ])
    shared_daemon = summary.get("shared_daemon", {})
    if shared_daemon:
        lines.append(
            "Shared daemon: "
            + (
                f"enabled at {shared_daemon.get('endpoint')} with state {shared_daemon.get('state_dir')}"
                if shared_daemon.get("enabled")
                else f"disabled ({shared_daemon.get('reason', 'unknown')})"
            )
        )
    lines.extend([
        "",
        "| Project | Expectation | Mode | Result | Plan Source | Upstream ms | Substrate ms | Rust ms | Non-Rust ms | JVM forwards |",
        "| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ])
    for result in results:
        checks = result.get("checks", {})
        signals = result.get("substrate_signals", {})
        jvm_forwards = checks.get("jvm_forward_count", "")
        lines.append(
            "| {name} | {expectation} | {mode} | {status} | {plan_source} | {upstream_ms} | {substrate_ms} | {rust_ms} | {non_rust_ms} | {jvm_forwards} |".format(
                name=result["name"],
                expectation=result["expectation"],
                mode=result["mode"],
                status="PASS" if result["match"] else "FAIL",
                plan_source=signals.get("plan_source", ""),
                upstream_ms=result["upstream"].get("duration_ms", 0),
                substrate_ms=result["substrate"].get("duration_ms", 0),
                rust_ms=signals.get("rust_bootstrap_duration_ms", 0),
                non_rust_ms=signals.get("non_rust_overhead_ms", 0),
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
    source_cache_dir = output_dir / "sources"
    fetch_payload = fetch_manifest(manifest_path, source_cache_dir)
    if not fetch_payload.get("success", False):
        return {
            "valid": True,
            "errors": ["fetch failed"],
            "fetch": fetch_payload,
            "results": [],
            "summary": {},
        }
    projects = [materialized_project(project, source_cache_dir) for project in projects]
    shared_state_dir, shared_daemon, shared_daemon_info = prewarm_shared_substrate_daemon(output_dir, daemon_binary)
    try:
        results = [
            run_project(
                project,
                output_dir,
                gradle_command,
                daemon_binary,
                shared_substrate_state_dir=shared_state_dir,
                verbose=verbose,
            )
            for project in projects
        ]
    finally:
        stop_shared_substrate_daemon(shared_daemon)
    summary = summarize_execution(results)
    summary["shared_daemon"] = shared_daemon_info
    report_path = write_markdown_report(output_dir, summary, results)
    payload = {
        "valid": True,
        "errors": [],
        "manifest": str(manifest_path.resolve()),
        "output_dir": str(output_dir.resolve()),
        "report": str(report_path.resolve()),
        "summary": summary,
        "fetch": fetch_payload,
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
    parser.add_argument("--fetch-only", action="store_true", help="Fetch pinned external sources without executing Gradle")
    parser.add_argument("--output-dir", default="build/dogfood", help="Directory for execution results")
    parser.add_argument("--source-cache-dir", default=None, help="Directory for fetched external sources")
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
    if args.fetch_only:
        source_cache_dir = Path(args.source_cache_dir) if args.source_cache_dir else Path(args.output_dir) / "sources"
        payload = fetch_manifest(Path(args.manifest), source_cache_dir)
        if args.json:
            print(json.dumps(payload, indent=2, sort_keys=True))
        else:
            for fetch in payload.get("fetches", []):
                status = "PASS" if fetch["success"] else "FAIL"
                print(f"{status} {fetch['name']}: {fetch['message']}")
        return 0 if payload.get("success", False) else 1

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
