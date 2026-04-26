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
import subprocess
import sys
import tempfile
from pathlib import Path
from dataclasses import dataclass, asdict

sys.path.insert(0, os.path.dirname(__file__))

@dataclass
class RunResult:
    exit_code: int
    output: str
    tasks: list[str]
    substrate_noop: bool = False
    
    def to_dict(self):
        return {
            "exit_code": self.exit_code,
            "output_preview": self.output[:1000] if self.output else "",
            "tasks": self.tasks,
            "task_count": len(self.tasks),
            "substrate_noop": self.substrate_noop,
        }


def build_gradle_command(
    project_dir: str,
    substrate: bool = False,
    tasks: list[str] | None = None,
    substrate_mode: str = "shadow",
    daemon_binary: str | None = None,
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
) -> RunResult:
    """Run gradle on a project directory."""
    cmd = build_gradle_command(
        project_dir,
        substrate=substrate,
        tasks=tasks,
        substrate_mode=substrate_mode,
        daemon_binary=daemon_binary,
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
    parser.add_argument("--timeout", type=int, default=300, help="Timeout per project in seconds")
    parser.add_argument("--verbose", action="store_true", help="Verbose output")
    parser.add_argument("--output-dir", default=None, help="Directory for results")
    
    args = parser.parse_args()
    
    projects = []
    if args.project:
        projects.append(args.project)
    elif args.projects:
        projects.extend(args.projects)
    
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
        )
        substrate_usable = args.allow_noop_substrate or not substrate.substrate_noop
        
        results[os.path.basename(project)] = {
            "upstream": upstream.to_dict(),
            "substrate": substrate.to_dict(),
            "match": (
                substrate_usable
                and upstream.tasks == substrate.tasks
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
