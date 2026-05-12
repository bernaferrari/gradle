# Rust Substrate Preview Contract

This document is the product contract for the Rust Substrate Preview tracked by
Beads epic `gradle-fork-ujt3`. The supported/fail-closed matrix is locked by
`gradle-fork-wx9j`. The preview is not a claim that Rust can run all Gradle
builds. It is a documented, measurable path where a supported build uses Gradle
for JVM-owned configuration semantics and then hands the admitted
post-configuration execution plan to Rust without task-by-task fallback.

## Product Shape

The preview should let a fresh evaluator run one checked-in command against
checked-in sample builds and observe these user-visible properties in the first
minute:

- the Rust wrapper locates or launches `gradle-substrate-daemon`;
- daemon socket readiness is measured;
- Gradle evaluates settings, DSL, `buildSrc`, and JVM plugins;
- the JVM bridge emits a typed build-plan contract;
- Rust admits or rejects the whole selected plan before task execution;
- admitted builds execute the selected DAG through Rust `RunBuild`;
- no JVM task forwards happen after successful kernel admission;
- dependency/repository IO, file-watch responsiveness, and cold/warm Rust DAG
  execution are reported as metrics.

The preview command line is:

```bash
tools/demo/rust_substrate_demo.sh
```

The direct first-60s metrics command is:

```bash
python3 tools/demo/first_60_seconds.py \
  --mode fast \
  --skip-build \
  --output build/first60-preview.json \
  --markdown-output build/first60-preview.md
```

The heavier proof command is:

```bash
python3 tools/demo/first_60_seconds.py \
  --mode proof \
  --skip-build \
  --output build/first60-preview-proof.json \
  --markdown-output build/first60-preview-proof.md
```

These commands assume this fork has already produced `build/gradle-under-test`
and `target/debug/gradle-substrate-daemon`, or that the demo command is allowed
to build them from the local checkout.

## Supported Build Shapes

The supported preview corpus is intentionally narrow and explicit:

- standard Java library and Java application builds;
- Java multi-project builds where selected tasks lower to known native task
  contracts;
- resource processing, `Copy`, `Sync`, `Jar`, `Zip`, `War`, `Ear`, `Tar`,
  `Exec`, `JavaExec`, `Javadoc`, and JUnit Platform `Test` shapes covered by
  checked-in corpus fixtures;
- exact single-output static `writeText` report tasks covered by the checked-in
  custom task fixture;
- exact detached-configuration resolved-file reports covered by the checked-in
  static fixture;
- exact lenient artifact-view resolved-file reports covered by the checked-in
  static fixture;
- exact artifact-transform marker reports covered by the checked-in static
  fixture;
- Maven-layout external dependencies with static selectors, representative BOM
  and dependency-constraint cases, selected Gradle Module Metadata cases, and
  artifact classifier/extension cases covered by the external corpus;
- local repository fixtures and safe HTTP Maven repository fixtures used by the
  first-60s and corpus runners.

Supported builds must pass the corpus runner with authoritative kernel mode,
declared graph parity where applicable, resolved graph parity where applicable,
output/hash/archive parity, and zero JVM task forwards.

## Supported Preview Matrix

This matrix is the public preview contract. A build shape is not supported just
because a unit test exists; it is supported only when the named evidence gate
proves no JVM task forwards plus the listed parity checks.

| Shape | Current preview status | Required evidence |
| --- | --- | --- |
| Offline Java library/application/multiproject lifecycle | Supported | `testing/corpus/manifest.json` authoritative RunBuild with task parity, output/hash/archive parity, and zero JVM forwards. |
| Offline resources, Copy, Sync, CopySpec mapping, archive, Exec, JavaExec, Javadoc, and JUnit Platform `Test` slices | Supported when represented by checked-in corpus fixtures | `testing/corpus/manifest.json` authoritative RunBuild with zero JVM forwards and fixture-specific output/hash/archive parity. |
| Exact static report-style tasks: single-output `writeText`, detached resolved-file report, lenient artifact-view report, artifact-transform marker report | Supported only for the exact captured static contract shapes | Checked-in corpus contract/admission plus authoritative RunBuild parity for the fixture; arbitrary task actions remain unsupported. |
| External Maven dependency builds with static selectors, representative BOM/platform, constraints, conflict resolution, classifier/artifact shape, JUnit runtime dependencies, and selected Gradle Module Metadata | Supported for `testing/corpus/external-manifest.json` fixtures | Authoritative RunBuild with declared and resolved dependency graph parity where requested, output/hash/archive parity, and zero JVM forwards. |
| Real-project dogfood: OSS-style Java library, Java multiproject, external JUnit library, external BOM/conflict | Supported strict dogfood entries | `testing/dogfood/manifest.json` execution with configured parity checks and zero JVM forwards. |
| Real OSS dogfood: Spring guide REST `clean assemble`, Spring PetClinic `clean compileJava` | Supported narrow slices only | `testing/dogfood/oss-manifest.json` execution with task/output/hash/archive parity and zero JVM forwards. |

No other Gradle build shape is part of the supported preview contract until it
is added to this table and backed by reproducible evidence.

