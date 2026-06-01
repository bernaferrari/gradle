import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "tools" / "warm_runner" / "run.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("warm_runner", RUNNER)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


warm_runner = load_runner()


class WarmRunnerTest(unittest.TestCase):
    def test_parse_direct_runbuild_output(self):
        signals = warm_runner.parse_direct_runbuild_output(
            "direct-runbuild status=COMPLETED tasks=14 succeeded=14 failed=0 skipped=4 "
            "up_to_date=8 from_cache=0 jvm_forwarded=0 duration_ms=48 "
            "plan_source=build-plan-shadow"
        )

        self.assertTrue(signals["marker"])
        self.assertEqual("COMPLETED", signals["status"])
        self.assertEqual(14, signals["tasks"])
        self.assertEqual(0, signals["jvm_forwarded"])
        self.assertEqual("build-plan-shadow", signals["plan_source"])

    def test_classifies_recapturable_direct_rejections(self):
        self.assertEqual(
            ("cache-miss", "no cached build-plan artifact"),
            warm_runner.classify_direct_rejection(1, "no cached build-plan artifact"),
        )
        self.assertEqual(
            ("stale", "cached build-plan artifact is stale"),
            warm_runner.classify_direct_rejection(1, "cached build-plan artifact is stale"),
        )
        self.assertTrue(warm_runner.should_capture_after_direct_rejection("cache-miss"))
        self.assertTrue(warm_runner.should_capture_after_direct_rejection("stale"))

    def test_classifies_non_recapturable_rejections(self):
        kind, _ = warm_runner.classify_direct_rejection(
            1,
            "multiple cached build-plan artifacts match project",
        )
        self.assertEqual("ambiguous-cache", kind)
        self.assertFalse(warm_runner.should_capture_after_direct_rejection(kind))

    def test_direct_command_forwards_tasks_and_changed_paths(self):
        cmd = warm_runner.direct_runbuild_command(
            Path("/bin/runbuild"),
            "tcp://127.0.0.1:1",
            Path("/state"),
            Path("/repo"),
            [":build"],
            4,
            [Path("src/Main.java")],
            Path("/tmp/changed.txt"),
        )

        self.assertIn("--task", cmd)
        self.assertIn(":build", cmd)
        self.assertIn("--changed-path", cmd)
        self.assertIn("src/Main.java", cmd)
        self.assertIn("--changed-paths-file", cmd)
        self.assertIn("/tmp/changed.txt", cmd)

    def test_gradle_capture_command_uses_strict_runbuild_flags(self):
        cmd = warm_runner.gradle_capture_command(
            Path("/bin/gradle"),
            Path("/repo"),
            ["clean", "build"],
            Path("/bin/daemon"),
            Path("/state"),
            Path("/home"),
        )

        self.assertIn("-Dorg.gradle.rust.substrate.runbuild.enabled=true", cmd)
        self.assertIn("-Dorg.gradle.rust.substrate.execution.kernel=true", cmd)
        self.assertIn("-Dorg.gradle.rust.substrate.state.dir=/state", cmd)
        self.assertIn("--gradle-user-home=/home", cmd)


if __name__ == "__main__":
    unittest.main()
