#!/usr/bin/env python3
"""Measure Rust substrate wins that are visible in the first minute.

This is intentionally small and local:
  - daemon readiness is measured by Unix socket availability
  - fast mode reports user-visible warm daemon, Rust DAG, dependency read-through,
    and file-watch responsiveness
  - proof mode runs the heavier checked-in transport/cache/checksum smoke tests
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
        "threshold_ms": 1500 if label == "file_watch_first_event" else 3000,
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
            entry = zipfile.ZipInfo(f"projectA-{version}.txt")
            entry.date_time = (1980, 1, 1, 0, 0, 0)
            entry.compress_type = zipfile.ZIP_STORED
            jar.writestr(entry, f"payload-{version}\n")

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

    static_dir = repo / "org" / "test" / "projectStatic" / "1.0"
    static_dir.mkdir(parents=True, exist_ok=True)
    (static_dir / "projectStatic-1.0.pom").write_text(
        "<project>"
        "<modelVersion>4.0.0</modelVersion>"
        "<groupId>org.test</groupId>"
        "<artifactId>projectStatic</artifactId>"
        "<version>1.0</version>"
        "<dependencies>"
        "<dependency>"
        "<groupId>org.test</groupId>"
        "<artifactId>projectStaticChild</artifactId>"
        "<version>1.0</version>"
        "</dependency>"
        "</dependencies>"
        "</project>",
        encoding="utf-8",
    )
    with zipfile.ZipFile(static_dir / "projectStatic-1.0.jar", "w") as jar:
        entry = zipfile.ZipInfo("projectStatic-1.0.txt")
        entry.date_time = (1980, 1, 1, 0, 0, 0)
        entry.compress_type = zipfile.ZIP_STORED
        jar.writestr(entry, "static-prefetch-payload\n")

    static_child_dir = repo / "org" / "test" / "projectStaticChild" / "1.0"
    static_child_dir.mkdir(parents=True, exist_ok=True)
    (static_child_dir / "projectStaticChild-1.0.pom").write_text(
        "<project>"
        "<modelVersion>4.0.0</modelVersion>"
        "<groupId>org.test</groupId>"
        "<artifactId>projectStaticChild</artifactId>"
        "<version>1.0</version>"
        "</project>",
        encoding="utf-8",
    )
    with zipfile.ZipFile(static_child_dir / "projectStaticChild-1.0.jar", "w") as jar:
        entry = zipfile.ZipInfo("projectStaticChild-1.0.txt")
        entry.date_time = (1980, 1, 1, 0, 0, 0)
        entry.compress_type = zipfile.ZIP_STORED
        jar.writestr(entry, "static-prefetch-child-payload\n")


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
        (project / "settings.gradle").write_text("rootProject.name = 'real-readthrough'\n", encoding="utf-8")
        (project / "build.gradle").write_text(
            """
