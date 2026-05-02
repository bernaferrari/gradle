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
import functools
import hashlib
import http.server
import json
import os
import re
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
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


def resolve_gradle_under_test() -> Path | None:
    configured = os.environ.get("GRADLE_UNDER_TEST_BIN", "").strip()
    if not configured:
        gradle_under_test = os.environ.get("GRADLE_UNDER_TEST", "").strip()
        if gradle_under_test:
            configured = str(Path(gradle_under_test) / "bin" / "gradle")
    candidate = Path(configured) if configured else ROOT / "build" / "gradle-under-test" / "bin" / "gradle"
    return candidate if candidate.exists() else None


def write_maven_module(repo: Path) -> None:
    module_dir = repo / "org" / "test" / "projectA"
    for version in ("1.0", "1.5"):
        version_dir = module_dir / version
        version_dir.mkdir(parents=True, exist_ok=True)
        (version_dir / f"projectA-{version}.pom").write_text(
            "<project>"
            "<modelVersion>4.0.0</modelVersion>"
            "<groupId>org.test</groupId>"
            "<artifactId>projectA</artifactId>"
            f"<version>{version}</version>"
            "</project>",
            encoding="utf-8",
        )
        with zipfile.ZipFile(version_dir / f"projectA-{version}.jar", "w") as jar:
            jar.writestr(f"projectA-{version}.txt", f"payload-{version}\n")

    (module_dir / "maven-metadata.xml").write_text(
        "<metadata>"
        "<groupId>org.test</groupId>"
        "<artifactId>projectA</artifactId>"
        "<versioning>"
        "<latest>1.5</latest>"
        "<release>1.5</release>"
        "<versions><version>1.0</version><version>1.5</version></versions>"
        "<lastUpdated>20260501000000</lastUpdated>"
        "</versioning>"
        "</metadata>",
        encoding="utf-8",
    )


class CountingHttpServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def measure_real_build_dependency_readthrough(timeout: int = 75) -> dict[str, object]:
    gradle = resolve_gradle_under_test()
    if gradle is None:
        return {
            "name": "real_build_dependency_readthrough",
            "ok": True,
            "skipped": True,
            "elapsed_ms": 0,
            "threshold_ms": 30000,
            "tail": "Skipped: build/gradle-under-test/bin/gradle was not found. Build :distributions-full:install or set GRADLE_UNDER_TEST_BIN.",
        }

    temp = Path(tempfile.mkdtemp(prefix="gradle-real-readthrough."))
    repo = temp / "repo"
    project = temp / "project"
    requests: list[str] = []
    server = None
    started = time.perf_counter()

    try:
        write_maven_module(repo)

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, fmt: str, *args: object) -> None:
                pass

            def do_GET(self) -> None:
                requests.append(self.path)
                super().do_GET()

            def do_HEAD(self) -> None:
                requests.append("HEAD " + self.path)
                super().do_HEAD()

        handler = functools.partial(Handler, directory=str(repo))
        server = CountingHttpServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        repository_url = f"http://127.0.0.1:{server.server_address[1]}"

        project.mkdir(parents=True)
        (project / "settings.gradle.kts").write_text('rootProject.name = "real-readthrough"\n', encoding="utf-8")
        (project / "build.gradle.kts").write_text(
            f"""
repositories {{ maven {{ url = uri("{repository_url}") }} }}
configurations {{ create("compile") }}
dependencies {{ "compile"("org.test:projectA:1.+") }}
tasks.register<Sync>("retrieve") {{
    from(configurations.getByName("compile"))
    into(layout.buildDirectory.dir("libs"))
}}
""",
            encoding="utf-8",
        )

        state_dir = temp / "substrate-state"

        def run_retrieve(gradle_home: Path) -> tuple[subprocess.CompletedProcess[str], list[str]]:
            before = len(requests)
            completed = subprocess.run(
                [
                    str(gradle),
                    "-p",
                    str(project),
                    "retrieve",
                    "--no-daemon",
                    "--console=plain",
                    f"--gradle-user-home={gradle_home}",
                    "-Dorg.gradle.rust.substrate.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.download.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.readthrough.metadata=true",
                    "-Dorg.gradle.rust.substrate.dependency.readthrough.artifacts=true",
                    f"-Dorg.gradle.rust.substrate.daemon.path={DAEMON}",
                    f"-Dorg.gradle.rust.substrate.state.dir={state_dir}",
                ],
                cwd=ROOT,
                text=True,
                capture_output=True,
                timeout=timeout,
                check=False,
            )
            return completed, requests[before:]

        first, first_requests = run_retrieve(temp / "gradle-home-1")
        shutil.rmtree(project / "build", ignore_errors=True)
        second, second_requests = run_retrieve(temp / "gradle-home-2")

        output_file = project / "build" / "libs" / "projectA-1.5.jar"
        output_sha256 = hashlib.sha256(output_file.read_bytes()).hexdigest() if output_file.exists() else None
        elapsed_ms = round((time.perf_counter() - started) * 1000, 1)
        remote_requests_avoided = len(first_requests) - len(second_requests)
        ok = (
            first.returncode == 0
            and second.returncode == 0
            and len(first_requests) > 0
            and len(second_requests) == 0
            and output_file.exists()
        )
        output = first.stdout + first.stderr + "\n--- second run ---\n" + second.stdout + second.stderr
        return {
            "name": "real_build_dependency_readthrough",
            "ok": ok,
            "elapsed_ms": elapsed_ms,
            "threshold_ms": 30000,
            "first_run_remote_requests": len(first_requests),
            "second_run_remote_requests": len(second_requests),
            "remote_requests_avoided": remote_requests_avoided,
            "output_sha256": output_sha256,
            "first_run_requests": first_requests,
            "second_run_requests": second_requests,
            "tail": "\n".join(output.strip().splitlines()[-20:]),
        }
    finally:
        if server is not None:
            server.shutdown()
            server.server_close()
        shutil.rmtree(temp, ignore_errors=True)


def write_report(path: Path, results: list[dict[str, object]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"results": results}, indent=2) + "\n", encoding="utf-8")


def print_summary(results: list[dict[str, object]]) -> None:
    print("\nFirst-60-second Rust substrate wins")
    for result in results:
        status = "SKIP" if result.get("skipped") else ("PASS" if result["ok"] else "FAIL")
        if result["name"] == "daemon_socket_ready":
            extra = f", daemon self-reported {result['daemon_reported_ready_ms']}ms"
        elif result["name"] == "real_build_dependency_readthrough" and not result.get("skipped"):
            extra = (
                f", remote avoided {result['remote_requests_avoided']}/"
                f"{result['first_run_remote_requests']}, output {result['output_sha256']}"
            )
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
    print("- real_build_dependency_readthrough proves a real Gradle build can warm Rust over HTTP, then rerun from a fresh Gradle user home with zero remote requests")
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
        measure_real_build_dependency_readthrough(),
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
