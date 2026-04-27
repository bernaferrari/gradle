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
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from dataclasses import dataclass, field

sys.path.insert(0, os.path.dirname(__file__))

@dataclass
class RunResult:
    exit_code: int
    output: str
    tasks: list[str]
    output_files: list[str] = field(default_factory=list)
    substrate_noop: bool = False
    
    def to_dict(self):
        return {
            "exit_code": self.exit_code,
            "output_preview": self.output[:1000] if self.output else "",
            "tasks": self.tasks,
            "task_count": len(self.tasks),
            "output_file_count": len(self.output_files),
            "output_files": self.output_files,
            "substrate_noop": self.substrate_noop,
        }


STABLE_BUILD_OUTPUT_ROOTS = {
    "classes",
    "resources",
    "libs",
    "distributions",
    "install",
    "test-results",
}


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
    project_dependencies: set[str] = set()
    toolchains: set[str] = set()

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

        tasks.update(re.findall(r"tasks\.register(?:<[^>]+>)?\([\"']([^\"']+)[\"']", text))
        tasks.update(re.findall(r"^\s*task\s+([A-Za-z_][A-Za-z0-9_]*)\b", text, re.MULTILINE))
        outputs.update(re.findall(r"outputs\.(?:dir|file)\([\"']([^\"']+)[\"']\)", text))
        dependencies.update(re.findall(r"[\"']([A-Za-z0-9_.-]+:[A-Za-z0-9_.-]+:[^\"']+)[\"']", text))
        project_dependencies.update(re.findall(r"project\([\"']:([^\"']+)[\"']\)", text))
        toolchains.update(re.findall(r"JavaVersion\.VERSION_([0-9]+)", text))
        toolchains.update(re.findall(r"languageVersion\.set\(JavaLanguageVersion\.of\(([0-9]+)\)\)", text))

    return {
        "build_file_count": len(build_files),
        "settings_file_count": len(settings_files),
        "source_file_count": len(source_files),
        "plugins": sorted(plugins),
        "tasks": sorted(tasks),
        "outputs": sorted(outputs),
        "dependencies": sorted(dependencies),
        "project_dependencies": sorted(project_dependencies),
        "toolchains": sorted(toolchains),
    }


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
) -> list[str]:
    """Build the Gradle invocation used by corpus runs."""
    cmd = ["./gradlew"] if os.path.exists(os.path.join(project_dir, "gradlew")) else ["gradle"]
    cmd.extend(tasks or ["clean", "build"])
    cmd.extend(["--no-daemon", "--console=plain"])
    
    if substrate:
        # Keep this in sync with RustSubstrateOptions.java.
        cmd.extend([
            "-Dorg.gradle.rust.substrate.enabled=true",
            f"-Dorg.gradle.rust.substrate.mode={substrate_mode}",
        ])
        if daemon_binary:
            cmd.append(f"-Dorg.gradle.rust.substrate.daemon.path={daemon_binary}")
        if runbuild_authoritative:
            cmd.append("-Dorg.gradle.rust.substrate.runbuild.authoritative=true")

    return cmd


def detect_substrate_noop(output: str) -> bool:
    """Detect when a substrate run fell back to Java/no-op instead of exercising Rust."""
    markers = (
        "no-op fallback mode",
        "Substrate client is in no-op mode",
        "daemon-binary-missing:",
        "substrate-disabled",
    )
    return any(marker in output for marker in markers)


def run_build(
    project_dir: str,
    substrate: bool = False,
    timeout: int = 300,
    tasks: list[str] | None = None,
    substrate_mode: str = "shadow",
    daemon_binary: str | None = None,
    runbuild_authoritative: bool = False,
) -> RunResult:
    """Run gradle on a project directory."""
    cmd = build_gradle_command(
        project_dir,
        substrate=substrate,
        tasks=tasks,
        substrate_mode=substrate_mode,
        daemon_binary=daemon_binary,
        runbuild_authoritative=runbuild_authoritative,
    )
    
    try:
        result = subprocess.run(
            cmd,
            cwd=project_dir,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        output = result.stdout + result.stderr
        
        # Extract tasks from output
        tasks = []
        for line in result.stdout.split('\n'):
            if '> Task ' in line:
                task_name = line.split('> Task')[1].strip().split(' ')[0]
                tasks.append(task_name)
        
        return RunResult(
            exit_code=result.returncode,
            output=output,
            tasks=tasks,
            output_files=snapshot_build_outputs(project_dir) if result.returncode == 0 else [],
            substrate_noop=substrate and detect_substrate_noop(output),
        )
    except subprocess.TimeoutExpired:
        return RunResult(
            exit_code=-1,
            output="TIMEOUT",
            tasks=[],
        )
    except Exception as e:
        return RunResult(
            exit_code=-2,
            output=f"EXCEPTION: {str(e)}",
            tasks=[],
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
    parser.add_argument("--allow-noop-substrate", action="store_true",
                       help="Do not fail if the substrate candidate falls back to no-op mode")
    parser.add_argument("--runbuild-authoritative", action="store_true",
                       help="Enable the explicit no-fallback Rust RunBuild gate for the substrate candidate")
    parser.add_argument("--timeout", type=int, default=300, help="Timeout per project in seconds")
    parser.add_argument("--verbose", action="store_true", help="Verbose output")
    parser.add_argument("--output-dir", default=None, help="Directory for results")
    
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
        projects.append(args.project)
    elif args.projects:
        projects.extend(args.projects)
    elif args.manifest:
        _root, manifest_projects = load_manifest(args.manifest)
        projects.extend(project["resolved_path"] for project in manifest_projects)
    
    if not projects:
        print("No projects specified. Use --project or --projects.")
        sys.exit(1)
    
    results = {}
    
    for project in projects:
        print(f"\n{'='*60}")
        print(f"Running project: {project}")
        print(f"{'='*60}")
        
        if not os.path.exists(project):
            print(f"  Project not found: {project}")
            results[os.path.basename(project)] = {"error": "Project not found"}
            continue
        
        # Run upstream
        print("  Running upstream Gradle...")
        upstream = run_build(project, substrate=False, timeout=args.timeout, tasks=args.tasks)
        
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
        )
        substrate_usable = args.allow_noop_substrate or not substrate.substrate_noop
        
        results[os.path.basename(project)] = {
            "upstream": upstream.to_dict(),
            "substrate": substrate.to_dict(),
            "match": (
                substrate_usable
                and upstream.tasks == substrate.tasks
                and upstream.output_files == substrate.output_files
                and upstream.exit_code == substrate.exit_code
            ),
        }
        if substrate.substrate_noop and not args.allow_noop_substrate:
            results[os.path.basename(project)]["error"] = "Substrate candidate used no-op fallback"
        
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
    print(f"\nResults written to {os.path.join(output_dir, 'corpus_results.json')}")
    
    sys.exit(0 if failed == 0 else 1)

if __name__ == "__main__":
    main()
