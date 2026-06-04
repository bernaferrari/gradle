# Rust Substrate Turbopack-Style Plan

Status: aggressive migration plan
Last updated: 2026-06-01

## Reference Model

The useful Turbopack lesson is not "rewrite every class in Rust." The useful
lesson is to build a new Rust engine around a persistent graph, make work lazy
and incremental, and keep compatibility layers only where the ecosystem needs
them.

For Gradle, that means moving from "Gradle with Rust helper services" to "a Rust
build engine with a Gradle compatibility frontend."

Relevant Turbopack ideas:

- A Rust-native core owns the hot path, not only leaf utilities.
- One persistent graph represents units of work and their dependencies.
- Work is demand driven: calculate only the requested task closure.
- Invalidations are fine grained and content based.
- Compatibility is a migration tool, not the long-term architecture.

Official references:

- https://nextjs.org/docs/architecture/turbopack
- https://vercel.com/blog/turbopack

## What 100% Rust Can Realistically Mean

The realistic target is 100% Rust for Gradle's build engine:

- file system model, VFS, watching, snapshots, hashing, and file operations
- dependency metadata transport, cache, graph solving, and artifact stores
- task graph planning, scheduling, execution, history, and build cache
- common native task families: Java, resources, copy/sync/delete, archives,
  test execution, exec/javaexec, javadoc, publishing, and start scripts
- configuration-cache-like model persistence and direct warm execution
- diagnostics, event stream, problem reporting, and observability

It cannot honestly mean that arbitrary existing JVM plugins become Rust without
a migration boundary. The Gradle plugin ecosystem is part of Gradle's product
surface. A realistic "100% Rust" architecture keeps a JVM compatibility island
for existing plugins and DSL execution until those plugins can move to a Rust or
Wasm plugin ABI.

The end state should be:

- Rust is authoritative for the engine.
- JVM code is a guest compatibility runtime.
- Native Rust plugins are first class.
- Unsupported JVM-only behavior is isolated, measured, and eventually optional.

## Current State

Good:

- The substrate daemon, proto bridge, and feature flags exist.
- The bridge now compiles, test classes compile, and focused bridge tests run.
- Several core file-operation and execution-path slices already execute in Rust.
- Direct warm build-plan execution exists as the highest-value proof: one JVM
  capture can feed repeated zero-forward Rust executions for supported builds.
- Fail-closed boundaries are present for unsupported semantics, which is better
  than silent approximation.

Bad:

- The JVM still owns most configuration, DSL evaluation, arbitrary plugin code,
  and large parts of the Gradle API surface.
- Many Rust services are still shadow, partial, or scaffolding rather than
  authoritative engine ownership.
- Some generated or noisy Rust files make review harder and should be cleaned as
  part of hardening.
- The migration has breadth but not enough authoritative vertical slices.
- "Service count" is not the metric that matters. Native no-forward builds are.

How much is left:

- For the Rust substrate itself, the foundation is real.
- For full Gradle compatibility, most of the hard work remains: configuration
  semantics, plugin execution, dependency resolution edge cases, workers,
  artifact transforms, publishing, build cache, tooling API behavior, and the
  plugin ecosystem.
- The aggressive path is to stop spreading across services and instead finish
  authoritative verticals that produce useful no-JVM-forward builds.

## Architecture Target

### 1. Rust Incremental Graph

Create one canonical Rust graph that can represent:

- build settings and project model nodes
- repositories, dependency requests, variants, artifacts, and metadata
- task registrations, inputs, outputs, destroyables, local state, and services
- VFS snapshots, file-watch deltas, and content fingerprints
- execution history, build cache entries, and diagnostics

Every node should be:

- deterministic
- content addressable where possible
- cheap to invalidate
- serializable across daemon restarts
- comparable against JVM capture during migration

### 2. Gradle Compatibility Frontend

The current JVM bridge should become a frontend that captures a stable IR:

