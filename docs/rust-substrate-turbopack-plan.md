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

### Phase 3: Make Warm Builds Rust First

Goal: one capture, many direct Rust executions.

- Treat build-plan shadow artifacts as durable Rust graph snapshots.
- Make direct warm runbuild the default for supported repeated invocations.
- Validate build definition inputs before execution.
- Use file-watch deltas and content fingerprints for targeted invalidation.
- Record exactly why a plan is stale or unsupported.

This is the strongest Turbopack-style wedge: skip JVM configuration when the
stable Rust graph is still valid.

### Phase 4: Move Dependency Resolution Into Rust

Goal: Rust owns dependency graph solving for common repositories.

- Maven/Ivy transport, metadata cache, checksums, redirects, mirrors, auth, and
  offline behavior.
- POM and Gradle module metadata parsing.
- Variant and capability selection for the most common JVM ecosystem shapes.
- BOM/platform constraints, exclusions, dynamic versions, and conflict handling.
- Fail closed for component metadata rules, custom artifact transforms, and
  plugin-specific resolution hooks until modeled.

### Phase 5: Move Configuration To A Stable IR

Goal: JVM configuration is a capture source, not the warm engine.

- Capture settings, projects, source sets, tasks, extensions, and common plugin
  models into a versioned Rust IR.
- Cache the IR with precise invalidation for build scripts, settings files,
  catalogs, init scripts, environment, and plugin classpath.
- Execute supported configuration directly from Rust for common builds.
- Keep JVM configuration as fallback and differential oracle.

### Phase 6: Native Plugin ABI

Goal: new plugin work does not require JVM.

- Define a Rust plugin ABI for model contribution, task registration, dependency
  requests, diagnostics, and task execution.
- Consider Wasm for sandboxed third-party plugins.
- Provide compatibility shims for common Gradle plugin patterns.
- Keep JVM plugins as a guest runtime with explicit capability boundaries.

### Phase 7: Shrink The JVM Shell

Goal: JVM is optional compatibility, not the engine.

- CLI can start Rust first.
- Rust daemon owns graph, cache, scheduler, diagnostics, and build lifecycle.
- JVM starts only when a build, plugin, Tooling API path, or DSL feature needs
  the compatibility runtime.
- Native plugin builds never need the JVM shell.

## Aggressive Near-Term Backlog

1. Keep the just-fixed `rust-bridge:testClasses` gate green in CI.
2. Make Copy/Sync/Delete/Symlink parity authoritative for supported file specs.
3. Promote the direct warm runbuild path from demo to primary supported workflow.
4. Finish build-cache packaging proto and Rust pack/unpack parity.
5. Wire execution history and up-to-date checks to VFS deltas, not only mtimes.
6. Expand dogfood zero-forward support for Java-library and Spring-style builds.
7. Add Rust dependency-resolution graph parity for file and static Maven repos.
8. Create a canonical Rust `BuildGraph` IR and make JVM capture write it.
9. Add a strict unsupported-feature registry with counts in every dogfood run.
10. Delete or quarantine noisy generated Rust/docs that obscure real ownership.

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
