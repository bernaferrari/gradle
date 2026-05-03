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

# Validate explicitly unsupported corpus contracts
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/unsupported-manifest.json \
  --contract-only

# Run in shadow mode with an explicit substrate daemon binary
python3 tools/corpus_runner/run.py \
  --project /path/to/project \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --substrate-mode shadow \
  --daemon-binary /path/to/gradle-substrate-daemon

# Run the explicit no-fallback RunBuild gate against the checked-in corpus
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative

# Run the explicit no-fallback RunBuild gate against the networked JUnit corpus
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative

# Run the networked corpus with declared dependency graph parity artifacts
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --dependency-graph-parity \
  --output-dir build/corpus-external-with-graphs

# Run the networked corpus with Gradle ResolutionResult graph parity artifacts
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --resolved-dependency-graph-parity \
  --output-dir build/corpus-external-with-resolved-graphs

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
- Output files
- Non-archive output hashes
- Archive entry inventories
- No-fallback substrate execution
- Optional declared dependency graph parity
- Optional Gradle `ResolutionResult` graph parity
- Build duration (informational only)

The substrate candidate is considered invalid if Gradle reports that it used
no-op fallback mode. Use `--allow-noop-substrate` only when explicitly testing
fallback behavior rather than Rust parity.

RunBuild parity claims must use a Gradle-under-test distribution built from
this fork. Pass it with `--gradle-command`, set `GRADLE_UNDER_TEST_BIN`, or set
`GRADLE_UNDER_TEST` to the distribution home. The repository wrapper is only a
bootstrap wrapper and may run an upstream Gradle distribution that does not
contain the Rust bridge. For explicit RunBuild gates, the runner adds `--info`
and rejects the substrate candidate if no `[substrate:run-build]` signal is
observed.

When `--daemon-binary` is relative, the runner converts it to an absolute path
before invoking Gradle from each corpus project directory. This prevents
project-local working directories from accidentally turning a real RunBuild gate
into no-op fallback.

`--runbuild-authoritative` adds
`-Dorg.gradle.rust.substrate.execution.kernel=true`. This is stricter
than umbrella authoritative mode: Gradle skips its JVM task executor only when
Rust `RunBuild` completes the selected plan with zero JVM forwards and the exact
scheduled task count. Current authoritative runs use task contracts captured at
Gradle task-graph population when they match the finalized execution plan, which
keeps classpaths and task inputs Gradle-owned while the Rust daemon controls the
scheduled DAG.

`--runbuild-native-ready-default` adds
`-Dorg.gradle.rust.substrate.runbuild.native-ready-default=true`. It tries Rust
RunBuild first and delegates back to the JVM executor when the selected plan is
not fully native-ready.

`--contract-only` does not invoke Gradle. It scans checked-in sample projects
and validates deterministic build-plan signals such as plugins, declared tasks,
declared outputs, source files, external dependencies, project dependencies, and
Java toolchain declarations. This is suitable for quick CI gates and keeps the
corpus useful without network access.

`--dependency-graph-parity` emits lightweight declared dependency graph
artifacts for each project under
`<output-dir>/dependency-graphs/<project-name>/`:

- `upstream-declared-graph.json`
- `substrate-declared-graph.json`
- `declared-graph-diff.json`

The diff participates in the project match result and fails the run when
declared dependency coordinates or unsupported feature markers drift. This is a
guardrail for runner/corpus honesty, not full resolved Gradle solver parity: it
does not prove repository selection, variant/capability selection, artifact
files, checksums, or rich conflict-resolution semantics.
The canonical build-plan dependency notation does preserve resolved artifact
classifier and extension when the JVM host exposes them, so non-main or non-JAR
artifacts are visible to Rust admission and classpath enrichment instead of being
collapsed into plain `group:name:version` coordinates. Declared graph artifacts
also split `classifier` and `extension` into explicit fields for drift checks.