- evaluated settings and project model
- selected task graph
- task contracts and plugin-origin metadata
- dependency-resolution requests and observed results
- unsafe or unsupported semantics as explicit markers

The frontend can stay JVM-backed while plugin compatibility matters. It should
not be the warm path once a valid Rust graph is available.

### 3. Rust Scheduler And Execution Kernel

The Rust scheduler should be the default executor for admitted plans:

- selected task closure only
- parallel execution with declared dependency ordering
- VFS-delta invalidation before execution
- execution history and up-to-date checks in Rust
- local and remote build-cache read/write in Rust
- no JVM task forwarding in strict authoritative mode

### 4. Plugin Runtime Split

Native plugins should target a Rust ABI. Legacy JVM plugins run in a guest
runtime that can either:

- declare a stable task/model contract that Rust can execute, or
- remain isolated and mark the build as requiring JVM compatibility.

This is the difference between a Rust engine and a class-by-class rewrite.

## Migration Strategy

### Phase 0: Keep The Gates Green

Goal: make the bridge and substrate safe to change quickly.

- Keep `:rust-bridge:compileJava`, `:rust-bridge:testClasses`, and focused
  bridge tests passing.
- Keep `cargo check` and focused Rust executor tests passing.
- Remove or quarantine generated-noise code when it blocks review.
- Keep shadow and authoritative feature flags explicit.

### Phase 1: Finish Native File System Ownership

Goal: Rust owns file state and file mutations.

- Make VFS, file watching, snapshots, hashing, file-hash cache, and file tree
  traversal authoritative.
- Finish Copy, Sync, Delete, Symlink, Mkdir, WriteFile, and archive parity.
- Add platform-specific tests for symlinks, permissions, case sensitivity, and
  delete retry semantics.
- Route Gradle file operations through Rust by default in authoritative mode.

Completion checkpoint, 2026-06-01:

- Rust file watching is authoritative in strict mode: Rust watch startup and
  polling fail closed, Java watcher events are suppressed as a source of truth,
  and Rust change events invalidate Gradle's VFS lifecycle.
- File snapshots, hashing, file-hash cache, file tree traversal, directory
  symlink traversal, and cycle detection have authoritative bridge coverage for
  the admitted Rust path.
- Copy, Sync, Delete, Symlink, Mkdir, WriteFile, Jar, Zip, War, Ear, and Tar
  have native Rust executors for admitted contracts, including permissions,
  symlink behavior, delete retry behavior, and archive traversal parity.
- `mode=authoritative`, strict execution-kernel flags, and the legacy
  authoritative run-build flag now invoke Rust `RunBuild` with JVM task
  forwarding disabled for captured/native-ready plans.
- Boundary: arbitrary JVM plugin action bodies that call `Project.copy`,
  `FileSystemOperations`, `File.delete`, or other ad hoc Java file APIs remain
  inside the JVM compatibility island until the configuration/plugin-runtime
  phases capture those actions as stable Rust contracts. They are not part of
  the completed Phase 1 authoritative kernel surface.

### Phase 2: Finish Native Execution Verticals

Goal: common Java builds can run with zero JVM task forwards.

- JavaCompile with classpath, annotation processor, incremental input, and
  compiler argument parity.
- ProcessResources, Copy, Sync, Jar, Zip, Tar, Test, Exec, JavaExec, Javadoc,
  CreateStartScripts, and lifecycle/no-source/no-op tasks.
- Execution history, up-to-date checks, and build-cache pack/unpack in Rust.
- Dogfood gates must measure task parity, output hashes, archive entries, and
  zero JVM forwards.

Completion checkpoint, 2026-06-01:

- Completed for the supported Java preview slice, not for arbitrary Gradle
  builds. JVM configuration, `buildSrc`, arbitrary plugin logic, Kotlin
  compilation, and unmodeled task implementations remain outside this phase.
