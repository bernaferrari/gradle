# Corpus Runner

Runs Gradle build projects through both upstream Gradle and the Rust substrate daemon, comparing outputs to verify behavioral parity.

## Quick Start

```bash
# Run on a corpus directory
python3 tools/corpus_runner/run.py --projects /path/to/corpus

# Run on a single project
python3 tools/corpus_runner/run.py --project /path/to/single/project

# Validate the checked-in offline corpus contracts
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --contract-only

# Validate the optional networked corpus contracts
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --contract-only

# Run in shadow mode with an explicit substrate daemon binary
python3 tools/corpus_runner/run.py \
  --project /path/to/project \
  --substrate-mode shadow \
  --daemon-binary /path/to/gradle-substrate-daemon

# Run the explicit no-fallback RunBuild gate against the checked-in corpus
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative

# Run the explicit no-fallback RunBuild gate against the networked JUnit corpus
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative

# Run with verbose output
python3 tools/corpus_runner/run.py --project /path/to/project --verbose
```

## How It Works

The corpus runner executes each build project twice:

1. **Upstream reference**: Runs with vanilla Gradle daemon
2. **Rust substrate**: Runs with the Rust substrate daemon enabled

It then compares:
- Task graph (task names and dependencies)
- Exit codes
- Build duration (informational only)
- Output files
- Diagnostics/warnings

The substrate candidate is considered invalid if Gradle reports that it used
no-op fallback mode. Use `--allow-noop-substrate` only when explicitly testing
fallback behavior rather than Rust parity.

`--runbuild-authoritative` adds
`-Dorg.gradle.rust.substrate.runbuild.authoritative=true`. This is stricter
than umbrella authoritative mode: Gradle skips its JVM task executor only when
Rust `RunBuild` completes the selected plan with zero JVM forwards and the exact
scheduled task count.

`--contract-only` does not invoke Gradle. It scans checked-in sample projects
and validates deterministic build-plan signals such as plugins, declared tasks,
declared outputs, source files, external dependencies, project dependencies, and
Java toolchain declarations. This is suitable for quick CI gates and keeps the
corpus useful without network access.

## Corpus Structure

The repository includes a small offline corpus at `testing/corpus/manifest.json`
covering Java library, Java application, and Java multi-project builds. The
optional `testing/corpus/external-manifest.json` adds a pinned JUnit Platform
sample for real non-empty test execution and requires network or a warm Gradle
dependency cache.

Reference-mode runs compare upstream Gradle and Rust substrate exit codes, task
lists, and stable build output file inventories under `build/classes`,
`build/resources`, `build/libs`, `build/distributions`, `build/install`, and
`build/test-results`. Non-archive outputs are also compared by SHA-256 hash.
Archive byte-for-byte parity and test-result content parity are tracked
separately because compression, metadata, timings, and binary result stores can
differ while logical artifacts are still equivalent.

Keep offline samples deterministic: prefer built-in Gradle plugins and local
sources. Put external repositories or dependencies in the external manifest with
pinned coordinates.

A corpus should be a directory containing Gradle projects:

```
my-corpus/
  project-a/
    build.gradle
    src/...
  project-b/
    build.gradle.kts
    src/...
  shared/
    settings.gradle
    build.gradle
```

## Output

Results are written to `corpus_results.json` in the selected output directory:

```json
{
  "project-a": {
    "upstream": {
      "exit_code": 0,
      "tasks": [":compileJava", ":jar"],
      "output_file_count": 2,
      "output_files": ["build/classes/java/main/App.class", "build/libs/app.jar"],
      "output_hashes": {"build/classes/java/main/App.class": "..."}
    },
    "substrate": {
      "exit_code": 0,
      "tasks": [":compileJava", ":jar"],
      "output_file_count": 2,
      "output_files": ["build/classes/java/main/App.class", "build/libs/app.jar"],
      "output_hashes": {"build/classes/java/main/App.class": "..."}
    },
    "match": true
  }
}
```

## Adding Projects to Corpus

Copy or symlink Gradle projects into the corpus directory. The runner discovers
all projects with `build.gradle` or `build.gradle.kts` files.