repositories { maven { url = uri("%s") } }
configurations {
    compile
    staticCompile
}
dependencies {
    compile "org.test:projectA:1.+"
    staticCompile "org.test:projectStatic:1.0"
}
tasks.register("retrieve", Sync) {
    from configurations.compile
    into layout.buildDirectory.dir("libs")
}
tasks.register("resolveStaticGraph") {
    def outputFile = layout.buildDirectory.file("resolution/static-graph.txt")
    outputs.file(outputFile)
    doLast {
        def modules = configurations.staticCompile.incoming.resolutionResult.allComponents
            .collect { it.moduleVersion }
            .findAll { it != null }
            .collect { "${it.group}:${it.name}:${it.version}" }
            .sort()
        outputFile.get().asFile.text = modules.join("\\n")
    }
}
tasks.register("retrieveStatic", Sync) {
    from configurations.staticCompile
    into layout.buildDirectory.dir("static-libs")
}
""" % repository_url,
            encoding="utf-8",
        )

        state_dir = temp / "substrate-state"

        def run_gradle(gradle_home: Path, tasks: list[str]) -> tuple[subprocess.CompletedProcess[str], list[str]]:
            before = len(requests)
            completed = subprocess.run(
                [
                    str(gradle),
                    "-p",
                    str(project),
                    *tasks,
                    "--no-daemon",
                    "--console=plain",
                    f"--gradle-user-home={gradle_home}",
                    "-Dorg.gradle.rust.substrate.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.download.enabled=true",
                    "-Dorg.gradle.rust.substrate.dependency.readthrough.metadata=true",
                    "-Dorg.gradle.rust.substrate.dependency.readthrough.artifacts=true",
                    "-Dorg.gradle.rust.substrate.dependency.prefetch.artifacts=true",
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

        first, first_requests = run_gradle(temp / "gradle-home-1", ["resolveStaticGraph", "retrieve"])
        static_graph_file = project / "build" / "resolution" / "static-graph.txt"
        static_graph_text = static_graph_file.read_text() if static_graph_file.exists() else ""
        static_graph_recorded = (
            "org.test:projectStatic:1.0" in static_graph_text
            and "org.test:projectStaticChild:1.0" in static_graph_text
        )
        shutil.rmtree(project / "build", ignore_errors=True)
        second, second_requests = run_gradle(temp / "gradle-home-2", ["retrieve", "retrieveStatic"])

        output_file = project / "build" / "libs" / "projectA-1.5.jar"
        static_output_file = project / "build" / "static-libs" / "projectStatic-1.0.jar"
        static_child_output_file = project / "build" / "static-libs" / "projectStaticChild-1.0.jar"
        output_sha256 = hashlib.sha256(output_file.read_bytes()).hexdigest() if output_file.exists() else None
        static_output_sha256 = (
            hashlib.sha256(static_output_file.read_bytes()).hexdigest() if static_output_file.exists() else None
        )
        static_child_output_sha256 = (
            hashlib.sha256(static_child_output_file.read_bytes()).hexdigest()
            if static_child_output_file.exists()
            else None
        )
        elapsed_ms = round((time.perf_counter() - started) * 1000, 1)
        remote_requests_avoided = len(first_requests) - len(second_requests)
        ok = (
            first.returncode == 0
            and second.returncode == 0
            and len(first_requests) > 0
            and len(second_requests) == 0
            and output_file.exists()
            and static_output_file.exists()
            and static_child_output_file.exists()
            and static_graph_recorded
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
            "static_prefetch_output_sha256": static_output_sha256,
            "static_prefetch_child_output_sha256": static_child_output_sha256,
            "static_prefetch_graph_recorded": static_graph_recorded,
            "first_run_requests": first_requests,
            "second_run_requests": second_requests,
            "tail": "\n".join(output.strip().splitlines()[-20:]),
        }
    finally:
        if server is not None:
            server.shutdown()
            server.server_close()
        shutil.rmtree(temp, ignore_errors=True)


def measure_authoritative_runbuild_fast(timeout: int = 90) -> dict[str, object]:
    gradle = resolve_gradle_under_test()
    if gradle is None:
        return {
            "name": "authoritative_rust_dag",
            "ok": True,
            "skipped": True,
            "elapsed_ms": 0,
            "threshold_ms": 30000,
            "tail": "Skipped: build/gradle-under-test/bin/gradle was not found. Build :distributions-full:install or set GRADLE_UNDER_TEST_BIN.",
        }

    source_project = ROOT / "testing" / "corpus" / "java-library-kotlin-dsl"
    temp = Path(tempfile.mkdtemp(prefix="gradle-rust-dag-fast."))
    project = temp / "project"
    state_dir = temp / "substrate-state"
    started = time.perf_counter()

    try:
        shutil.copytree(
            source_project,
            project,
            ignore=shutil.ignore_patterns(".gradle", "build"),
        )
        gradle_home = temp / "gradle-home"

        def run_authoritative(tasks: list[str]) -> tuple[subprocess.CompletedProcess[str], float]:
            invocation_started = time.perf_counter()
            completed = subprocess.run(
                [
                    str(gradle),
                    "-p",
                    str(project),
                    *tasks,
                    "--no-daemon",
                    "--console=plain",
                    "--info",
                    "--configuration-cache",
                    f"--gradle-user-home={gradle_home}",
                    "-Dorg.gradle.rust.substrate.enabled=true",
                    "-Dorg.gradle.rust.substrate.mode=shadow",
                    "-Dorg.gradle.rust.substrate.runbuild.authoritative=true",
                    f"-Dorg.gradle.rust.substrate.daemon.path={DAEMON}",
                    f"-Dorg.gradle.rust.substrate.state.dir={state_dir}",
                ],
                cwd=ROOT,
                text=True,
                capture_output=True,
                timeout=timeout,
                check=False,
            )
            return completed, round((time.perf_counter() - invocation_started) * 1000, 1)

        first, first_elapsed_ms = run_authoritative(["build"])
        warm, warm_elapsed_ms = run_authoritative(["build"])
        elapsed_ms = round((time.perf_counter() - started) * 1000, 1)

        def parse_run(output: str) -> dict[str, object]:
            executed_match = re.search(
                r"\[substrate:run-build\] Rust executed (\d+)(?: Gradle)? tasks .* JVM (?:forwarding|fallback) disabled",
                output,
            )
            selected_match = re.search(r"Tasks to be executed:\s*\[(.*?)\]", output, re.DOTALL)
            selected_tasks = re.findall(r"task '([^']+)'", selected_match.group(1)) if selected_match else []
            jvm_forwarded_match = re.search(r"jvmForwarded=(\d+)", output)
            return {
                "marker_present": executed_match is not None,
                "rust_executed_tasks": int(executed_match.group(1)) if executed_match else 0,
                "selected_task_count": len(selected_tasks),
                "selected_tasks": selected_tasks,
                "tasks_forwarded_to_jvm": int(jvm_forwarded_match.group(1)) if jvm_forwarded_match else 0,
                "configuration_cache_reused": "Configuration cache entry reused" in output,
                "configuration_cache_stored": "Configuration cache entry stored" in output,
            }

        first_output = first.stdout + first.stderr
        warm_output = warm.stdout + warm.stderr
        first_parsed = parse_run(first_output)
        warm_parsed = parse_run(warm_output)
        jar = project / "build" / "libs" / "corpus-java-library-1.0.0.jar"
        output_sha256 = hashlib.sha256(jar.read_bytes()).hexdigest() if jar.exists() else None
        ok = (
            first.returncode == 0
            and warm.returncode == 0
            and bool(first_parsed["marker_present"])
            and bool(warm_parsed["marker_present"])
            and int(first_parsed["rust_executed_tasks"]) > 0
            and int(warm_parsed["rust_executed_tasks"]) > 0
            and int(first_parsed["tasks_forwarded_to_jvm"]) == 0
            and int(warm_parsed["tasks_forwarded_to_jvm"]) == 0
            and jar.exists()
        )
        return {
            "name": "authoritative_rust_dag",
            "ok": ok,
            "elapsed_ms": elapsed_ms,
            "threshold_ms": 30000,
            "first_elapsed_ms": first_elapsed_ms,
            "warm_elapsed_ms": warm_elapsed_ms,
            "first_rust_executed_tasks": first_parsed["rust_executed_tasks"],
            "warm_rust_executed_tasks": warm_parsed["rust_executed_tasks"],
            "first_selected_task_count": first_parsed["selected_task_count"],
            "warm_selected_task_count": warm_parsed["selected_task_count"],
            "first_selected_tasks": first_parsed["selected_tasks"],
            "warm_selected_tasks": warm_parsed["selected_tasks"],
            "first_tasks_forwarded_to_jvm": first_parsed["tasks_forwarded_to_jvm"],
            "warm_tasks_forwarded_to_jvm": warm_parsed["tasks_forwarded_to_jvm"],
            "first_configuration_cache_reused": first_parsed["configuration_cache_reused"],
            "first_configuration_cache_stored": first_parsed["configuration_cache_stored"],
            "warm_configuration_cache_reused": warm_parsed["configuration_cache_reused"],
            "warm_configuration_cache_stored": warm_parsed["configuration_cache_stored"],
            "tasks_forwarded_to_jvm": int(first_parsed["tasks_forwarded_to_jvm"])
            + int(warm_parsed["tasks_forwarded_to_jvm"]),
            "output_sha256": output_sha256,
            "tail": "\n".join((first_output + "\n--- warm run ---\n" + warm_output).strip().splitlines()[-20:]),
        }
    finally:
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
        elif result["name"] == "authoritative_rust_dag" and not result.get("skipped"):
            extra = (
                f", cold {result['first_elapsed_ms']}ms/{result['first_rust_executed_tasks']} tasks, "
                f"warm {result['warm_elapsed_ms']}ms/{result['warm_rust_executed_tasks']} tasks, "
                f"cc reused {result['warm_configuration_cache_reused']}, "
                f"JVM forwards {result['tasks_forwarded_to_jvm']}, "
                f"output {result['output_sha256']}"
            )
        elif result["name"] == "real_build_dependency_readthrough" and not result.get("skipped"):
            extra = (
                f", remote avoided {result['remote_requests_avoided']}/"
                f"{result['first_run_remote_requests']}, output {result['output_sha256']}, "
                f"static {result['static_prefetch_output_sha256']}, "
                f"static child {result['static_prefetch_child_output_sha256']}"
            )
        elif result.get("runtime_metric_ms") is not None:
            extra = f", first event {result['runtime_metric_ms']}ms"
        else:
            extra = ""
        print(f"{status} {result['name']}: {result['elapsed_ms']}ms{extra}")

    explanations = {
        "daemon_socket_ready": "the time before Gradle can send work to the Rust sidecar",
        "authoritative_rust_dag": "cold and warm real Gradle invocations handing a Java-library build to Rust RunBuild with zero JVM task forwards",
        "real_build_dependency_readthrough": "a real Gradle build warming Rust over HTTP, including listener static prefetch, then rerunning from a fresh Gradle user home with zero remote requests",
        "file_watch_first_event": "the delay before source edits become observable",
        "dependency_transport_store_checksum": "the bounded Rust path for Maven bytes, local store, cache-first transport reuse, cache hit, and checksum verification",
        "dependency_metadata_transport_cache": "URL-only POM downloads through Rust warming the Rust metadata cache",
        "dependency_dynamic_metadata_transport_cache": "maven-metadata.xml downloads warming the dynamic-version metadata cache",
        "dependency_static_maven_prefetch": "Rust resolving a static Maven module and prefetching the artifact into the Rust store with checksum evidence",
        "dependency_artifact_readthrough": "Gradle skipping remote artifact access when Rust already has the JAR",
        "dependency_metadata_readthrough": "Gradle skipping remote POM metadata access and routing uncached resource downloads through Rust",
    }
    print("\nWhy these are visible:")
    for result in results:
        explanation = explanations.get(str(result["name"]))
        if explanation:
            print(f"- {result['name']} is {explanation}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-build", action="store_true", help="fail if the daemon binary is missing")
    parser.add_argument(
        "--mode",
        choices=("fast", "proof", "all"),
        default="fast",
        help="fast reports user-visible runtime wins; proof adds heavier subsystem smoke checks; all runs both",
    )
    parser.add_argument("--output", type=Path, help="write JSON metrics to this path")
    args = parser.parse_args()

    ensure_daemon_built(args.skip_build)
    def fast_results() -> list[dict[str, object]]:
        return [
            measure_daemon_readiness(),
            measure_authoritative_runbuild_fast(),
            measure_real_build_dependency_readthrough(),
            measure_cargo_test(
                "file_watch_first_event",
                "file_watch_reports_first_change_quickly",
                timeout=60,
            ),
        ]

    def proof_results() -> list[dict[str, object]]:
        return [
            measure_cargo_test(
                "dependency_transport_store_checksum",
                "test_download_",
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
            measure_cargo_test(
                "dependency_static_maven_prefetch",
                "test_resolve_dependencies_prefetches_static_maven_artifact",
                timeout=60,
            ),
            *measure_gradle_readthrough_smoke(),
        ]

    if args.mode == "fast":
        results = fast_results()
    elif args.mode == "proof":
        results = proof_results()
    else:
        results = fast_results() + proof_results()

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
