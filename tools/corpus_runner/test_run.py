#!/usr/bin/env python3

import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("run.py")
SPEC = importlib.util.spec_from_file_location("corpus_runner_run", MODULE_PATH)
corpus_run = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(corpus_run)


class CorpusRunnerCommandTest(unittest.TestCase):
    def test_substrate_command_uses_real_gradle_option_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = corpus_run.build_gradle_command(
                tmp,
                substrate=True,
                tasks=["help"],
                substrate_mode="shadow",
                daemon_binary="/tmp/gradle-substrate-daemon",
            )

        self.assertIn("-Dorg.gradle.rust.substrate.enabled=true", command)
        self.assertIn("-Dorg.gradle.rust.substrate.mode=shadow", command)
        self.assertIn(
            "-Dorg.gradle.rust.substrate.daemon.path=/tmp/gradle-substrate-daemon",
            command,
        )
        self.assertNotIn("-Dorg.gradle.rust.substrate.enable=true", command)

    def test_runbuild_authoritative_adds_explicit_no_fallback_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = corpus_run.build_gradle_command(
                tmp,
                substrate=True,
                tasks=["build"],
                runbuild_authoritative=True,
            )

        self.assertIn(
            "-Dorg.gradle.rust.substrate.runbuild.authoritative=true",
            command,
        )
        self.assertNotIn("-Dorg.gradle.rust.substrate.runbuild.enabled=true", command)

    def test_prefers_project_wrapper_when_present(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "gradlew").touch()

            command = corpus_run.build_gradle_command(tmp, substrate=False, tasks=["help"])

        self.assertEqual("./gradlew", command[0])
        self.assertIn("help", command)
        self.assertIn("--no-daemon", command)
        self.assertIn("--console=plain", command)

    def test_detects_noop_substrate_output(self):
        self.assertTrue(
            corpus_run.detect_substrate_noop(
                "[substrate] Rust substrate is enabled but bridge is running in no-op fallback mode: "
                "daemon-binary-missing:/tmp/gradle-substrate-daemon"
            )
        )
        self.assertFalse(corpus_run.detect_substrate_noop("BUILD SUCCESSFUL"))

    def test_checked_in_corpus_manifest_contracts_pass(self):
        repo_root = Path(__file__).resolve().parents[2]
        manifest = repo_root / "testing" / "corpus" / "manifest.json"

        results = corpus_run.run_manifest_contracts(str(manifest))

        self.assertIn("java-library-kotlin-dsl", results)
        self.assertIn("java-application-groovy-dsl", results)
        self.assertIn("java-multiproject-kotlin-dsl", results)
        failures = {
            name: result["mismatches"]
            for name, result in results.items()
            if not result["match"]
        }
        self.assertEqual({}, failures)

    def test_contract_comparison_reports_mismatch(self):
        mismatches = corpus_run.compare_contract(
            {"plugins": ["java"], "source_file_count": 1},
            {"plugins": ["java-library"], "source_file_count": 2},
        )

        self.assertEqual(2, len(mismatches))
        self.assertTrue(any("plugins" in mismatch for mismatch in mismatches))


if __name__ == "__main__":
    unittest.main()