- Rust now owns native execution for the admitted common Java verticals:
  JavaCompile, ProcessResources/Copy/Sync, Jar/Zip/War/Ear/Tar, Test, Exec,
  JavaExec, Javadoc, CreateStartScripts, lifecycle, no-source, and static
  report-style WriteFile contracts.
- Rust execution planning now performs up-to-date/cache decisions from captured
  work metadata, admits Gradle `@CacheableTask` markers, avoids synthetic cache
  hits in authoritative mode, restores declared outputs from the Rust local
  cache, and stores successful native outputs using the Rust pack/unpack layout.
- The Phase 2 completion dogfood run used the rebuilt daemon and passed
  `build/dogfood-phase2-20260601/dogfood-summary.md`: 7/7 projects matched,
  6/6 supported projects had zero JVM forwards, task graph captures and Rust
  RunBuild markers were 7/7, and the unsupported composite-substitution fixture
  failed closed.

### Phase 3: Make Warm Builds Rust First

Goal: one capture, many direct Rust executions.

- Treat build-plan shadow artifacts as durable Rust graph snapshots.
- Make direct warm runbuild the default for supported repeated invocations.
- Validate build definition inputs before execution.
- Use file-watch deltas and content fingerprints for targeted invalidation.
- Record exactly why a plan is stale or unsupported.

This is the strongest Turbopack-style wedge: skip JVM configuration when the
stable Rust graph is still valid.

Completion checkpoint, 2026-06-01:

- Completed for the supported Java preview slice. The warm path is still a
  supported-slice workflow, not a universal Gradle CLI replacement.
- `tools/warm_runner/run.py` now makes Rust-first warm execution the default
  workflow: it tries direct `gradle-substrate-runbuild` first, captures through
  Gradle/JVM only on `cache-miss`, `stale`, `unsafe-cache`, `task-mismatch`, or
  `incomplete-cache`, and keeps unsupported execution as a fail-closed result.
- `gradle-substrate-runbuild` now accepts trusted file-watch deltas via
  `--changed-path` and `--changed-paths-file`, so direct warm validation can
  rehash affected project inputs while still validating external
  cache/dependency inputs fully. Without trusted deltas it keeps full
  conservative validation.
- The warm runner writes structured reasons and evidence JSON for direct hits,
  recaptures, capture failures, and unsupported direct execution.
- Evidence: `build/warm-runner-phase3-20260601/result.json` proved
  `cache-miss -> capture -> direct Rust RunBuild`; `result-warm-hit.json`
  proved the next invocation was `WARM_HIT` with no capture, 14 tasks,
  8 up-to-date tasks, 4 skipped tasks, zero JVM forwards, and
  `build-plan-shadow`.
- Dogfood evidence:
  `build/direct-warm-phase3-20260601/direct-warm-summary.md` passed 6/6
  supported projects with zero JVM forwards and 4575.9 ms total direct warm
  wall time across the supported local dogfood set.

### Phase 4: Move Dependency Resolution Into Rust

Goal: Rust owns dependency graph solving for common repositories.

- Maven/Ivy transport, metadata cache, checksums, redirects, mirrors, auth, and
  offline behavior.
- POM and Gradle module metadata parsing.
- Variant and capability selection for the most common JVM ecosystem shapes.
- BOM/platform constraints, exclusions, dynamic versions, and conflict handling.
- Fail closed for component metadata rules, custom artifact transforms, and
  plugin-specific resolution hooks until modeled.

Progress checkpoint, 2026-06-01:

- Gradle Module Metadata JVM artifact selection now rejects ambiguous artifact
  shapes instead of taking the first published file. Supported JVM variants may
  publish no files and fall back to Maven-layout artifact URLs, or publish one
  primary jar after ignoring documentation jars. Multiple primary jars,
  non-JVM files, and malformed artifact entries fail closed with explicit
  diagnostics.
- Gradle Module Metadata dependency declarations now fail closed for
  dependency attributes, requested capabilities, strict-version endorsement, and
  unknown non-null extension fields. Advisory `reason` metadata remains allowed.
