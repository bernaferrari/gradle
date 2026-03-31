# Corpus Runner

Runs Gradle build projects through both upstream Gradle and the Rust substrate daemon, comparing outputs to verify behavioral parity.

## Quick Start

```bash
# Run on a corpus directory
python3 tools/corpus_runner/run.py --projects /path/to/corpus

# Run on a single project
python3 tools/corpus_runner/run.py --project /path/to/single/project

# Run in shadow mode (Rust + JVM in parallel)
python3 tools/corpus_runner/run.py --project /path/to/project --mode shadow

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

## Corpus Structure

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
