# Parity Status

Module: `substrate-daemon` (`substrate/`)  
Status: **native-kernel preview** (not full Gradle parity)

This file is a short parity snapshot. The product contract, supported/unsupported
matrix, ship gates, and evidence rules live in:

**[`docs/rust-substrate-preview.md`](../docs/rust-substrate-preview.md)**

Related:

- Strategy: [`docs/rust-substrate-turbopack-plan.md`](../docs/rust-substrate-turbopack-plan.md)
- Dogfood evidence: [`docs/rust-substrate-dogfood.md`](../docs/rust-substrate-dogfood.md)
- Warm path: [`WARM_PATH_ROADMAP.md`](WARM_PATH_ROADMAP.md)
- Roadmap: [`plan.md`](plan.md)
- Migration notes: [`MIGRATION.md`](MIGRATION.md)

## Modes

| Mode | Meaning |
| --- | --- |
| **Authoritative / kernel** | `-Dorg.gradle.rust.substrate.execution.kernel=true` (alias: `runbuild.authoritative`). Rust admits or rejects the whole selected plan, then runs `RunBuild` with **zero JVM task forwards**. Hidden fallback after admission is a failed gate. |
| **Shadow** | JVM remains source of truth; Rust runs in parallel or records comparisons via shadow reporters. Used while landing surfaces before promotion. |
| **Unsupported / fail-closed** | Shape is outside the preview contract. Admission or an unsupported corpus gate must reject with a precise diagnostic—never approximate. |

Public CLI aliases: wrapper `--rust-substrate-kernel` (legacy `--rust-substrate-authoritative`).  
Safe default tries Rust only for admitted native-ready plans and otherwise delegates.

## Supported preview matrix (pointer)

Authoritative detail is only in the preview doc. Summary:

| Shape | Status |
| --- | --- |
| Offline Java library / application / multiproject lifecycle | Supported (`testing/corpus/manifest.json`) |
| Resources, Copy, Sync, archives, Exec, JavaExec, Javadoc, JUnit Platform Test (fixture-backed) | Supported when represented by checked-in corpus fixtures |
| Exact static report tasks (`writeText`, detached resolved-file, lenient artifact-view, transform marker) | Supported only for exact captured contracts |
| External Maven / BOM / constraints / selected GMM | Supported for `testing/corpus/external-manifest.json` fixtures |
| Local dogfood + narrow OSS slices | Supported per `testing/dogfood/manifest.json` and `oss-manifest.json` |
| Everything else | **Not** preview-supported until listed and evidenced |

## Ownership boundaries

**JVM-owned (before the typed contract):** settings/project DSL, Kotlin/Groovy
script execution, `buildSrc`, arbitrary JVM plugins, reflection-heavy APIs,
live model semantics that cannot be serialized stably.

**Rust-owned after admission (preview):** daemon readiness; build-plan admission
and unsupported diagnostics; supported dependency/repository read-through;
task DAG materialization; scheduling; up-to-date/cache/history decisions;
native execution for admitted task contracts; process launch for supported
`JavaCompile`, `Exec`, `JavaExec`, `Javadoc`, and `Test`; first-60s / no-forward
evidence.

## Module status (high level)

Statuses are relative to the **preview kernel path**, not “100% of Gradle.”

### Authoritative on admitted preview paths

| Area | Notes |
| --- | --- |
| Execution kernel / `RunBuild` | Whole-plan admit or reject; zero-forward gate after accept |
| Build-plan shadow + warm `runbuild` | Cached plan + content fingerprints; warm runner prefers direct Rust |
| File watch / snapshots / hashing / file-hash cache | Strict mode: Rust watch is source of truth; fail-closed startup |
| Task executors (fixture-backed) | JavaCompile, ProcessResources/Copy/Sync, Jar/Zip/War/Ear/Tar, Test, Exec, JavaExec, Javadoc, CreateStartScripts, lifecycle/no-op, static WriteFile reports |
| Up-to-date + local cache pack/unpack | From captured work metadata on admitted plans |
| Hash compatibility | Cross-language digest suite |
| Daemon lifecycle / endpoint reuse | Loopback endpoint + binary identity; launch-mode metadata |

### Partial / progressing (shadow or bounded authoritative)

| Area | Notes |
| --- | --- |
| Dependency resolution / solver | External corpus fixtures + GMM fail-closed rules; not full Gradle resolution |
| Configuration IR / Phase 5 graph | Captured beside shadow; warm invalidation; native replay only for supported built-ins |
| Native plugin ABI (Phase 6) | Built-ins: base, java, java-library, application; external plugins fail closed |
| Remote cache / GC / integrity | Service surface + differential coverage; not universal product claim |
| Execution history / incremental / workers | Present and exercised on supported paths; expand only with evidence |
| Config cache IR / schema_versioned stores | Durable envelopes and IR parsers advancing; dual-path with JVM where needed |
| Build script parsing | String-based parser is production path (102 tests); Groovy AST still bypassed for no-paren issues |
| Observability | Build events, problem reporting, console buffering — useful, not full IDE parity |

