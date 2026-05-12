# Rust Substrate Dogfood Manifest

`testing/dogfood/manifest.json` is the next evidence layer after the curated
corpus. It groups realistic Gradle build shapes that should be meaningful in
the first minute of use and records whether each project is expected to run
through the Rust kernel or fail closed before execution.

Validate and enumerate the manifest without invoking Gradle:

```bash
python3 tools/dogfood_runner/run.py --manifest testing/dogfood/manifest.json --list
```

Emit machine-readable inventory:

```bash
python3 tools/dogfood_runner/run.py --manifest testing/dogfood/manifest.json --json
```

This is not a full Gradle compatibility claim. The dogfood manifest is a
showability gate: supported entries must prove no JVM task forwards and parity
for configured checks; unsupported entries must produce precise fail-closed
diagnostics instead of hidden task-by-task fallback.