## Unsupported Build Shapes

Unsupported semantics must fail closed at build-plan admission, dependency graph
admission, or an explicit unsupported corpus gate. The preview does not support:

- arbitrary task actions or unmodeled task implementations outside the exact
  static `writeText` report-task contract;
- arbitrary JVM plugin behavior after the typed task/dependency contract has
  been captured;
- arbitrary `buildSrc` execution in Rust;
- DSL evaluation in Rust;
- rich dependency semantics not represented by the native contract, including
  unsupported dynamic selectors, non-module dependency substitution, richer
  enforced-platform forms outside exact static `enforcedPlatform("g:n:v")`,
  component metadata rules outside exact `components { all { status = "release" } }`
  normalization, detached-configuration semantics outside the exact resolved-file
  report fixture, artifact-view semantics outside the exact lenient resolved-file
  report fixture, artifact-transform semantics outside the exact marker report
  fixture, repository auth/proxy/offline/refresh semantics outside the checked-in
  contract, explicit settings-level `includeBuild(...)` composite substitution,
  and unsupported Gradle Module Metadata fields;
- task-by-task fallback inside an admitted Rust-authoritative run.

Unsupported fixtures belong in `testing/corpus/unsupported-manifest.json` or in
focused JVM/Rust tests that assert a precise unsupported diagnostic.

## Fail-Closed Matrix

These cases are intentionally rejected before Rust execution or by an explicit
unsupported corpus gate. Rejection is success for the preview because hidden
fallback would make the Rust-kernel claim false.

| Unsupported shape | Required diagnostic behavior |
| --- | --- |
| Arbitrary custom JVM task actions and unmodeled task implementations | Reject at build-plan admission unless the task matches an exact static report contract. |
| Unsupported CopySpec behavior, unsupported copy filters, method-level or richer test filters, unsafe symlink/archive variants | Reject via unsupported corpus or focused task-graph/executor admission tests before native execution. |
| DSL evaluation, `buildSrc`, arbitrary JVM plugins, and reflection-heavy Gradle APIs in Rust | Stay JVM-owned before the typed contract boundary; never approximate inside the Rust daemon. |
| Rich dependency semantics outside the native contract: unsupported dynamic selectors, non-module dependency substitution, unsupported enforced-platform forms, unsupported component metadata rules, repository regex/configuration/attribute filters, unsupported Gradle Module Metadata fields | Reject at dependency graph or execution-kernel admission with a precise unsupported-feature reason. |
| Explicit settings-level `includeBuild(...)` composite substitution | Reject as `composite-substitution:settings` until Rust has an included-build IR/execution model. |
| CycloneDX SBOM tasks without a complete schema-backed SBOM contract | Reject native execution; graph/artifact/POM evidence may be captured but must not be promoted without output parity. |
| Any task-by-task JVM fallback after an admitted authoritative Rust run | Treat as a failed preview gate. |

## Ownership Boundaries

JVM-owned surfaces:

- settings and project DSL evaluation;
- Kotlin/Groovy script compilation and execution;
- `buildSrc` and arbitrary JVM plugin code;
- reflection-heavy Gradle APIs and compatibility behavior before the typed
  contract boundary;
- live Gradle model semantics that cannot be serialized into a stable Rust
  contract.

Rust-owned preview surfaces after admission:

- daemon readiness and sidecar identity;
- build-plan admission and unsupported diagnostics;
- dependency/repository read-through and supported graph construction;
- task DAG materialization from the captured plan;
- scheduling, work validation, up-to-date checks, cache/history decisions, and
  native execution for supported task contracts;
- Rust-owned process launch for supported `JavaCompile`, `Exec`, `JavaExec`,
  `Javadoc`, and `Test` contracts, reported as `rust_process` task execution
  details; JVM worker/compatibility paths remain explicit sidecars rather than
  hidden in-daemon fallback;
- first-60s metrics and no-fallback execution evidence.

## Warm Cached-Plan Prototype

The preview now has a narrow Rust-only warm execution prototype:
`gradle-substrate-runbuild`. It connects to an already-running
`gradle-substrate-daemon`, reads one validated build-plan shadow artifact, calls
`InitBuild` to register the build scope, then calls `RunBuild` directly without
starting Gradle or evaluating the DSL again.

Example:

```bash
cargo build -p gradle-substrate-daemon --bin gradle-substrate-runbuild

target/debug/gradle-substrate-runbuild \
  --endpoint tcp://127.0.0.1:58276 \
  --artifact build/dogfood-overhead-probe/shared-substrate-state/state/config-cache/build-plan-shadow/<artifact>.json \
  --project-dir "$PWD/testing/corpus/oss-style-java-library-kotlin-dsl" \
  --max-parallelism 4
```

This is not a public replacement CLI yet. It is a proof seam for the next
architecture step: after Gradle/JVM has produced and validated a supported
build-plan artifact once, Rust can execute that cached plan directly. Current
shadow artifacts may have an empty canonical `project_dir`, so the prototype
requires `--project-dir` for those artifacts. Promoting this to a user-facing
warm path requires stable build identity, invalidation/fingerprint checks
against source/settings/build files, selected-task compatibility checks, and a
safe way to locate the right artifact without asking the user for a path.

