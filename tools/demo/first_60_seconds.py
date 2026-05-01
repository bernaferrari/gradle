#!/usr/bin/env python3
"""Measure Rust substrate wins that are visible in the first minute.

This is intentionally small and local:
  - daemon readiness is measured by Unix socket availability
  - dependency transport uses the checked-in local HTTP store/cache/checksum smoke test
    and the Gradle resource seam test for Rust-backed uncached downloads
  - dynamic metadata transport uses a local HTTP maven-metadata.xml smoke test
  - dependency read-through uses focused Gradle seam tests proving remote fetch is skipped
  - file watching uses a native notify first-event latency test

Build time is kept outside the headline timings so the numbers describe runtime
responsiveness, not Rust compilation.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DAEMON = ROOT / "target" / "debug" / "gradle-substrate-daemon"


def run(cmd: list[str], *, timeout: int = 60, capture: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        capture_output=capture,
        timeout=timeout,
        check=False,
    )


def daemon_binary_is_current() -> bool:
    if not DAEMON.exists():
        return False
    binary_mtime = DAEMON.stat().st_mtime
    inputs = [ROOT / "Cargo.toml", ROOT / "substrate" / "Cargo.toml"]
    inputs.extend((ROOT / "substrate" / "src").rglob("*.rs"))
    return all(path.stat().st_mtime <= binary_mtime for path in inputs if path.exists())


def ensure_daemon_built(skip_build: bool) -> None:
    if daemon_binary_is_current():
        return
    if skip_build:
        raise SystemExit(f"Missing or stale daemon binary: {DAEMON}")
    print("Building Rust daemon once if needed; build time is not included in readiness numbers.")
    completed = run(["cargo", "build", "-q", "-p", "gradle-substrate-daemon"], timeout=180)
    if completed.returncode != 0:
        sys.stdout.write(completed.stdout)
        sys.stderr.write(completed.stderr)
        raise SystemExit(completed.returncode)


def measure_daemon_readiness() -> dict[str, object]:
    temp = Path(tempfile.mkdtemp(prefix="gradle-rust-first60."))
    socket_path = temp / "daemon.sock"
    output = ""
    started = time.perf_counter()
    proc = subprocess.Popen(
        [
            str(DAEMON),
            "--socket-path",
            str(socket_path),
            "--cache-dir",
            str(temp / "cache"),
            "--history-dir",
            str(temp / "history"),
            "--config-cache-dir",
            str(temp / "config-cache"),
            "--toolchain-dir",
            str(temp / "toolchains"),
            "--artifact-store-dir",
            str(temp / "artifacts"),
            "--log-level",
            "warn",
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )

    try:
        deadline = time.perf_counter() + 10
        while time.perf_counter() < deadline:
            if socket_path.exists():
                ready_ms = round((time.perf_counter() - started) * 1000, 1)
                break
            if proc.poll() is not None:
                output = proc.stdout.read() if proc.stdout else ""
                raise RuntimeError(f"daemon exited before socket readiness:\n{output}")
            time.sleep(0.005)
        else:
            raise TimeoutError("daemon socket was not ready within 10s")

        proc.terminate()
        try:
            output = proc.communicate(timeout=5)[0]
        except subprocess.TimeoutExpired:
            proc.kill()
            output = proc.communicate(timeout=5)[0]

        internal_ready = None
        match = re.search(r"Ready in: (\d+)ms", output)
        if match:
            internal_ready = int(match.group(1))

        return {
            "name": "daemon_socket_ready",
            "ok": True,
            "elapsed_ms": ready_ms,
            "daemon_reported_ready_ms": internal_ready,
            "threshold_ms": 1000,
        }
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)
        shutil.rmtree(temp, ignore_errors=True)


def measure_cargo_test(label: str, test_filter: str, timeout: int) -> dict[str, object]:
    started = time.perf_counter()
    completed = run(
        [
            "cargo",
            "test",
            "-p",
            "gradle-substrate-daemon",
            test_filter,
            "--",
            "--nocapture",
        ],
        timeout=timeout,
    )
    elapsed_ms = round((time.perf_counter() - started) * 1000, 1)
    output = completed.stdout + completed.stderr
    metric_ms = None
    if label == "file_watch_first_event":
        match = re.search(r"file_watch_first_change_latency_ms=(\d+)", output)
        if match:
            metric_ms = int(match.group(1))

    return {
        "name": label,
        "ok": completed.returncode == 0,
        "elapsed_ms": elapsed_ms,
        "runtime_metric_ms": metric_ms,
        "threshold_ms": 1500 if label == "file_watch_first_event" else 2000,
        "test_filter": test_filter,
        "tail": "\n".join(output.strip().splitlines()[-12:]),
    }


def measure_gradle_readthrough_smoke(timeout: int = 90) -> list[dict[str, object]]:
    filters = [
        "org.gradle.api.internal.artifacts.ivyservice.ivyresolve.RepositoryChainArtifactResolverTest.uses read-through artifact cache between local and remote access",
        "org.gradle.api.internal.artifacts.ivyservice.ivyresolve.RepositoryChainArtifactResolverTest.uses read-through artifact cache for non-jar external module artifacts",
        "org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest.uses rust cached pom metadata when gradle has no local cached resource",
        "org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest.uses rust cached gradle module metadata when gradle has no local cached resource",
        "org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest.uses rust cached maven metadata when gradle has no local cached resource",
        "org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest.downloads uncached resource through rust transport before java transport",
        "org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest.passes explicit artifact coordinate to rust transport download",
    ]
    cmd = [
        "./gradlew",
        ":dependency-management:test",
        "-x",
        ":distributions-core:generateLicenseFile",
    ]
    for test_filter in filters:
        cmd.extend(["--tests", test_filter])

    started = time.perf_counter()
    completed = run(cmd, timeout=timeout)
    elapsed_ms = round((time.perf_counter() - started) * 1000, 1)
    output = completed.stdout + completed.stderr
    tail = "\n".join(output.strip().splitlines()[-12:])
    common = {
        "ok": completed.returncode == 0,
        "elapsed_ms": elapsed_ms,
        "threshold_ms": 30000,
        "test_filters": filters,
        "shared_gradle_invocation": True,
        "tail": tail,
    }
    return [
        {
            "name": "dependency_artifact_readthrough",
            **common,
        },
        {
            "name": "dependency_metadata_readthrough",
            **common,
        },
    ]


def write_report(path: Path, results: list[dict[str, object]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"results": results}, indent=2) + "\n", encoding="utf-8")


def print_summary(results: list[dict[str, object]]) -> None:
    print("\nFirst-60-second Rust substrate wins")
    for result in results:
        status = "PASS" if result["ok"] else "FAIL"
        if result["name"] == "daemon_socket_ready":
            extra = f", daemon self-reported {result['daemon_reported_ready_ms']}ms"
        elif result.get("runtime_metric_ms") is not None:
            extra = f", first event {result['runtime_metric_ms']}ms"
        else:
            extra = ""
        print(f"{status} {result['name']}: {result['elapsed_ms']}ms{extra}")

    print("\nWhy these are visible:")
    print("- daemon_socket_ready is the time before Gradle can send work to the Rust sidecar")
    print("- dependency_transport_store_checksum is the bounded Rust path for Maven bytes, local store, cache hit, and checksum verification")
    print("- dependency_metadata_transport_cache proves URL-only POM downloads through Rust warm the Rust metadata cache")
    print("- dependency_dynamic_metadata_transport_cache proves maven-metadata.xml downloads warm the dynamic-version metadata cache")
    print("- dependency_artifact_readthrough proves Gradle can skip remote artifact access when Rust already has the JAR")
    print("- dependency_metadata_readthrough proves Gradle can skip remote POM metadata access and can route uncached resource downloads through Rust")
    print("- file_watch_first_event is the delay before source edits become observable")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-build", action="store_true", help="fail if the daemon binary is missing")
    parser.add_argument("--output", type=Path, help="write JSON metrics to this path")
    args = parser.parse_args()

    ensure_daemon_built(args.skip_build)
    results = [
        measure_daemon_readiness(),
        measure_cargo_test(
            "dependency_transport_store_checksum",
            "test_download_artifact_populates_store_and_checksum_cache",
            timeout=60,
        ),
        measure_cargo_test(
            "dependency_metadata_transport_cache",
            "test_download_metadata_url_populates_metadata_cache",
            timeout=60,
        ),
        measure_cargo_test(
            "dependency_dynamic_metadata_transport_cache",
            "test_download_maven_metadata_url_populates_dynamic_metadata_cache",
            timeout=60,
        ),
        *measure_gradle_readthrough_smoke(),
        measure_cargo_test(
            "file_watch_first_event",
            "file_watch_reports_first_change_quickly",
            timeout=60,
        ),
    ]

    print_summary(results)
    if args.output:
        write_report(args.output, results)
        print(f"\nMetrics JSON: {args.output}")

    for result in results:
        if not result["ok"]:
            print(f"\nFailure tail for {result['name']}:\n{result.get('tail', '')}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
