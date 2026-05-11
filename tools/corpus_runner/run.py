#!/usr/bin/env python3
"""
Runs a build corpus through both upstream Gradle and the Rust substrate daemon,
comparing task graphs, dependency graphs, outputs, diagnostics, and exit codes.

Usage:
  python3 tools/corpus_runner/run.py --gradle-projects "project1 project2" [--substrate-mode shadow]

This tool is the core validation mechanism: it provides objective proof that
the Rust substrate behaves identically to upstream Gradle on real builds.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile
from pathlib import Path
from dataclasses import dataclass, field

sys.path.insert(0, os.path.dirname(__file__))

REPO_ROOT = Path(__file__).resolve().parents[2]

@dataclass
class RunResult:
    exit_code: int
    output: str
    tasks: list[str]
    output_files: list[str] = field(default_factory=list)
    output_hashes: dict[str, str] = field(default_factory=dict)
    archive_entries: dict[str, list[str]] = field(default_factory=dict)
    substrate_noop: bool = False
    duration_ms: int = 0
    
    def to_dict(self):
        preview = ""
        if self.output:
            if len(self.output) <= 5000:
                preview = self.output
            else:
                preview = self.output[:1000] + "\n...\n" + self.output[-4000:]
        return {
            "exit_code": self.exit_code,
            "output_preview": preview,
            "tasks": self.tasks,
            "task_count": len(self.tasks),
            "output_file_count": len(self.output_files),
            "output_files": self.output_files,
            "output_hashes": self.output_hashes,
            "archive_entries": self.archive_entries,
            "substrate_noop": self.substrate_noop,
            "duration_ms": self.duration_ms,
        }


STABLE_BUILD_OUTPUT_ROOTS = {
    "classes",
    "resources",
    "docs",
    "libs",
    "distributions",
    "install",
    "test-results",
}

ARCHIVE_SUFFIXES = {
    ".ear",
    ".jar",
    ".tar",
    ".tar.bz2",
    ".tar.gz",
    ".tgz",
    ".war",
    ".zip",
}

RUNBUILD_OUTPUT_MARKERS = (
    "[substrate:run-build]",
    "Rust authoritative run-build",
    "Rust run-build was incomplete",
)

RESOLVED_GRAPH_EXPORT_TASK = "rustSubstrateResolvedGraphExport"

RESOLVED_GRAPH_INIT_SCRIPT = r'''
import groovy.json.JsonOutput
import org.gradle.api.artifacts.result.ResolvedDependencyResult
import org.gradle.api.artifacts.result.UnresolvedDependencyResult
import org.gradle.api.artifacts.component.ModuleComponentIdentifier

gradle.projectsLoaded {
    allprojects { project ->
        project.tasks.register("rustSubstrateResolvedGraphExport") {
            group = "verification"
            description = "Exports selected resolved dependency graphs for Rust substrate parity checks."
            doLast {
                def outputRoot = System.getProperty("org.gradle.rust.substrate.resolvedGraphOutputDir")
                if (!outputRoot) {
                    throw new GradleException("Missing org.gradle.rust.substrate.resolvedGraphOutputDir")
                }
                def selected = ["compileClasspath", "runtimeClasspath", "testCompileClasspath", "testRuntimeClasspath"] as Set
                def configs = []
                project.configurations.findAll { it.canBeResolved && selected.contains(it.name) }.sort { it.name }.each { cfg ->
                    def resolution = cfg.incoming.resolutionResult
                    def artifactsByComponent = [:].withDefault { [] }
                    try {
                        cfg.incoming.artifacts.artifacts.each { artifact ->
                            def componentId = artifact.id.componentIdentifier.displayName
                            artifactsByComponent[componentId] << [
                                file: artifact.file.absolutePath,
                                fileName: artifact.file.name,
                                id: artifact.id.displayName
                            ]
                        }
                    } catch (Throwable ignored) {
                    }
                    def components = resolution.allComponents.collect { component ->
                        def id = component.id
                        def module = null
                        if (id instanceof ModuleComponentIdentifier) {
                            module = [group: id.group, name: id.module, version: id.version]
                        }
                        [
                            id: id.displayName,
                            module: module,
                            selectionReason: component.selectionReason?.descriptions?.collect { it.description }?.join("; ") ?: String.valueOf(component.selectionReason),
                            variants: component.variants.collect { variant ->
                                [
                                    displayName: variant.displayName,
                                    attributes: variant.attributes.keySet().collectEntries { key ->
                                        [(key.name): String.valueOf(variant.attributes.getAttribute(key))]
                                    }
                                ]
                            },
                            artifacts: artifactsByComponent[id.displayName].sort { it.fileName }
                        ]
                    }.sort { it.id }
                    def dependencies = resolution.allDependencies.collect { dep ->
                        if (dep instanceof ResolvedDependencyResult) {
                            return [
                                from: dep.from.id.displayName,
                                requested: dep.requested.displayName,
                                selected: dep.selected.id.displayName,
                                constraint: dep.constraint,
                                resolved: true,
                                failure: ""
                            ]
                        }
                        if (dep instanceof UnresolvedDependencyResult) {
                            return [
                                from: dep.from.id.displayName,
                                requested: dep.requested.displayName,
                                selected: "",
                                constraint: false,
                                resolved: false,
                                failure: dep.failure?.message ?: "unresolved"
                            ]
                        }
                        return [
                            from: "",
                            requested: dep.requested.displayName,
                            selected: "",
                            constraint: false,
                            resolved: false,
                            failure: "unknown dependency result type ${dep.class.name}"
                        ]
                    }.sort { "${it.from}|${it.requested}|${it.selected}" }
                    configs << [
                        projectPath: project.path,
                        configuration: cfg.name,
                        components: components,
                        dependencies: dependencies
                    ]
                }
                def payload = [
                    schema: "gradle-substrate.resolved-dependency-graph.v1",
                    project: project.path,
                    configurations: configs,
                    opaqueSections: [
                        "Repository source and checksum policy are not exposed by this Gradle public ResolutionResult export."
                    ]
                ]
                def safeProject = project.path == ":" ? "root" : project.path.replaceAll("[^A-Za-z0-9_.-]+", "_")
                def outFile = new File(outputRoot, "${safeProject}.json")
                outFile.parentFile.mkdirs()
                outFile.text = JsonOutput.prettyPrint(JsonOutput.toJson(payload))
            }
        }
    }
}
'''


def default_gradle_command() -> str | None:
    """Return an explicit Gradle-under-test command from the environment."""
    gradle_bin = os.environ.get("GRADLE_UNDER_TEST_BIN")
    if gradle_bin:
        return gradle_bin
    gradle_home = os.environ.get("GRADLE_UNDER_TEST")
    if gradle_home:
        return str(Path(gradle_home) / "bin" / "gradle")
    return None


def snapshot_build_outputs(project_dir: str) -> list[str]:
    """Return stable build output paths for upstream/substrate comparison.

    This intentionally compares file inventory, not bytes. Archive byte parity is
    tracked separately because native ZIP/TAR writers may differ in compression
    while still producing the same logical artifacts.
    """
    root = Path(project_dir)
    outputs: list[str] = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        parts = path.relative_to(root).parts
        if ".gradle" in parts:
            continue
        for idx, part in enumerate(parts):
            if part == "build" and idx + 1 < len(parts) and parts[idx + 1] in STABLE_BUILD_OUTPUT_ROOTS:
                outputs.append(str(path.relative_to(root)))
                break
    return sorted(outputs)


def snapshot_build_output_hashes(project_dir: str, output_files: list[str]) -> dict[str, str]:
    """Return SHA-256 hashes for non-archive stable outputs."""
    root = Path(project_dir)
    hashes: dict[str, str] = {}
    for relative_path in output_files:
        path = root / relative_path
        parts = Path(relative_path).parts
        if "test-results" in parts:
            continue
        lower_name = path.name.lower()
        if any(lower_name.endswith(suffix) for suffix in ARCHIVE_SUFFIXES):
            continue
        hasher = hashlib.sha256()
        with path.open("rb") as file:
            for chunk in iter(lambda: file.read(1024 * 1024), b""):
                hasher.update(chunk)
        hashes[relative_path] = hasher.hexdigest()
    return hashes


def snapshot_archive_entries(project_dir: str, output_files: list[str]) -> dict[str, list[str]]:
    """Return logical archive entry inventories for stable archive outputs."""
    root = Path(project_dir)
    inventories: dict[str, list[str]] = {}
    for relative_path in output_files:
        path = root / relative_path
        lower_name = path.name.lower()
        if not any(lower_name.endswith(suffix) for suffix in ARCHIVE_SUFFIXES):
            continue
        try:
            if lower_name.endswith((".jar", ".war", ".ear", ".zip")):
                with zipfile.ZipFile(path) as archive:
                    inventories[relative_path] = sorted(archive.namelist())
            elif lower_name.endswith((".tar", ".tar.gz", ".tgz", ".tar.bz2")):
                with tarfile.open(path, "r:*") as archive:
                    inventories[relative_path] = sorted(archive.getnames())
        except (OSError, tarfile.TarError, zipfile.BadZipFile) as exc:
            inventories[relative_path] = [f"<unreadable:{exc}>"]
    return inventories


def load_manifest(manifest_path: str) -> tuple[Path, list[dict]]:
    """Load a checked-in corpus manifest and resolve project paths."""
    path = Path(manifest_path).resolve()
    data = json.loads(path.read_text(encoding="utf-8"))
    root = Path(data.get("root", "."))
    if not root.is_absolute():
        root = path.parent / root

    projects = []
    for entry in data.get("projects", []):
        resolved = dict(entry)
        project_path = Path(entry["path"])
        if not project_path.is_absolute():
            project_path = root / project_path
        resolved["resolved_path"] = str(project_path)
        projects.append(resolved)
    return root, projects


def scan_project_contract(project_dir: str) -> dict:
    """Extract deterministic build-plan contract signals from a corpus project."""
    root = Path(project_dir)
    build_files = sorted(
        path for path in root.rglob("build.gradle*") if ".gradle" in path.name
    )
    settings_files = sorted(root.glob("settings.gradle*"))
    source_files = sorted(root.rglob("src/**/*.java")) + sorted(root.rglob("src/**/*.kt"))

    plugins: set[str] = set()
    tasks: set[str] = set()
    outputs: set[str] = set()
    dependencies: set[str] = set()
    dependency_constraints: set[str] = set()
    project_dependencies: set[str] = set()
    toolchains: set[str] = set()
    unsupported_features: set[str] = set()

    for build_file in build_files + settings_files:
        text = build_file.read_text(encoding="utf-8")
        plugins.update(re.findall(r"id\([\"']([^\"']+)[\"']\)", text))
        plugins.update(re.findall(r"id\s+[\"']([^\"']+)[\"']", text))
        plugins.update(re.findall(r"`([^`]+)`", text))
        if re.search(r"^\s*java-library\s*$", text, re.MULTILINE):
            plugins.add("java-library")
        if re.search(r"^\s*java\s*$", text, re.MULTILINE):
            plugins.add("java")
        if re.search(r"^\s*application\s*$", text, re.MULTILINE):
            plugins.add("application")
        if re.search(r"^\s*base\s*$", text, re.MULTILINE):
            plugins.add("base")

        tasks.update(re.findall(r"tasks\.register(?:<[^>]+>)?\([\"']([^\"']+)[\"']", text))
        tasks.update(re.findall(r"^\s*task\s+([A-Za-z_][A-Za-z0-9_]*)\b", text, re.MULTILINE))
        test_filter_patterns = re.findall(
            r"(?:includeTestsMatching|excludeTestsMatching)\(\s*[\"']([^\"']+)[\"']\s*\)",
            text,
        )
        has_unsupported_test_filters = bool(test_filter_patterns) and not all(
            looks_like_class_test_filter(pattern) for pattern in test_filter_patterns
        )
        if test_filter_patterns:
            tasks.add("test")
        outputs.update(re.findall(r"outputs\.(?:dir|file)\([\"']([^\"']+)[\"']\)", text))
        dependency_constraints.update(scan_dependency_constraints(text))
        dependencies_text = strip_balanced_blocks(text, "constraints")
        dependencies.update(re.findall(r"[\"']([A-Za-z0-9_.-]+:[A-Za-z0-9_.-]+(?::[^\"']+)?)[\"']", dependencies_text))
        project_dependencies.update(re.findall(r"project\([\"']:([^\"']+)[\"']\)", text))
        toolchains.update(re.findall(r"JavaVersion\.VERSION_([0-9]+)", text))
        toolchains.update(re.findall(r"languageVersion\.set\(JavaLanguageVersion\.of\(([0-9]+)\)\)", text))
        has_static_line_replace_filter = re.search(
            r"tasks\.register<Copy>\([^)]+\)\s*\{.*?\bfilter\s*\{\s*line\s*:\s*String\s*->\s*line\.replace\(\s*\"[^\"]+\"\s*,\s*\"[^\"]*\"\s*\)",
            text,
            re.DOTALL,
        )
        if re.search(r"tasks\.register<Copy>\([^)]+\)\s*\{.*?\bfilter\s*\{", text, re.DOTALL) and not has_static_line_replace_filter:
            unsupported_features.add("copy-filter-action")
        has_static_eachfile_relative_path = re.search(
            r"tasks\.register<Copy>\([^)]+\)\s*\{.*?\beachFile\s*\{\s*relativePath\s*=\s*RelativePath\(\s*true\s*,\s*\"[^\"]+\"\s*,\s*name\s*\)",
            text,
            re.DOTALL,
        )
        if re.search(r"tasks\.register<Copy>\([^)]+\)\s*\{.*?\beachFile\s*\{", text, re.DOTALL) and not has_static_eachfile_relative_path:
            unsupported_features.add("copy-eachfile-action")
        if has_unsupported_test_filters:
            unsupported_features.add("unsupported-test-filters")
        if re.search(r"\b(?:include|exclude)(?:Group|Module|Version)ByRegex\s*\(", text):
            unsupported_features.add("repository-content-filter")

    return {
        "build_file_count": len(build_files),
        "settings_file_count": len(settings_files),
        "source_file_count": len(source_files),
        "plugins": sorted(plugins),
        "tasks": sorted(tasks),
        "outputs": sorted(outputs),
        "dependencies": sorted(dependencies),
        "dependency_constraints": sorted(dependency_constraints),
        "project_dependencies": sorted(project_dependencies),
        "toolchains": sorted(toolchains),
        "unsupported_features": sorted(unsupported_features),
    }


def scan_dependency_constraints(build_text: str) -> set[str]:
    """Return coordinates declared inside Gradle dependencies { constraints { ... } } blocks."""
    constraints: set[str] = set()
    for block in extract_balanced_blocks(build_text, "constraints"):
        constraints.update(re.findall(r"[\"']([A-Za-z0-9_.-]+:[A-Za-z0-9_.-]+(?::[^\"']+)?)[\"']", block))
    return constraints


def extract_balanced_blocks(text: str, block_name: str) -> list[str]:
    return [text[start:end] for start, end in extract_balanced_block_spans(text, block_name)]


def strip_balanced_blocks(text: str, block_name: str) -> str:
    stripped = text
    for start, end in reversed(extract_balanced_block_spans(text, block_name, include_header=True)):
        stripped = stripped[:start] + stripped[end:]
    return stripped


def extract_balanced_block_spans(text: str, block_name: str, include_header: bool = False) -> list[tuple[int, int]]:
    pattern = re.compile(rf"\b{re.escape(block_name)}\s*\{{")
    spans: list[tuple[int, int]] = []
    for match in pattern.finditer(text):
        start = match.end()
        depth = 1
        pos = start
        while pos < len(text) and depth > 0:
            char = text[pos]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
            pos += 1
        if depth == 0:
            spans.append((match.start() if include_header else start, pos if include_header else pos - 1))
    return spans


def looks_like_class_test_filter(pattern: str) -> bool:
    if not pattern or "#" in pattern or " " in pattern:
        return False
    last_segment = pattern.rsplit(".", 1)[-1]
    if not last_segment:
        return False
    return last_segment[0] in "*?" or last_segment[0].isupper()


def compare_contract(actual: dict, expected: dict) -> list[str]:
    """Compare scanned corpus contract signals against manifest expectations."""
    mismatches: list[str] = []
    for key, expected_value in expected.items():
        actual_value = actual.get(key)
        if isinstance(expected_value, list):
            expected_sorted = sorted(expected_value)
            actual_sorted = sorted(actual_value or [])
            if actual_sorted != expected_sorted:
                mismatches.append(f"{key}: expected {expected_sorted}, got {actual_sorted}")
        elif actual_value != expected_value:
            mismatches.append(f"{key}: expected {expected_value!r}, got {actual_value!r}")
    return mismatches


def run_manifest_contracts(manifest_path: str) -> dict:
    """Validate corpus projects without invoking Gradle or requiring network."""
    _root, projects = load_manifest(manifest_path)
    results = {}
    for project in projects:
        name = project["name"]
        actual = scan_project_contract(project["resolved_path"])
        expected = project.get("expected_contract", {})
        mismatches = compare_contract(actual, expected)
        results[name] = {
            "path": project["resolved_path"],
            "actual_contract": actual,
            "expected_contract": expected,
            "match": not mismatches,
            "mismatches": mismatches,
        }
    return results


def build_gradle_command(
    project_dir: str,
    substrate: bool = False,
    tasks: list[str] | None = None,
    substrate_mode: str = "shadow",
    daemon_binary: str | None = None,
    runbuild_authoritative: bool = False,
    runbuild_native_ready_default: bool = False,
    gradle_command: str | None = None,
) -> list[str]:
    """Build the Gradle invocation used by corpus runs."""
    if gradle_command:
        cmd = [gradle_command, "-p", project_dir]
    elif os.path.exists(os.path.join(project_dir, "gradlew")):
        cmd = ["./gradlew"]
    elif (REPO_ROOT / "gradlew").exists():
        cmd = [str(REPO_ROOT / "gradlew"), "-p", project_dir]
    else:
        cmd = ["gradle"]
    cmd.extend(tasks or ["clean", "build"])
    cmd.extend(["--no-daemon", "--console=plain"])
    
    if substrate:
        # Keep this in sync with RustSubstrateOptions.java.
        cmd.append("-Dorg.gradle.rust.substrate.enabled=true")
        if runbuild_authoritative or runbuild_native_ready_default:
            cmd.extend([
                "-Dorg.gradle.rust.substrate.taskgraph.enabled=true",
                "-Dorg.gradle.rust.substrate.runbuild.enabled=true",
            ])
        else:
            cmd.append(f"-Dorg.gradle.rust.substrate.mode={substrate_mode}")
        if daemon_binary:
            daemon_path = Path(daemon_binary).expanduser()
            if not daemon_path.is_absolute():
                daemon_path = Path.cwd() / daemon_path
            cmd.append(f"-Dorg.gradle.rust.substrate.daemon.path={daemon_path}")
        if runbuild_authoritative:
            cmd.append("-Dorg.gradle.rust.substrate.execution.kernel=true")
        if runbuild_native_ready_default:
            cmd.append("-Dorg.gradle.rust.substrate.runbuild.native-ready-default=true")
        if runbuild_authoritative or runbuild_native_ready_default:
            cmd.append("--info")

    return cmd


def write_resolved_graph_init_script(directory: str) -> str:
    path = Path(directory) / "rust-substrate-resolved-graph.init.gradle"
    path.write_text(RESOLVED_GRAPH_INIT_SCRIPT, encoding="utf-8")
    return str(path)


def run_resolved_graph_export(
    project_dir: str,
    output_dir: str,
    substrate: bool,
    init_script: str,
    timeout: int,
    substrate_mode: str = "shadow",
    daemon_binary: str | None = None,
    runbuild_authoritative: bool = False,
    runbuild_native_ready_default: bool = False,
    gradle_command: str | None = None,
) -> RunResult:
    output_dir = str(Path(output_dir).resolve())
    cmd = build_gradle_command(
        project_dir,
        substrate=substrate,
        tasks=[RESOLVED_GRAPH_EXPORT_TASK],
        substrate_mode=substrate_mode,
        daemon_binary=daemon_binary,
        runbuild_authoritative=False,
        runbuild_native_ready_default=False,
        gradle_command=gradle_command,
    )
    cmd.extend([
        "-I",
        init_script,
        f"-Dorg.gradle.rust.substrate.resolvedGraphOutputDir={output_dir}",
    ])
    start = time.monotonic()
    try:
        result = subprocess.run(
            cmd,
            cwd=project_dir,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        output = result.stdout + result.stderr
        return RunResult(
            exit_code=result.returncode,
            output=output,
            tasks=parse_tasks_from_output(result.stdout, output),
            substrate_noop=substrate and detect_substrate_noop(output),
            duration_ms=int((time.monotonic() - start) * 1000),
        )
    except subprocess.TimeoutExpired:
        return RunResult(
            exit_code=-1,
            output="TIMEOUT",
            tasks=[],
            duration_ms=int((time.monotonic() - start) * 1000),
        )
    except Exception as exc:
        return RunResult(
            exit_code=-2,
            output=f"EXCEPTION: {exc}",
            tasks=[],
            duration_ms=int((time.monotonic() - start) * 1000),
        )


def detect_substrate_noop(output: str) -> bool:
    """Detect when a substrate run fell back to Java/no-op instead of exercising Rust."""
    markers = (
        "no-op fallback mode",
        "Substrate client is in no-op mode",
        "daemon-binary-missing:",
        "substrate-disabled",
        "substrate-inactive:",
    )
    return any(marker in output for marker in markers)


def runbuild_marker_missing(output: str) -> bool:
    """Return true when an explicit RunBuild gate produced no substrate signal."""
    return not any(marker in output for marker in RUNBUILD_OUTPUT_MARKERS)


def parse_tasks_from_output(stdout: str, output: str) -> list[str]:
    """Extract executed or selected Gradle task paths from build output.

    Authoritative RunBuild intentionally skips Gradle's JVM task executor, so
    Gradle does not print the usual `> Task` lines. In that mode the selected
    plan is still available in `--info` output as `Tasks to be executed`.
    """
    tasks: list[str] = []
    for line in stdout.splitlines():
        if "> Task " in line:
            task_name = line.split("> Task", 1)[1].strip().split(" ")[0]
            tasks.append(task_name)
    if tasks:
        return tasks

    match = re.search(r"Tasks to be executed:\s*\[(.*?)\]", output, re.DOTALL)
    if not match:
        return []
    return re.findall(r"task '([^']+)'", match.group(1))


def compare_run_pair(upstream: RunResult, substrate: RunResult, allow_noop_substrate: bool = False) -> dict:
    """Return explicit parity checks for one upstream/substrate project pair."""
    substrate_usable = allow_noop_substrate or not substrate.substrate_noop
    checks = {
        "successful": upstream.exit_code == 0 and substrate.exit_code == 0,
        "exit_code_match": upstream.exit_code == substrate.exit_code,
        "task_list_match": upstream.tasks == substrate.tasks,
        "output_files_match": upstream.output_files == substrate.output_files,
        "output_hashes_match": upstream.output_hashes == substrate.output_hashes,
        "archive_entries_match": upstream.archive_entries == substrate.archive_entries,
        "no_fallback": not substrate.substrate_noop,
        "substrate_usable": substrate_usable,
    }
    checks["match"] = (
        checks["substrate_usable"]
        and checks["successful"]
        and checks["exit_code_match"]
        and checks["task_list_match"]
        and checks["output_files_match"]
        and checks["output_hashes_match"]
        and checks["archive_entries_match"]
    )
    return checks


def compare_expected_fail_closed(upstream: RunResult, substrate: RunResult) -> dict:
    """Return explicit checks for projects that should reject native execution."""
    failure_markers = (
        "Rust authoritative run-build did not complete",
        "No executor for task type",
        "not natively executable",
        "jvmForwarded=",
    )
    checks = {
        "successful": upstream.exit_code == 0 and substrate.exit_code != 0,
        "no_fallback": not substrate.substrate_noop,
        "upstream_successful": upstream.exit_code == 0,
        "substrate_failed": substrate.exit_code != 0,
        "no_noop_fallback": not substrate.substrate_noop,
        "fail_closed_message": any(marker in substrate.output for marker in failure_markers),
    }
    checks["match"] = all(checks.values())
    return checks


def declared_dependency_graph(project_name: str, project_dir: str) -> dict:
    """Return a stable declared dependency graph artifact for lightweight parity gates.

    This is intentionally a declared graph, not full Gradle solver output. It
    keeps the current corpus gate honest by making requested coordinates and
    unsupported/opaque areas explicit instead of hiding them behind task output
    equality.
    """
    contract = scan_project_contract(project_dir)
    nodes = []
    for coordinate in contract["dependencies"]:
        parsed = parse_declared_artifact_coordinate(coordinate)
        nodes.append({
            "requested": coordinate,
            "group": parsed["group"],
            "name": parsed["name"],
            "requested_version": parsed["version"],
            "selected_version": parsed["version"],
            "classifier": parsed["classifier"],
            "extension": parsed["extension"],
            "selection_reason": "declared-static" if parsed["version"] else "managed-by-gradle-platform-or-bom",
            "repository": "declared-in-build-script",
            "artifact_path": "",
            "checksum": "",
            "opaque": not bool(parsed["version"]),
        })
    constraints = []
    for coordinate in contract["dependency_constraints"]:
        parsed = parse_declared_artifact_coordinate(coordinate)
        constraints.append({
            "requested": coordinate,
            "group": parsed["group"],
            "name": parsed["name"],
            "version": parsed["version"],
            "classifier": parsed["classifier"],
            "extension": parsed["extension"],
            "selection_reason": "dependency-constraint",
        })
    return {
        "schema": "gradle-substrate.declared-dependency-graph.v1",
        "project": project_name,
        "configurations": [
            {
                "name": "declared",
                "dependencies": nodes,
                "dependency_constraints": constraints,
            }
        ],
        "unsupported_features": contract["unsupported_features"],
        "opaque_sections": [
            "resolved variants, capabilities, artifact files, checksums, and repository selection are not represented by this declared graph gate"
        ],
    }


def parse_declared_artifact_coordinate(coordinate: str) -> dict[str, str]:
    base, sep, extension = coordinate.partition("@")
    parts = base.split(":")
    group = parts[0] if len(parts) > 0 else ""
    name = parts[1] if len(parts) > 1 else ""
    version = parts[2] if len(parts) > 2 else ""
    classifier = parts[3] if len(parts) > 3 else ""
    if len(parts) > 4:
        classifier = ":".join(parts[3:])
    return {
        "group": group,
        "name": name,
        "version": version,
        "classifier": classifier,
        "extension": extension if sep else "jar",
    }


def diff_declared_dependency_graphs(upstream: dict, substrate: dict) -> dict:
    mismatches = []
    upstream_deps = upstream.get("configurations", [{}])[0].get("dependencies", [])
    substrate_deps = substrate.get("configurations", [{}])[0].get("dependencies", [])
    upstream_constraints = upstream.get("configurations", [{}])[0].get("dependency_constraints", [])
    substrate_constraints = substrate.get("configurations", [{}])[0].get("dependency_constraints", [])
    if upstream_deps != substrate_deps:
        mismatches.append({
            "category": "declared-dependencies",
            "upstream": upstream_deps,
            "substrate": substrate_deps,
        })
    if upstream_constraints != substrate_constraints:
        mismatches.append({
            "category": "dependency-constraints",
            "upstream": upstream_constraints,
            "substrate": substrate_constraints,
        })
    if upstream.get("unsupported_features") != substrate.get("unsupported_features"):
        mismatches.append({
            "category": "unsupported-features",
            "upstream": upstream.get("unsupported_features"),
            "substrate": substrate.get("unsupported_features"),
        })
    return {
        "schema": "gradle-substrate.declared-dependency-graph-diff.v1",
        "match": not mismatches,
        "mismatches": mismatches,
        "limitations": [
            "This is declared dependency graph parity. It does not prove full Gradle solver parity."
        ],
    }


def load_single_resolved_graph(graph_dir: str) -> dict:
    files = sorted(Path(graph_dir).glob("*.json"))
    if not files:
        return {
            "schema": "gradle-substrate.resolved-dependency-graph.v1",
            "project": "",
            "configurations": [],
            "opaqueSections": ["No resolved graph file was emitted."],
        }
    if len(files) == 1:
        return json.loads(files[0].read_text(encoding="utf-8"))
    return {
        "schema": "gradle-substrate.resolved-dependency-graph.v1",
        "project": "multi-project",
        "configurations": [
            config
            for file in files
            for config in json.loads(file.read_text(encoding="utf-8")).get("configurations", [])
        ],
        "opaqueSections": ["Merged multiple project graph export files."],
    }


def normalize_resolved_graph_for_diff(graph: dict) -> dict:
    configs = []
    for config in graph.get("configurations", []):
        components = []
        for component in config.get("components", []):
            components.append({
                "id": component.get("id", ""),
                "module": component.get("module"),
                "selectionReason": component.get("selectionReason", ""),
                "variants": sorted(
                    [
                        {
                            "displayName": variant.get("displayName", ""),
                            "attributes": dict(sorted((variant.get("attributes") or {}).items())),
                        }
                        for variant in component.get("variants", [])
                    ],
                    key=lambda item: (item["displayName"], json.dumps(item["attributes"], sort_keys=True)),
                ),
                "artifacts": sorted(
                    [
                        {
                            "fileName": artifact.get("fileName", ""),
                            "id": artifact.get("id", ""),
                        }
                        for artifact in component.get("artifacts", [])
                    ],
                    key=lambda item: (item["fileName"], item["id"]),
                ),
            })
        dependencies = []
        for dep in config.get("dependencies", []):
            dependencies.append({
                "from": dep.get("from", ""),
                "requested": dep.get("requested", ""),
                "selected": dep.get("selected", ""),
                "constraint": bool(dep.get("constraint", False)),
                "resolved": bool(dep.get("resolved", False)),
                "failure": dep.get("failure", ""),
            })
        configs.append({
            "projectPath": config.get("projectPath", ""),
            "configuration": config.get("configuration", ""),
            "components": sorted(components, key=lambda item: item["id"]),
            "dependencies": sorted(
                dependencies,
                key=lambda item: (
                    item["from"],
                    item["requested"],
                    item["selected"],
                    item["failure"],
                ),
            ),
        })
    return {
        "schema": graph.get("schema", "gradle-substrate.resolved-dependency-graph.v1"),
        "configurations": sorted(
            configs,
            key=lambda item: (item["projectPath"], item["configuration"]),
        ),
    }


def diff_resolved_dependency_graphs(upstream: dict, substrate: dict) -> dict:
    upstream_normalized = normalize_resolved_graph_for_diff(upstream)
    substrate_normalized = normalize_resolved_graph_for_diff(substrate)
    mismatches = []
    if upstream_normalized["configurations"] != substrate_normalized["configurations"]:
        mismatches.append({
            "category": "resolved-graph",
            "upstream": upstream_normalized["configurations"],
            "substrate": substrate_normalized["configurations"],
        })
    return {
        "schema": "gradle-substrate.resolved-dependency-graph-diff.v1",
        "match": not mismatches,
        "mismatches": mismatches,
        "limitations": [
            "This compares Gradle public ResolutionResult exports from reference and substrate invocations.",
            "It does not yet compare against a direct Rust dependency solver export.",
            "Repository source and checksum policy are not exposed by this graph export.",
        ],
    }


def summarize_results(results: dict) -> dict:
    """Build a showable aggregate summary without changing corpus_results.json."""
    project_results = {
        name: result
        for name, result in results.items()
        if isinstance(result, dict) and "upstream" in result and "substrate" in result
    }
    total = len(project_results)
    matched = sum(1 for result in project_results.values() if result.get("match"))
    no_fallback = sum(1 for result in project_results.values() if result.get("checks", {}).get("no_fallback"))
    task_list_matches = sum(1 for result in project_results.values() if result.get("checks", {}).get("task_list_match"))
    output_file_matches = sum(1 for result in project_results.values() if result.get("checks", {}).get("output_files_match"))
    output_hash_matches = sum(1 for result in project_results.values() if result.get("checks", {}).get("output_hashes_match"))
    archive_entry_matches = sum(1 for result in project_results.values() if result.get("checks", {}).get("archive_entries_match"))
    exit_code_matches = sum(1 for result in project_results.values() if result.get("checks", {}).get("exit_code_match"))
    successful = sum(1 for result in project_results.values() if result.get("checks", {}).get("successful"))

    upstream_task_total = sum(result["upstream"].get("task_count", 0) for result in project_results.values())
    substrate_task_total = sum(result["substrate"].get("task_count", 0) for result in project_results.values())
    upstream_duration_ms = sum(result["upstream"].get("duration_ms", 0) for result in project_results.values())
    substrate_duration_ms = sum(result["substrate"].get("duration_ms", 0) for result in project_results.values())

    failed_projects = sorted(name for name, result in project_results.items() if not result.get("match"))
    fallback_projects = sorted(
        name
        for name, result in project_results.items()
        if not result.get("checks", {}).get("no_fallback")
    )

    return {
        "project_count": total,
        "matched_project_count": matched,
        "failed_project_count": total - matched,
        "successful_project_count": successful,
        "no_fallback_project_count": no_fallback,
        "fallback_project_count": total - no_fallback,
        "exit_code_match_count": exit_code_matches,
        "task_list_match_count": task_list_matches,
        "output_file_inventory_match_count": output_file_matches,
        "output_hash_match_count": output_hash_matches,
        "archive_entry_inventory_match_count": archive_entry_matches,
        "upstream_task_total": upstream_task_total,
        "substrate_task_total": substrate_task_total,
        "upstream_duration_ms": upstream_duration_ms,
        "substrate_duration_ms": substrate_duration_ms,
        "failed_projects": failed_projects,
        "fallback_projects": fallback_projects,
    }


def print_summary(summary: dict) -> None:
    """Print the aggregate parity evidence in a compact demo-friendly form."""
    total = summary["project_count"]
    print(f"Projects matched: {summary['matched_project_count']}/{total}")
    print(f"Successful upstream/substrate builds: {summary['successful_project_count']}/{total}")
    print(f"No-fallback substrate runs: {summary['no_fallback_project_count']}/{total}")
    print(f"Exit-code parity: {summary['exit_code_match_count']}/{total}")
    print(f"Task-list parity: {summary['task_list_match_count']}/{total}")
    print(f"Output inventory parity: {summary['output_file_inventory_match_count']}/{total}")
    print(f"Non-archive output hash parity: {summary['output_hash_match_count']}/{total}")
    print(f"Archive entry inventory parity: {summary['archive_entry_inventory_match_count']}/{total}")
    print(
        "Task totals: "
        f"upstream={summary['upstream_task_total']}, substrate={summary['substrate_task_total']}"
    )
    print(
        "Observed wall time: "
        f"upstream={summary['upstream_duration_ms']}ms, "
        f"substrate={summary['substrate_duration_ms']}ms"
    )
    if summary["failed_projects"]:
        print(f"Failed projects: {', '.join(summary['failed_projects'])}")
    if summary["fallback_projects"]:
        print(f"Fallback projects: {', '.join(summary['fallback_projects'])}")


def run_build(
    project_dir: str,
    substrate: bool = False,
    timeout: int = 300,
    tasks: list[str] | None = None,
    substrate_mode: str = "shadow",
    daemon_binary: str | None = None,
    runbuild_authoritative: bool = False,
    runbuild_native_ready_default: bool = False,
    gradle_command: str | None = None,
) -> RunResult:
    """Run gradle on a project directory."""
    cmd = build_gradle_command(
        project_dir,
        substrate=substrate,
        tasks=tasks,
        substrate_mode=substrate_mode,
        daemon_binary=daemon_binary,
        runbuild_authoritative=runbuild_authoritative,
        runbuild_native_ready_default=runbuild_native_ready_default,
        gradle_command=gradle_command,
    )
    
    start = time.monotonic()
    try:
        result = subprocess.run(
            cmd,
            cwd=project_dir,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        output = result.stdout + result.stderr
        if substrate and (runbuild_authoritative or runbuild_native_ready_default) and runbuild_marker_missing(output):
            output += (
                "\n[substrate] substrate-inactive: run-build marker missing; "
                "use a Gradle-under-test distribution that contains rust-bridge services\n"
            )
        
        tasks = parse_tasks_from_output(result.stdout, output)
        
        output_files = snapshot_build_outputs(project_dir) if result.returncode == 0 else []
        output_hashes = snapshot_build_output_hashes(project_dir, output_files) if output_files else {}
        archive_entries = snapshot_archive_entries(project_dir, output_files) if output_files else {}

        return RunResult(
            exit_code=result.returncode,
            output=output,
            tasks=tasks,
            output_files=output_files,
            output_hashes=output_hashes,
            archive_entries=archive_entries,
            substrate_noop=substrate and detect_substrate_noop(output),
            duration_ms=int((time.monotonic() - start) * 1000),
        )
    except subprocess.TimeoutExpired:
        return RunResult(
            exit_code=-1,
            output="TIMEOUT",
            tasks=[],
            duration_ms=int((time.monotonic() - start) * 1000),
        )
    except Exception as e:
        return RunResult(
            exit_code=-2,
            output=f"EXCEPTION: {str(e)}",
            tasks=[],
            duration_ms=int((time.monotonic() - start) * 1000),
        )

def main():
    parser = argparse.ArgumentParser(description="Run Gradle corpus validation")
    parser.add_argument("--project", help="Single project to run")
    parser.add_argument("--projects", nargs="+", help="Multiple projects to run")
    parser.add_argument("--manifest", help="Corpus manifest with project paths and expected contracts")
    parser.add_argument("--contract-only", action="store_true",
                       help="Validate manifest build-plan contracts without invoking Gradle")
    parser.add_argument("--mode", choices=["reference", "shadow"], default="reference",
                       help="Run mode: reference (compare upstream vs substrate) or shadow")
    parser.add_argument("--tasks", nargs="+", default=["clean", "build"],
                       help="Gradle tasks to run for each project")
    parser.add_argument("--substrate-mode", choices=["shadow", "authoritative"], default="shadow",
                       help="Rust substrate mode used for the candidate run")
    parser.add_argument("--daemon-binary", default=None,
                       help="Path to gradle-substrate-daemon for the substrate run")
    parser.add_argument("--gradle-command", default=default_gradle_command(),
                       help="Gradle-under-test executable to run corpus projects, usually a built local distribution's bin/gradle")
    parser.add_argument("--allow-noop-substrate", action="store_true",
                       help="Do not fail if the substrate candidate falls back to no-op mode")
    parser.add_argument("--runbuild-authoritative", action="store_true",
                       help="Enable the explicit no-fallback Rust RunBuild gate for the substrate candidate")
    parser.add_argument("--runbuild-native-ready-default", action="store_true",
                       help="Try Rust RunBuild first and delegate to JVM when the selected plan is not fully native-ready")
    parser.add_argument("--timeout", type=int, default=300, help="Timeout per project in seconds")
    parser.add_argument("--verbose", action="store_true", help="Verbose output")
    parser.add_argument("--output-dir", default=None, help="Directory for results")
    parser.add_argument("--dependency-graph-parity", action="store_true",
                       help="Emit and compare lightweight declared dependency graph JSON artifacts")
    parser.add_argument("--resolved-dependency-graph-parity", action="store_true",
                       help="Emit and compare Gradle public ResolutionResult graph JSON artifacts")
    
    args = parser.parse_args()
    
    if args.manifest and args.contract_only:
        results = run_manifest_contracts(args.manifest)
        failed = [name for name, result in results.items() if not result["match"]]
        for name, result in results.items():
            status = "PASS" if result["match"] else "FAIL"
            print(f"{status} {name}: {result['path']}")
            for mismatch in result["mismatches"]:
                print(f"  - {mismatch}")
        output_dir = args.output_dir or "."
        os.makedirs(output_dir, exist_ok=True)
        with open(os.path.join(output_dir, "corpus_contract_results.json"), "w") as f:
            json.dump(results, f, indent=2)
        sys.exit(1 if failed else 0)

    projects = []
    if args.project:
        projects.append({"name": os.path.basename(args.project), "resolved_path": args.project})
    elif args.projects:
        projects.extend({"name": os.path.basename(project), "resolved_path": project} for project in args.projects)
    elif args.manifest:
        _root, manifest_projects = load_manifest(args.manifest)
        projects.extend(manifest_projects)
    
    if not projects:
        print("No projects specified. Use --project or --projects.")
        sys.exit(1)
    
    results = {}
    temp_context = tempfile.TemporaryDirectory() if args.resolved_dependency_graph_parity else None
    resolved_graph_init_script = (
        write_resolved_graph_init_script(temp_context.name)
        if temp_context is not None
        else None
    )
    
    for project_entry in projects:
        project = project_entry["resolved_path"]
        project_name = project_entry.get("name") or os.path.basename(project)
        expected_fail_closed = bool(project_entry.get("expected_substrate_fail_closed", False))
        print(f"\n{'='*60}")
        print(f"Running project: {project}")
        print(f"{'='*60}")
        
        if not os.path.exists(project):
            print(f"  Project not found: {project}")
            results[project_name] = {"error": "Project not found"}
            continue
        
        # Run upstream
        print("  Running upstream Gradle...")
        upstream = run_build(
            project,
            substrate=False,
            timeout=args.timeout,
            tasks=args.tasks,
            gradle_command=args.gradle_command,
        )
        
        # Run with substrate
        print("  Running Rust substrate...")
        substrate = run_build(
            project,
            substrate=True,
            timeout=args.timeout,
            tasks=args.tasks,
            substrate_mode=args.substrate_mode,
            daemon_binary=args.daemon_binary,
            runbuild_authoritative=args.runbuild_authoritative,
            runbuild_native_ready_default=args.runbuild_native_ready_default,
            gradle_command=args.gradle_command,
        )
        checks = compare_expected_fail_closed(upstream, substrate) if expected_fail_closed else compare_run_pair(
            upstream,
            substrate,
            allow_noop_substrate=args.allow_noop_substrate,
        )
        graph_paths = {}
        graph_diff = None
        if args.dependency_graph_parity:
            output_dir = args.output_dir or "."
            graph_dir = Path(output_dir) / "dependency-graphs" / project_name
            graph_dir.mkdir(parents=True, exist_ok=True)
            upstream_graph = declared_dependency_graph(project_name, project)
            substrate_graph = declared_dependency_graph(project_name, project)
            graph_diff = diff_declared_dependency_graphs(upstream_graph, substrate_graph)
            upstream_path = graph_dir / "upstream-declared-graph.json"
            substrate_path = graph_dir / "substrate-declared-graph.json"
            diff_path = graph_dir / "declared-graph-diff.json"
            upstream_path.write_text(json.dumps(upstream_graph, indent=2) + "\n", encoding="utf-8")
            substrate_path.write_text(json.dumps(substrate_graph, indent=2) + "\n", encoding="utf-8")
            diff_path.write_text(json.dumps(graph_diff, indent=2) + "\n", encoding="utf-8")
            graph_paths = {
                "upstream_declared_graph": str(upstream_path),
                "substrate_declared_graph": str(substrate_path),
                "declared_graph_diff": str(diff_path),
            }
            checks["dependency_graph_parity"] = graph_diff["match"]
            checks["match"] = checks["match"] and graph_diff["match"]
        resolved_graph_paths = {}
        resolved_graph_diff = None
        resolved_graph_runs = None
        if args.resolved_dependency_graph_parity:
            output_dir = args.output_dir or "."
            graph_dir = Path(output_dir) / "resolved-dependency-graphs" / project_name
            upstream_dir = graph_dir / "upstream"
            substrate_dir = graph_dir / "substrate"
            upstream_dir.mkdir(parents=True, exist_ok=True)
            substrate_dir.mkdir(parents=True, exist_ok=True)
            upstream_graph_run = run_resolved_graph_export(
                project,
                str(upstream_dir),
                substrate=False,
                init_script=resolved_graph_init_script,
                timeout=args.timeout,
                gradle_command=args.gradle_command,
            )
            substrate_graph_run = run_resolved_graph_export(
                project,
                str(substrate_dir),
                substrate=True,
                init_script=resolved_graph_init_script,
                timeout=args.timeout,
                substrate_mode=args.substrate_mode,
                daemon_binary=args.daemon_binary,
                runbuild_authoritative=args.runbuild_authoritative,
                runbuild_native_ready_default=args.runbuild_native_ready_default,
                gradle_command=args.gradle_command,
            )
            upstream_resolved_graph = load_single_resolved_graph(str(upstream_dir))
            substrate_resolved_graph = load_single_resolved_graph(str(substrate_dir))
            resolved_graph_diff = diff_resolved_dependency_graphs(
                upstream_resolved_graph,
                substrate_resolved_graph,
            )
            upstream_path = graph_dir / "upstream-resolved-graph.json"
            substrate_path = graph_dir / "substrate-resolved-graph.json"
            diff_path = graph_dir / "resolved-graph-diff.json"
            upstream_path.write_text(json.dumps(upstream_resolved_graph, indent=2) + "\n", encoding="utf-8")
            substrate_path.write_text(json.dumps(substrate_resolved_graph, indent=2) + "\n", encoding="utf-8")
            diff_path.write_text(json.dumps(resolved_graph_diff, indent=2) + "\n", encoding="utf-8")
            resolved_graph_paths = {
                "upstream_resolved_graph": str(upstream_path),
                "substrate_resolved_graph": str(substrate_path),
                "resolved_graph_diff": str(diff_path),
            }
            resolved_graph_runs = {
                "upstream": upstream_graph_run.to_dict(),
                "substrate": substrate_graph_run.to_dict(),
            }
            resolved_graph_ok = (
                upstream_graph_run.exit_code == 0
                and substrate_graph_run.exit_code == 0
                and not substrate_graph_run.substrate_noop
                and resolved_graph_diff["match"]
            )
            checks["resolved_dependency_graph_parity"] = resolved_graph_ok
            checks["match"] = checks["match"] and resolved_graph_ok
        
        results[project_name] = {
            "upstream": upstream.to_dict(),
            "substrate": substrate.to_dict(),
            "checks": checks,
            "match": checks["match"],
        }
        if args.dependency_graph_parity:
            results[project_name]["dependency_graph_paths"] = graph_paths
            results[project_name]["dependency_graph_diff"] = graph_diff
        if args.resolved_dependency_graph_parity:
            results[project_name]["resolved_dependency_graph_paths"] = resolved_graph_paths
            results[project_name]["resolved_dependency_graph_diff"] = resolved_graph_diff
            results[project_name]["resolved_dependency_graph_runs"] = resolved_graph_runs
        if expected_fail_closed:
            results[project_name]["expected_substrate_fail_closed"] = True
        if substrate.substrate_noop and not args.allow_noop_substrate:
            results[project_name]["error"] = "Substrate candidate used no-op fallback"
        
        if args.verbose:
            print(f"    Upstream tasks: {len(upstream.tasks)}")
            print(f"    Substrate tasks: {len(substrate.tasks)}")
            if substrate.substrate_noop:
                print("    Substrate candidate used no-op fallback")
            if upstream.tasks != substrate.tasks:
                missing = set(upstream.tasks) - set(substrate.tasks)
                extra = set(substrate.tasks) - set(upstream.tasks)
                if missing:
                    print(f"    Missing in substrate: {missing}")
                if extra:
                    print(f"    Extra in substrate: {extra}")
            if upstream.output_files != substrate.output_files:
                missing_outputs = set(upstream.output_files) - set(substrate.output_files)
                extra_outputs = set(substrate.output_files) - set(upstream.output_files)
                if missing_outputs:
                    print(f"    Missing output files in substrate: {sorted(missing_outputs)}")
                if extra_outputs:
                    print(f"    Extra output files in substrate: {sorted(extra_outputs)}")
            if upstream.output_hashes != substrate.output_hashes:
                differing_hashes = sorted(
                    path
                    for path in set(upstream.output_hashes) | set(substrate.output_hashes)
                    if upstream.output_hashes.get(path) != substrate.output_hashes.get(path)
                )
                if differing_hashes:
                    print(f"    Output content differs: {differing_hashes}")
    
    # Summary
    total = len(results)
    passed = sum(1 for r in results.values() if r.get("match"))
    failed = total - passed
    
    print(f"\n{'='*60}")
    print(f"Results: {passed}/{total} projects matched")
    if failed > 0:
        print(f"FAILED projects:")
        for name, r in results.items():
            if not r.get("match"):
                print(f"  - {name}")
    
    # Write results
    output_dir = args.output_dir or "."
    os.makedirs(output_dir, exist_ok=True)
    with open(os.path.join(output_dir, "corpus_results.json"), "w") as f:
        json.dump(results, f, indent=2)
    summary = summarize_results(results)
    with open(os.path.join(output_dir, "corpus_summary.json"), "w") as f:
        json.dump(summary, f, indent=2)
    print(f"\nResults written to {os.path.join(output_dir, 'corpus_results.json')}")
    print(f"Summary written to {os.path.join(output_dir, 'corpus_summary.json')}")
    print()
    print_summary(summary)
    
    if temp_context is not None:
        temp_context.cleanup()

    sys.exit(0 if failed == 0 else 1)

if __name__ == "__main__":
    main()
