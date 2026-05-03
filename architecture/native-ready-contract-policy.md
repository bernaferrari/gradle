# Native-Ready Contract Policy

This document defines the compatibility boundary between the Rust substrate and the Gradle JVM daemon, the fail-closed policy for unsupported contracts, and the rules governing native-ready task execution.

## Compatibility Boundary

### Rust Owns the Hot Path

The Rust substrate owns all hot-path build execution for standard Java/JVM builds:

| Subsystem | Rust Responsibility | Status |
|-----------|-------------------|--------|
| CLI Launch | Native wrapper startup, daemon prewarm, endpoint reuse, distribution validation | verified |
| Daemon Lifecycle | Build process/session/tree/build/project state ownership with typed Rust contexts | verified |
| Plan Cache | Schema-versioned build-plan IR, configuration-cache hit execution without JVM | verified |
| DAG Scheduling | Critical-path-first task ordering, parallel work-stealing, cancellation, event streaming | verified |
| Dependency Transport | Maven/Ivy/HTTP artifact fetch, checksums, redirects, retries, auth/proxy/TLS | verified |
| Dependency Resolution | Conflict resolution, attributes, capabilities, platforms/BOMs, constraints, substitutions | partially-verified |
| VFS & Snapshots | File watching, path normalization, snapshots, fingerprinting, hashing, invalidation | verified |
| Persistent Caches | Build cache metadata, configuration-cache blobs, atomic commits, corruption detection | verified |
| Worker Orchestration | Worker pools, process launching, cancellation, stdout/stderr/progress frames | verified |
| Standard Task Execution | JavaCompile, Test, JavaExec, Javadoc, Jar, Copy, Sync, Exec, archive tasks | verified |
| Toolchain Detection | JVM toolchain detection, download/provisioning, validation, launcher/compiler path selection | verified |
| Publication | Maven/Ivy publishing artifact layout, metadata generation, signing, checksum verification | partially-verified |

### JVM Owns Compatibility Islands

The JVM daemon retains ownership of subsystems that require arbitrary code execution, reflection, or DSL evaluation:

| Subsystem | JVM Responsibility | Reason |
|-----------|-------------------|--------|
| Groovy/Kotlin DSL Evaluation | Build script compilation and execution | Requires Groovy/Kotlin runtime, arbitrary code |
| buildSrc Compilation | Custom build logic compilation and classloading | Arbitrary JVM code, no static contract |
| Arbitrary JVM Plugin Execution | Third-party plugin lifecycle hooks | Unknown reflection/API usage patterns |
| Reflection-Heavy APIs | Gradle public/internal APIs using reflection | Cannot statically determine access patterns |
| Model Builder Execution | IDE model construction via Tooling API | Arbitrary model builder implementations |
| Test Framework Classloading | JUnit/TestNG framework initialization | Requires JVM classloader isolation |

### Boundary Rule

> **Rust executes data-driven contracts. JVM executes code-driven contracts.**

A task is native-ready when its inputs, outputs, and execution semantics can be fully expressed as typed data without requiring runtime code evaluation. A task falls to the JVM compatibility island when it requires arbitrary code execution, reflection, or DSL evaluation that cannot be statically captured.

## Fail-Closed Policy

### Principle

> **Unsupported contracts fail closed, never approximate.**

When the Rust substrate encounters a task input, configuration, or execution semantic that it cannot handle with full fidelity, it must:

1. **Detect** the unsupported shape at contract capture time
2. **Report** the specific unsupported feature with a diagnostic message
3. **Fail closed** by falling back to JVM execution (or rejecting the build in authoritative mode)
4. **Never approximate** behavior that would produce different outputs than the JVM

### Implementation

```
┌─────────────────────────────────────────────────────────┐
│              Contract Capture (Rust Bridge)              │
│                                                         │
│  Task inputs ──► Type check ──► Supported? ──► Capture  │
│                                  │                      │
│                                  ▼ No                   │
│                          Emit diagnostic                │
│                          Mark unsupported               │
│                          Fall back to JVM               │
└─────────────────────────────────────────────────────────┘
```

### Examples

| Scenario | Correct Behavior | Incorrect Behavior |
|----------|-----------------|-------------------|
| Test filter with method-level granularity | Fail closed, emit "method-level filters unsupported" | Approximate with class-level filter |
| CopySpec with `eachFile` closure | Fail closed, emit "closure-based CopySpec unsupported" | Skip the closure transformation |
| Dependency with dynamic version range | Fail closed, emit "dynamic versions unsupported" | Resolve to latest stable |
| Task with custom `TaskAction` via reflection | Fail closed, emit "custom TaskAction unsupported" | Execute without the action |
| Plugin with `project.afterEvaluate` hook | Fail closed, emit "afterEvaluate hooks unsupported" | Skip the hook |

### Diagnostic Requirements

Every fail-closed decision must emit a diagnostic that includes:

1. **Task path** — fully qualified task identity (e.g., `:app:compileJava`)
2. **Unsupported feature** — specific feature name (e.g., "method-level test filter")
3. **Expected behavior** — what the JVM would do
4. **Fallback action** — what the substrate will do instead (e.g., "forwarding to JVM")

