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

    def test_parse_jvm_forwards_from_runbuild_output(self):
        self.assertEqual(7, dogfood_run.parse_jvm_forwards("failed jvmForwarded=7"))
        self.assertEqual(0, dogfood_run.parse_jvm_forwards("JVM forwarding disabled"))
        self.assertEqual(-1, dogfood_run.parse_jvm_forwards("BUILD SUCCESSFUL"))

    def test_summarizes_pass_and_fail_closed_results(self):
        results = [
            {
                "name": "supported",
                "expectation": "supported",
                "match": True,
                "upstream": {"duration_ms": 100, "task_count": 2},
                "substrate": {"duration_ms": 50, "task_count": 2},
                "checks": {"jvm_forward_count": 0},
            },
            {
                "name": "unsupported",
                "expectation": "fail-closed",
                "match": True,
                "upstream": {"duration_ms": 100, "task_count": 1},
                "substrate": {"duration_ms": 10, "task_count": 0},
                "checks": {"fail_closed_message": True},
            },
            {
                "name": "drift",
                "expectation": "supported",
                "match": False,
                "upstream": {"duration_ms": 100, "task_count": 1},
                "substrate": {"duration_ms": 90, "task_count": 2},
                "checks": {"jvm_forward_count": 1},
            },
        ]

        summary = dogfood_run.summarize_execution(results)

        self.assertEqual(3, summary["project_count"])
        self.assertEqual(2, summary["matched_project_count"])
        self.assertEqual(1, summary["zero_jvm_forward_supported_count"])
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