- Evidence: library-only Rust tests passed for the full
  `dependency_solver::gradle_module_metadata` suite and the broader
  `gradle_module_metadata` resolver filter: 25/25 focused metadata tests and
  32/32 metadata resolver tests. The package-wide integration test target still
  has unrelated pre-existing compile failures under
  `substrate/tests/differential` and `substrate/tests/benchmarks.rs`.
- External corpus evidence:
  `build/corpus-preview-external-phase4-20260601/corpus_summary.json` passed
  9/9 projects with authoritative RunBuild, no fallback substrate runs, exit
  code/task/output/hash/archive parity, and equal task totals
  (upstream=105, substrate=105). All 18 declared and resolved dependency graph
  diffs matched.

### Phase 5: Move Configuration To A Stable IR

Goal: JVM configuration is a capture source, not the warm engine.

- Capture settings, projects, source sets, tasks, extensions, and common plugin
  models into a versioned Rust IR.
- Cache the IR with precise invalidation for build scripts, settings files,
  catalogs, init scripts, environment, and plugin classpath.
- Execute supported configuration directly from Rust for common builds.
- Keep JVM configuration as fallback and differential oracle.

Progress checkpoint, 2026-06-02:

- Added a Rust-owned `CanonicalConfigurationGraph` schema for Phase 5. The
  graph captures settings, projects, default JVM source sets, task
  configuration summaries, plugin applications, dependency configurations,
  toolchains, version-catalog inputs, and script invalidation inputs with a
  deterministic fingerprint.
- JVM shadow capture now parses the inferred root settings script in addition
  to project build scripts and persists the configuration graph beside each
  build-plan shadow artifact. New artifacts validate and quarantine corrupt
  configuration-graph schema/build-id/fingerprint mismatches; old artifacts
  without a graph remain loadable.
- Warm shadow-plan reuse now validates captured configuration inputs before
  replay. If a settings script, project build script, or version catalog
  recorded in the graph changes, appears, disappears, or loses its stored
  content fingerprint, Rust quarantines the artifact instead of running a stale
  configuration graph.
- Configuration invalidation now also records Gradle user-home init script
  sentinels: `init.gradle`, `init.gradle.kts`, and the `init.d` directory.
  Directory inputs use deterministic content fingerprints, so adding an init
  script after capture invalidates warm replay.
- Configuration graphs now carry JVM environment identity and plugin classpath
  declarations captured from `buildscript { classpath(...) }` and versioned
  `plugins {}` entries. Warm replay validates runtime/environment-variable
  inputs without starting the JVM. Captured plugin classpath entries fail closed
  into Phase 6 until the native plugin ABI can execute those plugins.
- Added a native configuration replay admission gate over the graph. Supported
  Java/base/application plugin configuration can replay from Rust; applied
  external/custom plugins, unsupported repository shapes, parser warnings, and
  missing catalog/input fingerprints mark the hydrated shadow plan with
  `unsupported_configuration_semantics`, which the Rust execution kernel rejects
  before dispatch.
- Evidence: `cargo test -p gradle-substrate-daemon --lib
  configuration_ir::tests` passed 7/7, `cargo test -p
  gradle-substrate-daemon --lib build_plan_shadow::tests` passed 14/14, and
  `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test` passed
  7/7 with an integration assertion that the persisted shadow artifact contains
  the Phase 5 configuration graph. Additional replay-admission checks passed:
  `cargo test -p gradle-substrate-daemon --lib execution_kernel::tests` 26/26,
  `cargo test -p gradle-substrate-daemon --lib
  build_script_parser::tests::test_parse` 18/18, plus focused task-graph shadow
  hydration tests for unsupported configuration markers and build-plan-only
  compatibility.

### Phase 6: Native Plugin ABI

Goal: new plugin work does not require JVM.

- Define a Rust plugin ABI for model contribution, task registration, dependency
  requests, diagnostics, and task execution.
