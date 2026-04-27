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

# Run in shadow mode with an explicit substrate daemon binary
python3 tools/corpus_runner/run.py \
  --project /path/to/project \
  --substrate-mode shadow \
  --daemon-binary /path/to/gradle-substrate-daemon

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

`--contract-only` does not invoke Gradle. It scans checked-in sample projects
and validates deterministic build-plan signals such as plugins, declared tasks,
declared outputs, source files, dependencies, and Java toolchain declarations.
This is suitable for quick CI gates and keeps the corpus useful without network
access.

## Corpus Structure

A corpus should be a directory containing Gradle projects:

```

The repository includes a small offline corpus at `testing/corpus/` with a
manifest. Keep samples deterministic: prefer built-in Gradle plugins and local
sources; avoid external repositories or dependencies unless the manifest and
runner are extended to pin/cache them explicitly.
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

Results are written to `corpus_results.json` in the corpus directory:

```json
{
  "timestamp": "2024-01-15T12:00:00Z",
  "projects": {
    "project-a": {
      "upstream": {"exit_code": 0, "duration_ms": 1234},
      "substrate": {"exit_code": 0, "duration_ms": 1100},
      "match": true,
      "details": {...}
    }
  },
  "summary": {
    "total": 10,
    "passed": 8,
    "failed": 2,
    "coverage_pct": 80
  }
}
```

## Adding Projects to Corpus

Copy or symlink Gradle projects into the corpus directory. The runner discovers
all projects with `build.gradle` or `build.gradle.kts` files.