### Unsupported (fail closed; non-goals for preview)

- Arbitrary task actions / unmodeled task implementations
- DSL evaluation or `buildSrc` execution inside Rust
- Settings-level `includeBuild(...)` composite substitution
- Rich dependency semantics outside the native contract (dynamic selectors,
  custom metadata rules, unsupported GMM fields, etc.)
- Task-by-task JVM fallback after an admitted authoritative run
- Full Tooling API / arbitrary plugin ecosystem in Rust

See preview **Unsupported** and **Fail-Closed** tables for diagnostics.

## Known gaps

- **Not full Gradle.** Preview is a measured supported slice plus fail-closed
  rejection elsewhere. Do not claim universal parity.
- **JavaExec `main_class` on direct warm replay:** closed for the
  `javaexec-process-launch` dogfood fixture by merging direct build-graph
  metadata into the hydrated shadow task context before admission (preserves
  `main_class`, classpath, Java home, args, working directory). Re-open only if
  a new shape loses fields on replay—add a fixture, do not weaken admission.
- **Composite substitution:** still intentionally rejected
  (`composite-substitution:settings`) until an included-build IR exists.
- **Configuration / plugins:** external and custom plugins stay JVM-owned or
  fail closed; Phase 5/6 expand only built-in native contracts.
- **Warm path polish:** endpoint freshness, dry-run validation, richer stale
  diagnostics—see `WARM_PATH_ROADMAP.md`.
- **Package-wide noise:** some differential/benchmark targets may still fail to
  compile independently of the preview kernel path; prefer focused lib/tests and
  corpus/dogfood gates for ship evidence.
- **Symlink tests:** a few are `#[ignore]` under sandboxed macOS `/var` ELOOP
  behavior; not a runtime claim for non-sandboxed hosts.

## How to validate

Prereqs: `build/gradle-under-test` and `target/debug/gradle-substrate-daemon`
(and `gradle-substrate-runbuild` for warm paths), or allow demo scripts to build.

### Rust unit / focused

```bash
cargo test -p gradle-substrate-daemon --lib
cargo test -p gradle-substrate-daemon execution_kernel --lib
cargo test -p gradle-substrate-daemon kernel_dependency_graph --lib
cargo test -p gradle-substrate-daemon --test hash_compatibility_test
cargo test -p gradle-substrate-daemon --test build_plan_ir_golden_test
cargo test -p gradle-substrate-daemon --test build_plan_shadow_test \
  refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact
```

### Bridge

```bash
./gradlew :rust-bridge:test --no-daemon --console=plain
./gradlew :core:test --tests org.gradle.execution.RustAuthoritativeBuildExecutionActionTest \
  -x :distributions-core:generateLicenseFile
```

### Corpus (authoritative)

```bash
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --tasks clean build \
  --timeout 300 \
  --output-dir build/corpus-preview-offline

python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/external-manifest.json \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --dependency-graph-parity \
  --resolved-dependency-graph-parity \
  --tasks clean build \
  --timeout 300 \
  --output-dir build/corpus-preview-external

python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/unsupported-manifest.json \
  --contract-only \
  --output-dir build/corpus-preview-unsupported
```

### Dogfood + warm

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/manifest.json \
  --execute \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood-current \
  --verbose

python3 tools/dogfood_runner/direct_warm.py \
  --manifest testing/dogfood/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-binary target/debug/gradle-substrate-runbuild \
  --output-dir build/direct-warm-dogfood-current

python3 tools/warm_runner/run.py \
  --project-dir "$PWD/testing/corpus/oss-style-java-library-kotlin-dsl" \
  --task :build \
  --state-dir build/warm-rust-state \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-binary target/debug/gradle-substrate-runbuild \
  --output-json build/warm-rust-state/result.json
```

### Demo / first-60s / stabilization

```bash
tools/demo/rust_substrate_demo.sh --quick
python3 tools/demo/first_60_seconds.py --mode fast --skip-build \
  --output build/first60-preview.json --markdown-output build/first60-preview.md
./tools/stabilization/run_strict_stabilization.sh quick
```

## Evidence rules

1. A shape is preview-supported only if it is in the preview matrix **and** a
   reproducible command shows zero JVM forwards plus required parity.
2. Unit tests alone do not promote a shape.
3. Unsupported cases must show fail-closed diagnostics, not silent success.
4. Record commands and artifact paths when closing work; update this file only
   for durable status changes—not per-agent session logs.