- Consider Wasm for sandboxed third-party plugins.
- Provide compatibility shims for common Gradle plugin patterns.
- Keep JVM plugins as a guest runtime with explicit capability boundaries.

Progress checkpoint, 2026-06-02:

- Added a versioned `NativePluginContract` ABI with model contributions, task
  registrations, dependency requests, diagnostics, and execution handler
  declarations. Built-in Rust contracts now exist for `base`, `java`,
  `java-library`, and `application`.
- Phase 6 now resolves native plugin contracts from the Phase 5 configuration
  graph. Supported built-in plugins produce contracts; custom/external applied
  plugins and captured plugin classpath dependencies produce explicit
  rejections until native ABI support or a JVM guest runtime is available.
- Native plugin contracts now materialize directly into the Rust task graph
  when a persisted build-plan shadow has configuration data but no captured JVM
  task plan. Supported built-in plugin tasks are hydrated from ABI task
  registrations; unsupported ABI/plugin-classpath semantics still mark the task
  context with `unsupported_configuration_semantics` so the execution kernel
  fails closed before dispatch.
- Evidence: `cargo test -p gradle-substrate-daemon --lib
  plugin_abi::tests` passed 4/4. Focused task-graph checks passed for native
  ABI task materialization, unsupported configuration replay markers, and
  build-plan shadow replacement:
  `cargo test -p gradle-substrate-daemon --lib
  task_graph::tests::test_build_plan_shadow_materializes_tasks_from_native_plugin_abi_when_plan_is_empty`,
  `cargo test -p gradle-substrate-daemon --lib
  task_graph::tests::test_build_plan_shadow_marks_unsupported_configuration_replay`,
  and `cargo test -p gradle-substrate-daemon --lib
  task_graph::tests::test_prefer_build_plan_shadow_replaces_registered_tasks`.

### Phase 7: Shrink The JVM Shell

Goal: JVM is optional compatibility, not the engine.

- CLI can start Rust first.
- Rust daemon owns graph, cache, scheduler, diagnostics, and build lifecycle.
- JVM starts only when a build, plugin, Tooling API path, or DSL feature needs
  the compatibility runtime.
- Native plugin builds never need the JVM shell.

Progress checkpoint, 2026-06-02:

- Fresh JVM bridge launches now start the Rust daemon process first and attach
  the JVM compatibility host only after the Rust process has launched, and only
  when the build-session options request that backchannel. If the optional JVM
  host fails to start, the just-launched Rust daemon is shut down instead of
  leaving an orphaned sidecar.
- Persisted daemon endpoint files now declare launch-mode metadata:
  JVM-bridge-launched daemons write `launchMode=rust-daemon-primary` with a
  `jvmHostMode` of `attached-on-demand` or `standalone`, while Rust-wrapper
  prewarmed daemons write `launchMode=rust-wrapper-prewarm` and
  `jvmHostMode=standalone`.
- The JVM bridge now parses that launch metadata before reusing persisted TCP
  endpoints. Legacy endpoint files remain accepted for compatibility, but
  inconsistent metadata is rejected and forces a fresh Rust daemon launch.
- The Rust wrapper now has an opt-in `--rust-substrate-direct` path, also
  available through `GRADLEW_RUST_DIRECT_RUNBUILD=true`, that prewarms the Rust
  daemon, tries cached `gradle-substrate-runbuild` before resolving or launching
  the Gradle JVM distribution, and delegates to the compatibility frontend when
  the direct warm plan is absent, stale, unsafe, or requested with unsupported
  Gradle CLI options.
- `gradle-substrate-runbuild` now separates exit codes for wrapper fallback:
  `0` means direct Rust execution completed, `1` means the direct Rust build ran
  and failed, and `2` means direct execution was unavailable so the wrapper may
  delegate to Gradle.
