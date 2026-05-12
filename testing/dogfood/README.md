# Rust Substrate Dogfood Manifest

`testing/dogfood/manifest.json` is the next evidence layer after the curated
corpus. It groups realistic Gradle build shapes that should be meaningful in
the first minute of use and records whether each project is expected to run
through the Rust kernel or fail closed before execution.

`testing/dogfood/oss-manifest.json` is the next layer after that: pinned
external OSS repositories. It currently includes a supported Spring guide
`clean assemble` slice, a supported Spring PetClinic `clean compileJava` slice,
and strict fail-closed gates for broader Spring/Kotlin-heavy builds.

Validate and enumerate the manifest without invoking Gradle:

```bash
python3 tools/dogfood_runner/run.py --manifest testing/dogfood/manifest.json --list
```

Emit machine-readable inventory:

```bash
python3 tools/dogfood_runner/run.py --manifest testing/dogfood/manifest.json --json
```

Validate the external OSS manifest without cloning:

```bash
python3 tools/dogfood_runner/run.py --manifest testing/dogfood/oss-manifest.json --list
```

Fetch pinned external OSS entries into `build/dogfood-oss/sources`:

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/oss-manifest.json \
  --fetch-only \
  --output-dir build/dogfood-oss
```

Execute the dogfood manifest against an installed Gradle-under-test and Rust
daemon:

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/manifest.json \
  --execute \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood
```

The execution path writes `dogfood-results.json`, one `result.json` per project,
and `dogfood-summary.md`.

This is not a full Gradle compatibility claim. The dogfood manifest is a
showability gate: supported entries must prove no JVM task forwards and parity
for configured checks; unsupported entries must produce precise fail-closed
diagnostics instead of hidden task-by-task fallback.