## Native-Ready Task Contract Rules

### Contract Capture

A task is native-ready when its contract can be captured as typed data at Gradle task-graph population time, before execution begins:

1. **Inputs are data** — All task inputs (source files, classpaths, options, properties) are captured as serializable values, not closures or providers requiring runtime resolution
2. **Outputs are declared** — All task outputs (files, directories, archives) are declared with known paths and types
3. **Execution is deterministic** — The task's execution semantics are fully specified by its inputs and do not depend on runtime code evaluation
4. **No reflection** — The task does not use reflection to access Gradle internals or project state

### Supported Native Task Types

| Task Type | Contract Shape | Key Inputs |
|-----------|---------------|------------|
| `JavaCompile` | Source files → class files | sourceDirs, classpath, annotationProcessorPath, options (source/target/release, encoding, compilerArgs) |
| `Test` | Class files → test results | testClassesDir, classpath, includes/excludes (class-name level), forks, jvmArgs |
| `JavaExec` | Classpath + main class → process exit | mainClass, classpath, jvmArgs, args, environment, workingDir |
| `Javadoc` | Source files → HTML docs | sourceFiles, classpath, destDir, title, encoding, maxMemory |
| `Jar`/`Zip`/`Tar` | Files → archive | sourceFiles, destFile, includes/excludes, manifest, compression |
| `Copy`/`Sync` | Files → files | sourceFiles, destDir, includes/excludes, rename, filter |
| `Exec` | Command → process exit | executable, args, environment, workingDir |
| `ProcessResources` | Resources → processed resources | sourceDirs, destDir, includes/excludes, filtering |

### Contract Versioning

Each native task type has a schema version in the build-plan IR:

- **Major version** — Breaking change to task inputs/outputs semantics
- **Minor version** — Addition of new optional inputs
- **Patch version** — Bug fix or diagnostic improvement

When a cached plan has a different schema version than the current substrate, the plan is invalidated and re-captured from the JVM.

## Non-Goals

The following are explicitly **out of scope** for the Rust substrate hot-path roadmap:

### 1. Groovy/Kotlin DSL Evaluation

The Rust substrate will not compile or execute Groovy or Kotlin build scripts. Build script evaluation remains a JVM compatibility island because:

- Groovy and Kotlin have complex runtime semantics (metaprogramming, extension functions, closures)
- Build scripts can invoke arbitrary JVM code
- The DSL surface is effectively unbounded

### 2. Arbitrary buildSrc/JVM Plugin Execution

The Rust substrate will not execute arbitrary plugins loaded from `buildSrc` or the buildscript classpath because:

- Plugins can register arbitrary task types with custom actions
- Plugin lifecycle hooks (`apply`, `afterEvaluate`, `projectLoaded`) execute arbitrary code
- Plugin dependencies may use reflection to access Gradle internals

### 3. Reflection-Heavy Gradle APIs

The Rust substrate will not implement Gradle public or internal APIs that rely on reflection because:

- Reflection patterns are not statically analyzable
- API consumers may use undocumented internal APIs
- Binary compatibility guarantees require runtime dispatch

### 4. Dynamic Build Features

The following dynamic build features are not supported in native-ready mode:

- `buildSrc` compilation and classloading
- `settings.gradle` dynamic includes
- `gradle.beforeProject`/`afterProject` hooks
- `gradle.taskGraph.whenReady` callbacks
- `project.afterEvaluate` blocks
- Custom `Configuration` resolution strategies
- Dynamic dependency versions (`1.+`, `latest.release`)
- Dependency substitution rules
- Resolution strategies with closures

### 5. Test Framework Initialization

Test framework classloading and initialization remains on the JVM because:

- JUnit Platform, TestNG, and other frameworks require JVM classloader isolation
- Test discovery uses reflection to find test classes and methods
- Custom test listeners and reporters may use arbitrary JVM code

## Enforcement

### Contract Gate

The authoritative execution gate enforces the fail-closed policy:

```
org.gradle.rust.substrate.runbuild.authoritative=true
```

In authoritative mode:
- Rust `RunBuild` executes the task DAG
- If any task fails closed, the build fails with a diagnostic
- JVM fallback is **not** available in authoritative mode
- This mode is used for validation and CI

### Shadow Mode

In shadow mode, both Rust and JVM execute the same tasks:

```
org.gradle.rust.substrate.runbuild.shadow=true
```

Shadow mode:
- Captures native-ready contracts from the JVM execution
- Executes the same contracts through Rust
- Compares results for parity validation
- Reports mismatches as test failures

### Contract Tests

Each native task type has contract tests that validate:

1. **Capture fidelity** — Inputs captured from JVM match the typed contract
2. **Execution parity** — Rust execution produces identical outputs to JVM
3. **Fail-closed behavior** — Unsupported shapes are detected and reported
4. **Schema versioning** — Cached plans are invalidated on schema changes

## References

- [Rust Substrate Architecture](rust-substrate-architecture.md)
- [Rust Substrate Stabilization](rust-substrate-stabilization.md)
- [Build Execution Model](build-execution-model.md)
- [PARITY.md](../substrate/PARITY.md) — Current parity status
