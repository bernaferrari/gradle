#!/usr/bin/env python3
"""Evaluate direct warm Rust RunBuild coverage for dogfood projects.

This runner is deliberately narrower than the main dogfood runner. For each
supported dogfood entry it performs one strict Gradle/JVM capture into an
isolated substrate state directory, then runs `gradle-substrate-runbuild`
directly against the cached build-plan shadow artifact. Success means the warm
path skipped Gradle configuration, validated cached-plan inputs, executed from
Rust, and reported zero JVM forwards.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DOGFOOD_RUNNER_PATH = REPO_ROOT / "tools" / "dogfood_runner" / "run.py"
DEFAULT_MANIFEST = REPO_ROOT / "testing" / "dogfood" / "manifest.json"


def load_dogfood_runner():
    spec = importlib.util.spec_from_file_location("dogfood_runner_run", DOGFOOD_RUNNER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load dogfood runner from {DOGFOOD_RUNNER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def reserve_loopback_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_for_tcp(port: int, timeout_seconds: float = 10.0) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def parse_direct_runbuild_output(output: str) -> dict[str, Any]:
    summary = re.search(
        r"direct-runbuild status=(?P<status>\S+) tasks=(?P<tasks>\d+) "
        r"succeeded=(?P<succeeded>\d+) failed=(?P<failed>\d+) skipped=(?P<skipped>\d+) "
        r"up_to_date=(?P<up_to_date>\d+) from_cache=(?P<from_cache>\d+) "
        r"jvm_forwarded=(?P<jvm_forwarded>\d+) duration_ms=(?P<duration_ms>\d+) "
        r"plan_source=(?P<plan_source>\S+)",
        output,
    )
    if summary is None:
        return {
            "marker": False,
            "status": "",
            "tasks": 0,
            "jvm_forwarded": None,
            "duration_ms": None,
            "plan_source": "",
        }
    data: dict[str, Any] = {"marker": True}
    for key, value in summary.groupdict().items():
        data[key] = int(value) if value.isdigit() else value
    return data


def count_input_fingerprints(state_dir: Path) -> int:
    root = state_dir / "state" / "config-cache" / "build-plan-shadow"
    total = 0
    for artifact in root.glob("*.json"):
        try:
            total += len(json.loads(artifact.read_text(encoding="utf-8")).get("input_fingerprints", []))
        except (OSError, json.JSONDecodeError):
            continue
    return total


def start_daemon(daemon_binary: Path, state_dir: Path) -> tuple[subprocess.Popen[str], str]:
    state_root = state_dir / "state"
    port = reserve_loopback_port()
    endpoint = f"tcp://127.0.0.1:{port}"
    proc = subprocess.Popen(
        [
            str(daemon_binary),
            "--socket-path",
            str(state_dir / "direct-warm.sock"),
            "--tcp-address",
            f"127.0.0.1:{port}",
            "--log-level",
            "warn",
            "--cache-dir",
            str(state_root / "cache"),
            "--history-dir",
            str(state_root / "history"),
            "--config-cache-dir",
            str(state_root / "config-cache"),
            "--toolchain-dir",
            str(state_root / "toolchains"),
            "--artifact-store-dir",
            str(state_root / "artifacts"),
        ],
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    if not wait_for_tcp(port):
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        raise RuntimeError("direct warm daemon did not become ready")
    return proc, endpoint


def stop_daemon(proc: subprocess.Popen[str] | None) -> None:
    if proc is None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def run_project(
    project: Any,
    output_dir: Path,
    gradle_command: Path,
    daemon_binary: Path,
    runbuild_binary: Path,
) -> dict[str, Any]:
    project_dir = project.path
    project_output = output_dir / project.name
    state_dir = project_output / "substrate-state"
    project_output.mkdir(parents=True, exist_ok=True)
    capture_started = time.monotonic()
    capture = subprocess.run(
        [
            str(gradle_command),
            "-p",
            str(project_dir),
            *project.tasks,
            "--no-daemon",
            "--console=plain",
            "--info",
            "-Dorg.gradle.rust.substrate.enabled=true",
            "-Dorg.gradle.rust.substrate.taskgraph.enabled=true",
            "-Dorg.gradle.rust.substrate.runbuild.enabled=true",
            "-Dorg.gradle.rust.substrate.execution.kernel=true",
            f"-Dorg.gradle.rust.substrate.daemon.path={daemon_binary}",
            f"-Dorg.gradle.rust.substrate.state.dir={state_dir}",
        ],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        timeout=project.timeout_seconds,
        check=False,
    )
    capture_ms = round((time.monotonic() - capture_started) * 1000, 1)
    capture_output = capture.stdout + capture.stderr
    (project_output / "capture.out").write_text(capture_output, encoding="utf-8")
    fingerprint_count = count_input_fingerprints(state_dir)

    daemon: subprocess.Popen[str] | None = None
    direct = None
    direct_ms = 0.0
    direct_output = ""
    direct_signals = parse_direct_runbuild_output("")
    rejection_reason = ""
    if capture.returncode == 0:
        try:
            daemon, endpoint = start_daemon(daemon_binary, state_dir)
            direct_started = time.monotonic()
            direct = subprocess.run(
                [
                    str(runbuild_binary),
                    "--endpoint",
                    endpoint,
                    "--state-dir",
                    str(state_dir),
                    "--project-dir",
                    str(project_dir),
                    "--max-parallelism",
                    "4",
                ],
                cwd=REPO_ROOT,
                text=True,
                capture_output=True,
                timeout=project.timeout_seconds,
                check=False,
            )
            direct_ms = round((time.monotonic() - direct_started) * 1000, 1)
            direct_output = direct.stdout + direct.stderr
            direct_signals = parse_direct_runbuild_output(direct_output)
        except Exception as error:  # noqa: BLE001 - report precise runner/setup failure
            rejection_reason = str(error)
        finally:
            stop_daemon(daemon)
    else:
        rejection_reason = "capture-failed"
    (project_output / "direct-runbuild.out").write_text(direct_output, encoding="utf-8")

    ok = (
        project.expectation == "supported"
        and capture.returncode == 0
        and direct is not None
        and direct.returncode == 0
        and direct_signals.get("marker") is True
        and direct_signals.get("status") == "COMPLETED"
        and direct_signals.get("jvm_forwarded") == 0
        and direct_signals.get("plan_source") == "build-plan-shadow"
    )
    if not ok and not rejection_reason:
        rejection_reason = direct_output.strip().splitlines()[-1] if direct_output.strip() else "direct-runbuild-failed"

    return {
        "name": project.name,
        "path": str(project_dir),
        "tasks": project.tasks,
        "expectation": project.expectation,
        "capture_exit_code": capture.returncode,
        "capture_ms": capture_ms,
        "direct_exit_code": direct.returncode if direct is not None else None,
        "direct_ms": direct_ms,
        "input_fingerprint_count": fingerprint_count,
        "direct": direct_signals,
        "match": ok,
        "rejection_reason": "" if ok else rejection_reason,
    }


def summarize(results: list[dict[str, Any]]) -> dict[str, Any]:
    supported = [result for result in results if result["expectation"] == "supported"]
    direct_ok = [result for result in supported if result["match"]]
    return {
        "schema": "gradle-substrate.direct-warm-dogfood-summary.v1",
        "project_count": len(results),
        "supported_project_count": len(supported),
        "direct_warm_supported_count": len(direct_ok),
        "zero_jvm_forward_count": sum(
            1 for result in direct_ok if result.get("direct", {}).get("jvm_forwarded") == 0
        ),
        "failed_projects": [result["name"] for result in supported if not result["match"]],
        "total_direct_ms": round(sum(float(result.get("direct_ms") or 0) for result in direct_ok), 1),
    }


def write_markdown(output_dir: Path, summary: dict[str, Any], results: list[dict[str, Any]]) -> Path:
    path = output_dir / "direct-warm-summary.md"
    lines = [
        "# Direct Warm RunBuild Dogfood",
        "",
        f"Supported direct warm projects: {summary['direct_warm_supported_count']}/{summary['supported_project_count']}",
        f"Zero-JVM-forward direct warm projects: {summary['zero_jvm_forward_count']}/{summary['supported_project_count']}",
        f"Total direct warm wall time: {summary['total_direct_ms']} ms",
        "",
        "| Project | Result | Capture ms | Direct ms | RunBuild ms | Tasks | Fingerprints | Plan | JVM forwards | Reason |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- | ---: | --- |",
    ]
    for result in results:
        direct = result.get("direct", {})
        lines.append(
            "| {name} | {status} | {capture_ms} | {direct_ms} | {runbuild_ms} | {tasks} | {fingerprints} | {plan} | {jvm} | {reason} |".format(
                name=result["name"],
                status="PASS" if result["match"] else ("SKIP" if result["expectation"] != "supported" else "FAIL"),
                capture_ms=result["capture_ms"],
                direct_ms=result["direct_ms"],
                runbuild_ms=direct.get("duration_ms", ""),
                tasks=direct.get("tasks", 0),
                fingerprints=result["input_fingerprint_count"],
                plan=direct.get("plan_source", ""),
                jvm=direct.get("jvm_forwarded", ""),
                reason=result["rejection_reason"],
            )
        )
    lines.extend([
        "",
        "This is warm-path coverage evidence only. It assumes one prior Gradle/JVM capture and then evaluates whether the cached plan can execute directly through Rust.",
    ])
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST))
    parser.add_argument("--output-dir", default="build/direct-warm-dogfood")
    dogfood = load_dogfood_runner()
    parser.add_argument(
        "--gradle-command",
        default=dogfood.default_gradle_command(),
        help="Gradle-under-test executable; defaults to the dogfood runner's Gradle-under-test discovery",
    )
    parser.add_argument("--daemon-binary", default="target/debug/gradle-substrate-daemon")
    parser.add_argument("--runbuild-binary", default="target/debug/gradle-substrate-runbuild")
    parser.add_argument("--project", action="append", help="Run only named project(s)")
    args = parser.parse_args(argv)

    errors = dogfood.validate_manifest(Path(args.manifest))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 2
    _data, projects = dogfood.load_manifest(Path(args.manifest))
    if args.project:
        requested = set(args.project)
        projects = [project for project in projects if project.name in requested]
    projects = [project for project in projects if project.expectation == "supported"]

    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if not args.gradle_command:
        print(
            "missing Gradle-under-test command; set GRADLE_UNDER_TEST_BIN, "
            "GRADLE_UNDER_TEST, build build/gradle-under-test, or pass --gradle-command",
            file=sys.stderr,
        )
        return 2
    gradle_command = Path(args.gradle_command).resolve()
    daemon_binary = (REPO_ROOT / args.daemon_binary).resolve() if not Path(args.daemon_binary).is_absolute() else Path(args.daemon_binary)
    runbuild_binary = (REPO_ROOT / args.runbuild_binary).resolve() if not Path(args.runbuild_binary).is_absolute() else Path(args.runbuild_binary)

    results = [
        run_project(project, output_dir, gradle_command, daemon_binary, runbuild_binary)
        for project in projects
    ]
    summary = summarize(results)
    (output_dir / "direct-warm-results.json").write_text(
        json.dumps({"summary": summary, "results": results}, indent=2) + "\n",
        encoding="utf-8",
    )
    report = write_markdown(output_dir, summary, results)
    print(f"Direct warm supported projects: {summary['direct_warm_supported_count']}/{summary['supported_project_count']}")
    print(f"Report: {report}")
    return 0 if not summary["failed_projects"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