## Ship Gates

Gate 1, product contract and non-goals (`gradle-fork-wx9j`):
this document exists, is linked from `substrate/PARITY.md`, names supported and
unsupported shapes, lists commands, thresholds, and maps every open preview
child task to a gate.

Gate 2, one Rust kernel admission path:
`org.gradle.rust.substrate.execution.kernel=true` is the documented strict
kernel mode. Admission returns structured accepted/rejected results before
execution. After accepted admission, the run must report zero JVM forwards.
Unsupported semantics must reject before execution. The safe public preview
entry uses the same admission resolver in `NATIVE_READY_DEFAULT` mode: it tries
Rust only for admitted plans and delegates otherwise. Legacy RunBuild flags are
compatibility inputs, not separate admission paths.

Gate 3, first-60s performance budget (`gradle-fork-ilbp`):
`tools/demo/first_60_seconds.py` emits JSON and a human summary. Fast mode must
pass these budgets unless the threshold is deliberately updated with evidence:
daemon socket readiness at or below 2000 ms, file-watch first event at or below
250 ms, at least 3 first-run real-build remote requests avoided on the second
run, real-build dependency read-through at or below 60000 ms, and authoritative
Rust DAG execution at or below 60000 ms total with zero JVM forwards. Cold and
warm DAG durations are reported; cold must stay at or below 30000 ms, warm must
stay at or below 10000 ms, and warm must be faster than cold for the checked-in
Java-library sample. The warm Rust DAG run must report `build-plan-cache` as
its plan source so configuration-cache replay is visible at the kernel
admission boundary. Installed authoritative file watching must also complete
`help --watch-fs` against the local Gradle-under-test image at or below 30000
ms while observing Rust daemon startup/connection and active Gradle file-system
watching.

Gate 4, external dependency corpus parity:
`testing/corpus/external-manifest.json` covers representative Maven POM,
BOM/platform, Gradle Module Metadata, classifier/artifact-shape, repository
content/filter/auth-local cases as supported or unsupported fixtures. Supported
cases pass no-fallback declared and resolved graph parity.

The manifest-backed supported cases are Maven POM/transitive chains, version
conflict, dependency constraints, BOM/platform, classifier/artifact shape, JUnit
runtime dependencies, public Gradle Module Metadata, and checked-in local Gradle
Module Metadata with constraints/excludes. Repository content filters,
session-scoped credentials, and unsupported repository metadata rules are
validated by focused bridge/Rust resolver tests and recorded in `PARITY.md`
until they have stable standalone corpus projects.

Composite substitution remains intentionally fail-closed. The bridge now records
explicit `includeBuild(...)` settings as `composite-substitution:settings`, and
Rust kernel admission rejects that plan before scheduler dispatch. Promoting this
requires a real included-build IR/execution model, including substituted project
dependency edges, included-build tasks, outputs, and classpaths across build
scopes.

Gate 5, one-command demo and dogfood workflow (`gradle-fork-9dh1`):
`tools/demo/rust_substrate_demo.sh` builds or locates prerequisites, runs
supported sample builds through kernel mode, prints JVM-owned and Rust-owned
phases, reports no JVM forwards, points to artifacts, and explains unsupported
failures without hidden manual setup.

Gate 6, maintenance guardrails:
[`architecture/rust-substrate-maintenance.md`](../architecture/rust-substrate-maintenance.md)
documents schema versioning, proto regeneration, Rust/JVM contract ownership,
how to add supported semantic slices, how to add unsupported fail-closed gates,
how to update `PARITY.md`, and how to record evidence in Beads.

## Validation Commands

Fast preview metrics:

```bash
python3 tools/demo/first_60_seconds.py \
  --mode fast \
  --skip-build \
  --output build/first60-preview.json \
  --markdown-output build/first60-preview.md
```

Proof preview metrics:

```bash
python3 tools/demo/first_60_seconds.py \
  --mode proof \
  --skip-build \
  --output build/first60-preview-proof.json \
  --markdown-output build/first60-preview-proof.md
```

Offline supported corpus:

```bash
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --tasks clean build \
  --timeout 300 \
  --output-dir build/corpus-preview-offline
```

External dependency corpus:

```bash
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
```

Unsupported corpus contract checks:

```bash
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/unsupported-manifest.json \
  --contract-only \
  --output-dir build/corpus-preview-unsupported
```

Focused Rust and bridge contract tests:

```bash
cargo test -p gradle-substrate-daemon execution_kernel --lib
cargo test -p gradle-substrate-daemon kernel_dependency_graph --lib
./gradlew :rust-bridge:test --no-daemon --console=plain
```

## Evidence Rules

Passing tests are not enough by themselves. A preview gate is complete only when
the command output or checked-in artifact proves the exact gate requirements:
supported corpus cases must show no fallback, unsupported cases must show
fail-closed diagnostics, first-60s runs must emit metrics JSON and pass the
budget, and documentation must avoid claiming full Gradle parity.
