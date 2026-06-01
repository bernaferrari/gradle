#!/usr/bin/env python3
"""Rust-first warm execution runner for supported Gradle builds.

The runner makes the Phase 3 workflow explicit:

1. Try `gradle-substrate-runbuild` against the cached build-plan shadow first.
2. If the artifact is missing, stale, unsafe, or task-incomplete, run one strict
   Gradle/JVM capture.
3. Run `gradle-substrate-runbuild` again and require zero JVM task forwards.

Unsupported execution failures do not silently fall back to task-by-task JVM
execution. They are reported as structured reasons.
"""

from __future__ import annotations

import argparse
import json
import re
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DAEMON = ROOT / "target" / "debug" / "gradle-substrate-daemon"
DEFAULT_RUNBUILD = ROOT / "target" / "debug" / "gradle-substrate-runbuild"
DEFAULT_GRADLE_UNDER_TEST = ROOT / "build" / "gradle-under-test" / "bin" / "gradle"

DIRECT_SUMMARY_RE = re.compile(
    r"direct-runbuild status=(?P<status>\S+) tasks=(?P<tasks>\d+) "
    r"succeeded=(?P<succeeded>\d+) failed=(?P<failed>\d+) skipped=(?P<skipped>\d+) "
    r"up_to_date=(?P<up_to_date>\d+) from_cache=(?P<from_cache>\d+) "
    r"jvm_forwarded=(?P<jvm_forwarded>\d+) duration_ms=(?P<duration_ms>\d+) "
    r"plan_source=(?P<plan_source>\S+)"
)

CAPTURE_REASON_PREFIXES = {
    "cache-miss": "no cached build-plan artifact",
    "stale": "cached build-plan artifact is stale",
    "unsafe-cache": "missing input_fingerprints",
    "task-mismatch": "does not contain requested task",
    "incomplete-cache": "cached build-plan artifact is incomplete",
}


def resolve_existing(path: str | Path | None, default: Path) -> Path:
    if path:
        candidate = Path(path)
    else:
        candidate = default
    if candidate.is_absolute() or candidate.parent == Path("."):
        return candidate
    return ROOT / candidate


def default_gradle_command() -> Path:
    return DEFAULT_GRADLE_UNDER_TEST if DEFAULT_GRADLE_UNDER_TEST.exists() else Path("gradle")


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