`--resolved-dependency-graph-parity` emits Gradle public `ResolutionResult`
artifacts for reference and substrate invocations under
`<output-dir>/resolved-dependency-graphs/<project-name>/`:

- `upstream-resolved-graph.json`
- `substrate-resolved-graph.json`
- `resolved-graph-diff.json`

The resolved graph includes configurations, components, selected module
coordinates, transitive edges, selection reasons, variant attributes, and
artifact file names/IDs where Gradle exposes them. The diff participates in the
project match result and fails the run on drift. This is the right gate for
reading Gradle semantics and porting them into Rust one slice at a time. It
still does not prove direct Rust solver parity by itself: the substrate graph is
currently exported from Gradle's public `ResolutionResult` during the substrate
invocation, and repository source/checksum policy are not exposed by this API.

## Corpus Structure

The repository includes a small offline corpus at `testing/corpus/manifest.json`
covering Java library, Java application, Java multi-project, Java resource
expansion, JavaCompile options, standalone Copy/Sync resource transforms,
CopySpec duplicate handling, nested Copy/Sync CopySpec `into(...)` mappings,
Zip/Tar nested CopySpec mappings, Zip/Tar gzip+bzip2/War/Ear archive builds,
simple Java launcher Exec and JavaExec tasks, native Javadoc, and simple
Copy/Zip file-symlink inputs that Gradle follows as target bytes, plus a
static `eachFile` relative-path rewrite captured as explicit copy mappings and
a static `filter { line.replace(...) }` literal replacement, plus class-name
Test include/exclude filters. The
optional offline corpus also includes an OSS-style Java library slice with
sources JAR/Javadoc/static report outputs. The optional `testing/corpus/external-manifest.json`
adds pinned external-dependency builds for real non-empty JUnit Platform test
execution with include/exclude tag filtering and class-name Test filters, plus
a richer dependency constraints/exclusion sample and a classified artifact
coordinate (`sources@jar`), and requires network or a warm Gradle dependency
cache.
`testing/corpus/unsupported-manifest.json` tracks work that must not be
approximated natively until a complete contract exists. It covers custom JVM
task actions beyond static literal file writes, unsupported CopySpec actions
and filter shapes beyond the static relative-path rewrite and static literal
line replacement, and method-level Test filters.

Reference-mode runs compare upstream Gradle and Rust substrate exit codes, task
lists, and stable build output file inventories under `build/classes`,
`build/resources`, `build/libs`, `build/distributions`, `build/install`, and
`build/test-results`. Non-archive outputs are also compared by SHA-256 hash,
and archive outputs are compared by logical entry inventory. Archive
byte-for-byte parity and test-result content parity are tracked separately
because compression, metadata, timings, and binary result stores can differ
while logical artifacts are still equivalent.

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
      "output_hashes": {"build/classes/java/main/App.class": "..."},
      "substrate_noop": false,
      "duration_ms": 1234
    },
    "substrate": {
      "exit_code": 0,
      "tasks": [":compileJava", ":jar"],
      "output_file_count": 2,
      "output_files": ["build/classes/java/main/App.class", "build/libs/app.jar"],
      "output_hashes": {"build/classes/java/main/App.class": "..."},
      "substrate_noop": false,
      "duration_ms": 980
    },
    "checks": {
      "exit_code_match": true,
      "task_list_match": true,
      "output_files_match": true,
      "output_hashes_match": true,
      "successful": true,
      "no_fallback": true,
      "substrate_usable": true,
      "match": true
    },
    "match": true
  }
}
```

The runner also writes `corpus_summary.json` with aggregate project counts,
no-fallback counts, parity counts, task totals, elapsed wall-clock timings, and
failed/fallback project names. The demo script prints this summary after the
authoritative corpus gate so public runs can be judged without manually opening
the JSON.

## Adding Projects to Corpus

Copy or symlink Gradle projects into the corpus directory. The runner discovers
all projects with `build.gradle` or `build.gradle.kts` files.
