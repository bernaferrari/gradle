#!/usr/bin/env python3

import importlib.util
import tempfile
import unittest
import zipfile
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
        self.assertIn("-Dorg.gradle.rust.substrate.taskgraph.enabled=true", command)
        self.assertIn("-Dorg.gradle.rust.substrate.runbuild.enabled=true", command)
        self.assertNotIn("-Dorg.gradle.rust.substrate.mode=shadow", command)
        self.assertIn("--info", command)

    def test_runbuild_native_ready_default_adds_delegating_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = corpus_run.build_gradle_command(
                tmp,
                substrate=True,
                tasks=["build"],
                runbuild_native_ready_default=True,
            )

        self.assertIn(
            "-Dorg.gradle.rust.substrate.runbuild.native-ready-default=true",
            command,
        )
        self.assertIn("-Dorg.gradle.rust.substrate.taskgraph.enabled=true", command)
        self.assertIn("-Dorg.gradle.rust.substrate.runbuild.enabled=true", command)
        self.assertNotIn("-Dorg.gradle.rust.substrate.mode=shadow", command)
        self.assertIn("--info", command)

    def test_prefers_project_wrapper_when_present(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "gradlew").touch()

            command = corpus_run.build_gradle_command(tmp, substrate=False, tasks=["help"])

        self.assertEqual("./gradlew", command[0])
        self.assertIn("help", command)
        self.assertIn("--no-daemon", command)
        self.assertIn("--console=plain", command)

    def test_uses_repo_wrapper_for_projects_without_wrapper(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = corpus_run.build_gradle_command(tmp, substrate=False, tasks=["help"])

        self.assertEqual(str(corpus_run.REPO_ROOT / "gradlew"), command[0])
        self.assertEqual(["-p", tmp], command[1:3])
        self.assertIn("help", command)

    def test_explicit_gradle_command_overrides_wrappers(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "gradlew").touch()

            command = corpus_run.build_gradle_command(
                tmp,
                substrate=False,
                tasks=["help"],
                gradle_command="/opt/gradle-under-test/bin/gradle",
            )

        self.assertEqual("/opt/gradle-under-test/bin/gradle", command[0])
        self.assertEqual(["-p", tmp], command[1:3])

    def test_detects_noop_substrate_output(self):
        self.assertTrue(
            corpus_run.detect_substrate_noop(
                "[substrate] Rust substrate is enabled but bridge is running in no-op fallback mode: "
                "daemon-binary-missing:/tmp/gradle-substrate-daemon"
            )
        )
        self.assertTrue(
            corpus_run.detect_substrate_noop(
                "[substrate] substrate-inactive: run-build marker missing"
            )
        )
        self.assertFalse(corpus_run.detect_substrate_noop("BUILD SUCCESSFUL"))

    def test_detects_missing_runbuild_marker(self):
        self.assertTrue(corpus_run.runbuild_marker_missing("BUILD SUCCESSFUL"))
        self.assertFalse(
            corpus_run.runbuild_marker_missing(
                "[substrate:run-build] Rust executed 3 tasks from build-plan-cache"
            )
        )

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

    def test_external_corpus_manifest_contracts_pass(self):
        repo_root = Path(__file__).resolve().parents[2]
        manifest = repo_root / "testing" / "corpus" / "external-manifest.json"

        results = corpus_run.run_manifest_contracts(str(manifest))

        self.assertIn("java-junit-kotlin-dsl", results)
        failures = {
            name: result["mismatches"]
            for name, result in results.items()
            if not result["match"]
        }
        self.assertEqual({}, failures)

    def test_unsupported_corpus_manifest_contracts_pass(self):
        repo_root = Path(__file__).resolve().parents[2]
        manifest = repo_root / "testing" / "corpus" / "unsupported-manifest.json"

        results = corpus_run.run_manifest_contracts(str(manifest))

        self.assertIn("custom-task-unsupported-kotlin-dsl", results)
        self.assertIn("copy-filter-unsupported-kotlin-dsl", results)
        self.assertIn("copy-eachfile-unsupported-kotlin-dsl", results)
        self.assertIn("test-filters-unsupported-kotlin-dsl", results)
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

    def test_archive_entry_snapshot_reads_zip_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive_path = root / "build" / "libs" / "sample.zip"
            archive_path.parent.mkdir(parents=True)
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr("nested/app.txt", "app")
                archive.writestr("readme.txt", "readme")

            entries = corpus_run.snapshot_archive_entries(
                tmp,
                ["build/libs/sample.zip"],
            )

        self.assertEqual(
            {"build/libs/sample.zip": ["nested/app.txt", "readme.txt"]},
            entries,
        )

    def test_compare_run_pair_reports_explicit_checks(self):
        upstream = corpus_run.RunResult(
            exit_code=0,
            output="",
            tasks=[":compileJava", ":jar"],
            output_files=["build/classes/java/main/App.class"],
            output_hashes={"build/classes/java/main/App.class": "abc"},
        )
        substrate = corpus_run.RunResult(
            exit_code=0,
            output="",
            tasks=[":compileJava", ":jar"],
            output_files=["build/classes/java/main/App.class"],
            output_hashes={"build/classes/java/main/App.class": "abc"},
            substrate_noop=False,
        )

        checks = corpus_run.compare_run_pair(upstream, substrate)

        self.assertTrue(checks["match"])
        self.assertTrue(checks["successful"])
        self.assertTrue(checks["no_fallback"])
        self.assertTrue(checks["task_list_match"])
        self.assertTrue(checks["output_hashes_match"])

    def test_compare_run_pair_rejects_noop_substrate_by_default(self):
        upstream = corpus_run.RunResult(exit_code=0, output="", tasks=[":build"])
        substrate = corpus_run.RunResult(
            exit_code=0,
            output="no-op fallback mode",
            tasks=[":build"],
            substrate_noop=True,
        )

        checks = corpus_run.compare_run_pair(upstream, substrate)

        self.assertFalse(checks["match"])
        self.assertFalse(checks["no_fallback"])
        self.assertFalse(checks["substrate_usable"])

    def test_compare_run_pair_rejects_equal_failed_builds(self):
        upstream = corpus_run.RunResult(exit_code=1, output="failed", tasks=[])
        substrate = corpus_run.RunResult(exit_code=1, output="failed", tasks=[])

        checks = corpus_run.compare_run_pair(upstream, substrate)

        self.assertFalse(checks["match"])
        self.assertFalse(checks["successful"])
        self.assertTrue(checks["exit_code_match"])

    def test_compare_expected_fail_closed_accepts_substrate_failure(self):
        upstream = corpus_run.RunResult(exit_code=0, output="BUILD SUCCESSFUL", tasks=[":build"])
        substrate = corpus_run.RunResult(
            exit_code=1,
            output="Rust authoritative run-build did not complete: No executor for task type",
            tasks=[],
            substrate_noop=False,
        )

        checks = corpus_run.compare_expected_fail_closed(upstream, substrate)

        self.assertTrue(checks["match"])
        self.assertTrue(checks["successful"])
        self.assertTrue(checks["no_fallback"])

    def test_summarize_results_counts_parity_and_fallbacks(self):
        results = {
            "ok": {
                "upstream": {"task_count": 2, "duration_ms": 100},
                "substrate": {"task_count": 2, "duration_ms": 80},
                "checks": {
                    "successful": True,
                    "exit_code_match": True,
                    "task_list_match": True,
                    "output_files_match": True,
                    "output_hashes_match": True,
                    "no_fallback": True,
                },
                "match": True,
            },
            "fallback": {
                "upstream": {"task_count": 1, "duration_ms": 50},
                "substrate": {"task_count": 1, "duration_ms": 40},
                "checks": {
                    "successful": False,
                    "exit_code_match": True,
                    "task_list_match": True,
                    "output_files_match": True,
                    "output_hashes_match": True,
                    "no_fallback": False,
                },
                "match": False,
            },
        }

        summary = corpus_run.summarize_results(results)

        self.assertEqual(2, summary["project_count"])
        self.assertEqual(1, summary["matched_project_count"])
        self.assertEqual(1, summary["successful_project_count"])
        self.assertEqual(1, summary["no_fallback_project_count"])
        self.assertEqual(3, summary["upstream_task_total"])
        self.assertEqual(["fallback"], summary["failed_projects"])
        self.assertEqual(["fallback"], summary["fallback_projects"])


if __name__ == "__main__":
    unittest.main()