- Evidence: `cargo test -p gradle-wrapper -- --test-threads=1` passed 29/29,
  and `./gradlew :rust-bridge:test --tests
  org.gradle.internal.rustbridge.SubstrateLifecycleTest --no-daemon
  --console=plain` passed, including endpoint metadata admission checks.
  `cargo build -p gradle-wrapper -p gradle-substrate-daemon --bin
  gradle-substrate-runbuild` also passed.

### Phase 8: Stable BuildGraph Identity

Goal: make durable Rust graph snapshots addressable by a stable build identity
instead of a per-session Gradle build UUID or project-directory scan.

- The live build session id remains unchanged so active services can keep using
  their existing per-build state keys.
- JVM task-graph capture now writes both the existing session-keyed inline
  build-plan shadow and a stable root-keyed shadow artifact with
  `stableBuildIdentity`/`sessionBuildId` metadata.
- The Rust wrapper computes the same canonical-root identity and passes it to
  `gradle-substrate-runbuild` as `--build-id`.
- `gradle-substrate-runbuild` resolves `--state-dir + --build-id` through the
  shadow store's keyed artifact filename and validates the embedded artifact
  identity before executing, avoiding direct warm project-dir scanning for this
  path.
- A canonical Rust `BuildGraph` IR now derives project nodes, task nodes,
  ordering edges, and dependency requests from the canonical build-plan IR.
  Build-plan shadow artifacts persist the graph plus a graph fingerprint, and
  load-time validation quarantines schema, build-id, and fingerprint
  mismatches while preserving compatibility with older graphless artifacts.
- Direct `gradle-substrate-runbuild` now consumes the persisted `BuildGraph`
  for task existence, dependency-closure expansion, and graph fingerprint
  validation, with an explicit `BuildPlan` fallback only for older graphless
  artifacts.
- JVM-host model capture now also writes a stable root-keyed artifact with
  `stableBuildIdentity` and `sessionBuildId`, so stable direct-warm lookup is
  available beyond the selected task-graph listener path.
- Evidence: `cargo test -p gradle-substrate-daemon --bin
  gradle-substrate-runbuild` passed 14/14, and `cargo test -p
  gradle-substrate-daemon --test build_plan_shadow_test
  capture_and_persist_shadow_build_plan_artifact` passed with assertions for
  both session-keyed and stable root-keyed artifacts.

### Phase 9: VFS-Backed Execution History

Goal: let direct Rust execution use trusted file-watch/VFS deltas to preserve
up-to-date correctness without falling back to broad mtime-only validation.

- `gradle-substrate-runbuild` now forwards trusted changed paths from
  `--changed-path` / `--changed-paths-file` as per-task
  `trusted_vfs_delta` context.
- `DagExecutor` merges those direct-run contexts into the hydrated shadow task
  context instead of replacing work metadata.
- Trusted deltas that intersect a task input fingerprint add a rebuild reason
  and prevent an otherwise up-to-date skip; unrelated deltas leave the work
  fingerprint untouched so history can still produce `UP_TO_DATE`.
- Daemon-managed file-watch events now retain a shared VFS delta store and feed
  `DagExecutor` admission when the caller has not supplied an explicit trusted
  delta. This makes live daemon watch state participate in the same fail-closed
  up-to-date decision path as direct `runbuild` changed paths.
- `ExecutionPlan` records a per-work VFS delta watermark in the persistent
  execution record. Replayed daemon events are ignored after the task has
  admitted them once, including across daemon restart via execution-history
  reload.
- Native `RunBuild` cache restore/store now participates in the configured
  remote build cache: local misses can restore from remote and promote locally,
  and successful native output packs are pushed to remote after local storage.
- Direct `gradle-substrate-runbuild` now forwards persisted `BuildGraph` task
  metadata into selected task contexts even without VFS changes. `DagExecutor`
  admits that metadata fail-closed by rejecting mismatched graph task paths
  before dispatch.
- Corpus and dogfood summaries now include a strict unsupported-feature
  registry: each project records detected unsupported features and
  `corpus_summary.json` aggregates per-feature counts and affected project
  counts.