def start_daemon(daemon_binary: Path, state_dir: Path) -> tuple[subprocess.Popen[str], str]:
    state_root = state_dir / "state"
    port = reserve_loopback_port()
    endpoint = f"tcp://127.0.0.1:{port}"
    state_root.mkdir(parents=True, exist_ok=True)
    proc = subprocess.Popen(
        [
            str(daemon_binary),
            "--socket-path",
            str(state_dir / "warm-runner.sock"),
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
        cwd=ROOT,
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    if not wait_for_tcp(port):
        stop_daemon(proc)
        raise RuntimeError("Rust daemon did not become ready")
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


def parse_direct_runbuild_output(output: str) -> dict[str, Any]:
    summary = DIRECT_SUMMARY_RE.search(output)
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


def classify_direct_rejection(exit_code: int, output: str) -> tuple[str, str]:
    if exit_code == 0:
        return "", ""
    normalized = output.strip()
    if "multiple cached build-plan artifacts" in normalized:
        return "ambiguous-cache", normalized
    if "direct RunBuild only supports fully-qualified task paths" in normalized:
        return "invalid-task", normalized
    for kind, needle in CAPTURE_REASON_PREFIXES.items():
        if needle in normalized:
            return kind, normalized
    if "JVM forwarding disabled" in normalized or "UNSUPPORTED" in normalized:
        return "unsupported-execution", normalized
    return "direct-runbuild-failed", normalized


def should_capture_after_direct_rejection(reason_kind: str) -> bool:
    return reason_kind in CAPTURE_REASON_PREFIXES


def direct_runbuild_command(
    runbuild_binary: Path,
    endpoint: str,
    state_dir: Path,
    project_dir: Path,
    tasks: list[str],
    max_parallelism: int,
    changed_paths: list[Path],
    changed_paths_file: Path | None,
) -> list[str]:
    cmd = [
        str(runbuild_binary),
        "--endpoint",
        endpoint,
        "--state-dir",
        str(state_dir),
        "--project-dir",
        str(project_dir),
        "--max-parallelism",
        str(max_parallelism),
    ]
    for task in tasks:
        cmd.extend(["--task", task])
    for path in changed_paths:
        cmd.extend(["--changed-path", str(path)])
    if changed_paths_file is not None:
        cmd.extend(["--changed-paths-file", str(changed_paths_file)])
    return cmd


def gradle_capture_command(
    gradle_command: Path,
    project_dir: Path,
    tasks: list[str],
    daemon_binary: Path,
    state_dir: Path,
    gradle_user_home: Path | None,
) -> list[str]:
    cmd = [
        str(gradle_command),
        "-p",
        str(project_dir),
        *tasks,
        "--no-daemon",
        "--console=plain",
        "--info",
        "-Dorg.gradle.rust.substrate.enabled=true",
        "-Dorg.gradle.rust.substrate.taskgraph.enabled=true",
        "-Dorg.gradle.rust.substrate.runbuild.enabled=true",
        "-Dorg.gradle.rust.substrate.execution.kernel=true",
        f"-Dorg.gradle.rust.substrate.daemon.path={daemon_binary}",
        f"-Dorg.gradle.rust.substrate.state.dir={state_dir}",
    ]
    if gradle_user_home is not None:
        cmd.append(f"--gradle-user-home={gradle_user_home}")
    return cmd


def run_direct(
    runbuild_binary: Path,
    daemon_binary: Path,
    state_dir: Path,
    project_dir: Path,
    tasks: list[str],
    max_parallelism: int,
    changed_paths: list[Path],
    changed_paths_file: Path | None,
    timeout: int,
) -> dict[str, Any]:
    daemon: subprocess.Popen[str] | None = None
    started = time.monotonic()
    try:
        daemon, endpoint = start_daemon(daemon_binary, state_dir)
        cmd = direct_runbuild_command(
            runbuild_binary,
            endpoint,
            state_dir,
            project_dir,
            tasks,
            max_parallelism,
            changed_paths,
            changed_paths_file,
        )
        completed = subprocess.run(
            cmd,
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=timeout,
            check=False,
        )
        output = completed.stdout + completed.stderr
        reason_kind, reason = classify_direct_rejection(completed.returncode, output)
        return {
            "exit_code": completed.returncode,
            "elapsed_ms": round((time.monotonic() - started) * 1000, 1),
            "signals": parse_direct_runbuild_output(output),
            "reason_kind": reason_kind,
            "reason": reason,
            "output_tail": "\n".join(output.strip().splitlines()[-20:]),
        }
    finally:
        stop_daemon(daemon)


def run_capture(
    gradle_command: Path,
    project_dir: Path,
    tasks: list[str],
    daemon_binary: Path,
    state_dir: Path,
    gradle_user_home: Path | None,
    timeout: int,
) -> dict[str, Any]:
    started = time.monotonic()
    completed = subprocess.run(
        gradle_capture_command(
            gradle_command,
            project_dir,
            tasks,
            daemon_binary,
            state_dir,
            gradle_user_home,
        ),
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )
    output = completed.stdout + completed.stderr
    return {
        "exit_code": completed.returncode,
        "elapsed_ms": round((time.monotonic() - started) * 1000, 1),
        "output_tail": "\n".join(output.strip().splitlines()[-20:]),
    }


def direct_success(result: dict[str, Any]) -> bool:
    signals = result.get("signals", {})
    return (
        result.get("exit_code") == 0
        and signals.get("marker") is True
        and signals.get("status") == "COMPLETED"
        and signals.get("jvm_forwarded") == 0
        and signals.get("plan_source") == "build-plan-shadow"
    )


def write_result(path: Path | None, result: dict[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-dir", required=True, type=Path)
    parser.add_argument("--task", action="append", default=[])
    parser.add_argument("--state-dir", type=Path)
    parser.add_argument("--gradle-command", type=Path, default=default_gradle_command())
    parser.add_argument("--daemon-binary", type=Path, default=DEFAULT_DAEMON)
    parser.add_argument("--runbuild-binary", type=Path, default=DEFAULT_RUNBUILD)
    parser.add_argument("--gradle-user-home", type=Path)
    parser.add_argument("--max-parallelism", type=int, default=4)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--output-json", type=Path)
    parser.add_argument("--changed-path", action="append", default=[], type=Path)
    parser.add_argument("--changed-paths-file", type=Path)
    parser.add_argument("--no-capture-on-miss", action="store_true")
    args = parser.parse_args(argv)

    project_dir = args.project_dir.resolve()
    state_dir = (args.state_dir or (project_dir / ".gradle" / "rust-substrate")).resolve()
    state_dir.mkdir(parents=True, exist_ok=True)
    gradle_command = resolve_existing(args.gradle_command, default_gradle_command())
    daemon_binary = resolve_existing(args.daemon_binary, DEFAULT_DAEMON)
    runbuild_binary = resolve_existing(args.runbuild_binary, DEFAULT_RUNBUILD)
    gradle_user_home = args.gradle_user_home.resolve() if args.gradle_user_home else None
    changed_paths = [path.resolve() if path.is_absolute() else path for path in args.changed_path]
    changed_paths_file = args.changed_paths_file.resolve() if args.changed_paths_file else None

    result: dict[str, Any] = {
        "schema": "gradle-substrate.warm-runner-result.v1",
        "project_dir": str(project_dir),
        "state_dir": str(state_dir),
        "tasks": args.task,
        "status": "FAILED",
        "direct_hit": False,
        "capture_ran": False,
    }

    first_direct = run_direct(
        runbuild_binary,
        daemon_binary,
        state_dir,
        project_dir,
        args.task,
        args.max_parallelism,
        changed_paths,
        changed_paths_file,
        args.timeout,
    )
    result["first_direct"] = first_direct
    if direct_success(first_direct):
        result.update({"status": "WARM_HIT", "direct_hit": True})
        write_result(args.output_json, result)
        print("rust-warm-build status=WARM_HIT direct_hit=true capture_ran=false reason=")
        return 0

    reason_kind = str(first_direct.get("reason_kind") or "direct-runbuild-failed")
    if args.no_capture_on_miss or not should_capture_after_direct_rejection(reason_kind):
        result.update({"status": "FAILED", "reason_kind": reason_kind})
        write_result(args.output_json, result)
        print(
            f"rust-warm-build status=FAILED direct_hit=false capture_ran=false reason={reason_kind}",
            file=sys.stderr,
        )
        return 1

    capture = run_capture(
        gradle_command,
        project_dir,
        args.task,
        daemon_binary,
        state_dir,
        gradle_user_home,
        args.timeout,
    )
    result["capture_ran"] = True
    result["capture_reason_kind"] = reason_kind
    result["capture"] = capture
    if capture["exit_code"] != 0:
        result.update({"status": "CAPTURE_FAILED", "reason_kind": "capture-failed"})
        write_result(args.output_json, result)
        print(
            "rust-warm-build status=CAPTURE_FAILED direct_hit=false capture_ran=true reason=capture-failed",
            file=sys.stderr,
        )
        return 1

    second_direct = run_direct(
        runbuild_binary,
        daemon_binary,
        state_dir,
        project_dir,
        args.task,
        args.max_parallelism,
        changed_paths,
        changed_paths_file,
        args.timeout,
    )
    result["second_direct"] = second_direct
    if direct_success(second_direct):
        result.update({"status": "CAPTURED_THEN_DIRECT", "direct_hit": False})
        write_result(args.output_json, result)
        print(
            f"rust-warm-build status=CAPTURED_THEN_DIRECT direct_hit=false capture_ran=true reason={reason_kind}"
        )
        return 0

    second_reason = str(second_direct.get("reason_kind") or "direct-runbuild-failed")
    result.update({"status": "FAILED_AFTER_CAPTURE", "reason_kind": second_reason})
    write_result(args.output_json, result)
    print(
        f"rust-warm-build status=FAILED_AFTER_CAPTURE direct_hit=false capture_ran=true reason={second_reason}",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
