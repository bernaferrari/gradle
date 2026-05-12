#!/usr/bin/env python3

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("run.py")
SPEC = importlib.util.spec_from_file_location("dogfood_runner_run", MODULE_PATH)
dogfood_run = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = dogfood_run
SPEC.loader.exec_module(dogfood_run)


class DogfoodRunnerTest(unittest.TestCase):
    def test_checked_in_manifest_validates(self):
        errors = dogfood_run.validate_manifest(dogfood_run.DEFAULT_MANIFEST)

        self.assertEqual([], errors)

    def test_enumerates_manifest_without_executing_gradle(self):
        summary = dogfood_run.enumerate_manifest(dogfood_run.DEFAULT_MANIFEST)

        self.assertTrue(summary["valid"])
        self.assertGreaterEqual(summary["project_count"], 5)
        self.assertGreaterEqual(summary["supported_count"], 3)
        self.assertGreaterEqual(summary["fail_closed_count"], 1)

    def test_checked_in_oss_manifest_validates_without_cloning(self):
        manifest = dogfood_run.REPO_ROOT / "testing" / "dogfood" / "oss-manifest.json"

        errors = dogfood_run.validate_manifest(manifest)

        self.assertEqual([], errors)

    def test_rejects_missing_supported_no_jvm_forward_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            project = Path(tmp) / "project"
            project.mkdir()
            manifest = Path(tmp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "schema": "gradle-substrate.dogfood-manifest.v1",
                        "root": ".",
                        "projects": [
                            {
                                "name": "p",
                                "path": "project",
                                "source": {"kind": "checked-in-local", "description": "fixture"},
                                "tasks": ["build"],
                                "mode": "strict",
                                "expectation": "supported",
                                "timeout_seconds": 1,
                                "reason": "test",
                                "checks": {"task_parity": True},
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            errors = dogfood_run.validate_manifest(manifest)

        self.assertIn("p: supported entries must require no_jvm_forwards", errors)

    def test_rejects_floating_git_ref(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "schema": "gradle-substrate.dogfood-manifest.v1",
                        "root": ".",
                        "projects": [
                            {
                                "name": "floating",
                                "path": "floating",
                                "source": {
                                    "kind": "git",
                                    "url": "https://example.invalid/repo.git",
                                    "ref": "main",
                                    "subdir": ".",
                                    "description": "floating",
                                },
                                "tasks": ["build"],
                                "mode": "native-ready-default",
                                "expectation": "supported-or-fail-closed",
                                "timeout_seconds": 1,
                                "reason": "test",
                                "checks": {
                                    "task_parity": True,
                                    "no_jvm_forwards": True,
                                    "fail_closed_diagnostic": "required-if-not-supported",
                                },
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            errors = dogfood_run.validate_manifest(manifest)

        self.assertIn("floating: git source requires immutable 40-character ref", errors)

    def test_builds_git_fetch_commands(self):
        project = dogfood_run.DogfoodProject(
            name="sample",
            path=Path("/unused"),
            tasks=["help"],
            mode="native-ready-default",
            expectation="supported-or-fail-closed",
            timeout_seconds=1,
            reason="test",
            checks={},
            source={
                "kind": "git",
                "url": "https://example.invalid/repo.git",
                "ref": "0123456789abcdef0123456789abcdef01234567",
                "subdir": ".",
            },
        )

        commands = dogfood_run.git_fetch_commands(project, Path("/tmp/cache"))

        self.assertEqual(
            ["git", "clone", "--no-checkout", "https://example.invalid/repo.git", "/tmp/cache/sample"],
            commands[0],
        )
        self.assertEqual(
            ["git", "-C", "/tmp/cache/sample", "fetch", "--depth", "1", "origin", "0123456789abcdef0123456789abcdef01234567"],
            commands[1],
        )

    def test_materializes_git_subdir_under_source_cache(self):
        project = dogfood_run.DogfoodProject(
            name="sample",
            path=Path("/unused"),
            tasks=["help"],
            mode="native-ready-default",
            expectation="supported-or-fail-closed",
            timeout_seconds=1,
            reason="test",
            checks={},
            source={"kind": "git", "subdir": "complete"},
        )

        materialized = dogfood_run.materialized_project(project, Path("/tmp/cache"))

        self.assertEqual(Path("/tmp/cache/sample/complete").resolve(), materialized.path)

    def test_parse_jvm_forwards_from_runbuild_output(self):
        self.assertEqual(7, dogfood_run.parse_jvm_forwards("failed jvmForwarded=7"))
        self.assertEqual(0, dogfood_run.parse_jvm_forwards("JVM forwarding disabled"))
        self.assertEqual(-1, dogfood_run.parse_jvm_forwards("BUILD SUCCESSFUL"))

    def test_parse_substrate_signals(self):
        signals = dogfood_run.parse_substrate_signals(
            "[substrate] Connecting to existing daemon at tcp://127.0.0.1:1\n"
            "[substrate:taskgraph] captured 3 selected task contracts\n"
            "[substrate:run-build] Rust executed 3 Gradle tasks from build-plan-cache "
            "with 0 up-to-date; JVM forwarding disabled"
        )

        self.assertTrue(signals["daemon_reused"])
        self.assertTrue(signals["runbuild_marker"])
        self.assertTrue(signals["taskgraph_captured"])
        self.assertEqual("build-plan-cache", signals["plan_source"])
        self.assertEqual(0, signals["jvm_forward_count"])

    def test_summarizes_pass_and_fail_closed_results(self):
        results = [
            {
                "name": "supported",
                "expectation": "supported",
                "match": True,
                "upstream": {"duration_ms": 100, "task_count": 2},
                "substrate": {"duration_ms": 50, "task_count": 2},
                "checks": {"jvm_forward_count": 0},
                "substrate_signals": {
                    "runbuild_marker": True,
                    "taskgraph_captured": True,
                    "daemon_started": True,
                    "daemon_reused": False,
                },
            },
            {
                "name": "unsupported",
                "expectation": "fail-closed",
                "match": True,
                "upstream": {"duration_ms": 100, "task_count": 1},
                "substrate": {"duration_ms": 10, "task_count": 0},
                "checks": {"fail_closed_message": True},
                "substrate_signals": {
                    "runbuild_marker": True,
                    "taskgraph_captured": True,
                    "daemon_started": False,
                    "daemon_reused": True,
                },
            },
            {
                "name": "drift",
                "expectation": "supported",
                "match": False,
                "upstream": {"duration_ms": 100, "task_count": 1},
                "substrate": {"duration_ms": 90, "task_count": 2},
                "checks": {"jvm_forward_count": 1},
                "substrate_signals": {
                    "runbuild_marker": False,
                    "taskgraph_captured": False,
                    "daemon_started": False,
                    "daemon_reused": False,
                },
            },
        ]

        summary = dogfood_run.summarize_execution(results)

        self.assertEqual(3, summary["project_count"])
        self.assertEqual(2, summary["matched_project_count"])
        self.assertEqual(1, summary["zero_jvm_forward_supported_count"])
        self.assertEqual(2, summary["runbuild_marker_count"])
        self.assertEqual(1, summary["daemon_started_count"])
        self.assertEqual(1, summary["daemon_reused_count"])
        self.assertEqual(["drift"], summary["failed_projects"])

    def test_writes_markdown_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            results = [
                {
                    "name": "supported",
                    "expectation": "supported",
                    "mode": "strict",
                    "match": True,
                    "upstream": {"duration_ms": 100, "task_count": 2},
                    "substrate": {"duration_ms": 50, "task_count": 2},
                    "checks": {"jvm_forward_count": 0},
                    "substrate_signals": {"plan_source": "build-plan-cache"},
                }
            ]
            summary = dogfood_run.summarize_execution(results)

            report = dogfood_run.write_markdown_report(Path(tmp), summary, results)
            text = report.read_text(encoding="utf-8")

        self.assertIn("Projects matched: 1/1", text)
        self.assertIn("not a 100% Gradle compatibility claim", text)

    def test_summary_names_task_and_output_drift_failures(self):
        results = [
            {
                "name": "task-drift",
                "expectation": "supported",
                "match": False,
                "upstream": {"duration_ms": 1, "task_count": 1},
                "substrate": {"duration_ms": 1, "task_count": 2},
                "checks": {
                    "task_list_match": False,
                    "output_hashes_match": True,
                    "jvm_forward_count": 0,
                },
            },
            {
                "name": "output-drift",
                "expectation": "supported",
                "match": False,
                "upstream": {"duration_ms": 1, "task_count": 1},
                "substrate": {"duration_ms": 1, "task_count": 1},
                "checks": {
                    "task_list_match": True,
                    "output_hashes_match": False,
                    "jvm_forward_count": 0,
                },
            },
        ]

        summary = dogfood_run.summarize_execution(results)

        self.assertEqual(["task-drift", "output-drift"], summary["failed_projects"])


if __name__ == "__main__":
    unittest.main()