- Evidence: `cargo test -p gradle-substrate-daemon --bin
  gradle-substrate-runbuild` passed 15/15, and `cargo test -p
  gradle-substrate-daemon --lib vfs_delta` passed 10 focused tests, including
  `RunBuild` assertions that input-intersecting VFS deltas force execution and
  persisted watermarks prevent already-admitted daemon deltas from replaying.
  Remote cache evidence: `cargo test -p gradle-substrate-daemon --lib
  remote_cache` passed 20/20 including native `RunBuild` remote restore,
  local-promotion, and remote-store coverage.
  BuildGraph context evidence: `cargo test -p gradle-substrate-daemon --bin
  gradle-substrate-runbuild` passed 16/16, and focused `DagExecutor`
  admission coverage rejects mismatched BuildGraph task metadata.
  Unsupported-registry evidence: `python3 -m unittest
  tools.corpus_runner.test_run` passed 45/45 with aggregate feature-count
  coverage.
- Declared dependency graph parity now includes statically declared repositories
  that the Rust dependency engine can model: `mavenCentral()`, `mavenLocal()`,
  absolute/relative/file Maven repos, and static HTTP Maven repos. Repository
  drift is a hard declared-graph mismatch, so file/static Maven repository
  parity can be measured before full Gradle solver ownership.
  Repository-graph evidence: `python3 -m unittest
  tools.corpus_runner.test_run` passed 48/48 with static Maven repository and
  repository-drift coverage.
- File-operation authority was hardened for followed directory symlink cycles:
  the Rust `Delete` executor now tracks canonical directories while descending
  and fails closed instead of recursing through a cycle. This aligns Delete with
  the existing Copy cycle boundary for supported file specs.
  File-operation evidence: `cargo test -p gradle-substrate-daemon --lib
  task_executor::delete` passed 9/9 focused tests.
- Dogfood zero-forward coverage is now explicit by build shape. Dogfood
  manifests carry `coverage_tags`, summaries report supported and zero-forward
  counts by tag, the checked-in local manifest now includes a strict supported
  Java application/start-scripts/distribution fixture, and the pinned OSS
  manifest tags Spring-style supported and fail-closed slices.
  Dogfood coverage evidence: `python3 -m unittest
  tools.dogfood_runner.test_run` passed 18/18, both local and OSS dogfood
  manifests validate, and a local execution attempt under
  `build/dogfood-phase-backlog-20260604` correctly failed all entries with
  `substrate-inactive: run-build marker missing` because the selected Gradle
  command did not include active rust-bridge services.
- Review-noise cleanup removed tracked Rust `.bak` source duplicates under
  `substrate/src/server` and `substrate/src/server/task_executor`. These files
  were not referenced and duplicated live modules, so deleting them reduces
  false ownership surface without changing compiled code.
  Cleanup evidence: `rg` found no references to the tracked `.bak` files, and
  this is source-tree cleanup only.
- Bridge gate evidence: `./gradlew -q :rust-bridge:testClasses` passed on
  2026-06-04.

## Aggressive Near-Term Backlog

No discrete items remain from the 2026-06-04 aggressive backlog checkpoint.
The next useful work is a new phase checkpoint, not carrying these completed
items forward as stale todos.

## Metrics

Use these instead of "percent ported":

- zero-forward supported dogfood builds
- direct warm build latency
- first task start latency after a file change
- shadow mismatch count
- authoritative service count with parity tests
- unsupported-feature frequency by real corpus project
- JVM startup/configuration avoided on warm paths
- output hash and archive-entry parity

## Decision Rules

- Do not port Gradle class-by-class.
- Do not count a service as migrated until Rust is authoritative for a real
  supported workflow.
- Prefer vertical slices that eliminate JVM work in an end-to-end build.
- Preserve JVM plugin compatibility, but keep it behind a measured boundary.
- Fail closed rather than approximate Gradle semantics.
- Every expansion needs a focused unit test, a bridge test where applicable, and
  a dogfood or corpus evidence path.
