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


if __name__ == "__main__":
    unittest.main()
