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


if __name__ == "__main__":
    unittest.main()
