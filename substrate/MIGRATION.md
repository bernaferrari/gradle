# Migration status: substrate

How the Rust substrate sits relative to Gradle JVM ownership. Product matrix and
gates: [`docs/rust-substrate-preview.md`](../docs/rust-substrate-preview.md).  
Strategy: [`docs/rust-substrate-turbopack-plan.md`](../docs/rust-substrate-turbopack-plan.md).  
Parity snapshot: [`PARITY.md`](PARITY.md).

Evidence boundary: the last documented pre-rebase run at `7eabe3f` on
2026-07-16 reported 8/8 supported direct-warm entries and 9/9 full-manifest
entries (8 supported + 1 fail-closed). The current rebased worktree has not
been built or tested, so migration targets added or refined here remain
unverified until those gates are rerun.

## Model

```
User / Gradle CLI
       │
       ▼
┌──────────────────────┐     capture IR / task contracts      ┌─────────────────────┐
│ JVM compatibility    │ ───────────────────────────────────► │ Rust substrate      │
│ frontend             │                                      │ daemon + RunBuild   │
│ (DSL, plugins,       │ ◄── gRPC / protos (substrate/proto) ─│                     │
│  unsupported paths)  │                                      │ admitted DAG only   │
└──────────────────────┘                                      └─────────────────────┘
```

1. JVM evaluates settings, DSL, plugins, and builds a typed plan / dependency
   contract where needed.
2. Rust **admits or rejects** the selected plan (execution kernel).
3. On accept in authoritative mode, Rust executes the DAG with **no JVM task
   forwards**.
4. On reject or unsupported CLI options, the path stays on the JVM compatibility
   frontend—or fails closed—never hidden post-admission fallback.

Warm path: after one successful capture, `gradle-substrate-runbuild` /
`tools/warm_runner/run.py` may skip JVM configuration when the shadow artifact
and fingerprints are still valid.

## Migration stages

| Stage | Description | Status (preview slice) |
| --- | --- | --- |
| Sidecar services | Hash, cache helpers, fingerprints over gRPC | Done / ongoing hardening |
| Shadow execution | Parallel compare, reporters, no ownership claim | Used when landing surfaces |
| Kernel admission | Whole-plan accept/reject before dispatch | Done for preview flags |
| Authoritative RunBuild | Zero-forward native executors on admitted plans | Done for supported fixtures |
| Direct warm | Cached build-plan shadow without Gradle config | Done for supported dogfood set |
| Dep graph in Rust | Solver + repo IO for common shapes | Partial (fixture-backed) |
| Configuration IR | Durable graph + native replay for built-ins | Partial |
| Native plugin ABI | New plugins without JVM | Partial (built-in set) |
| JVM-optional shell | Rust-first CLI; JVM on demand | In progress |

## What already migrated (authoritative on admitted paths)

- Execution kernel and strict no-forward RunBuild  
- Build-plan shadow store, fingerprint validation, warm runner  
- File watch, snapshots, hashing, selected file ops and archives  
- Task families listed in the preview matrix (Java lifecycle, copy/sync,
  archives, test/exec/javaexec/javadoc, start scripts, static reports)  
- Local up-to-date / cache pack-unpack for captured work  
- Daemon endpoint persistence and reuse with identity checks  

## In flight (not promoted)

- One checked-in Kotlin JVM library fixture can lower its narrow
  `KotlinCompile` shape to a Rust executor that launches the host `kotlinc`.
  This target is unverified after the rebase, exempts `*.class` and
  `*.kotlin_module` content hashes, and compares archives by entry inventory
  only. It does not establish Kotlin Gradle plugin, compiler-plugin, or Kotlin
  build-logic parity.

## Still JVM-owned or fail-closed

- Full DSL and `buildSrc`  
- Arbitrary and third-party plugins (until ABI or guest runtime)  
- Composite `includeBuild` substitution; captured classpath files do not model
  included-build producer tasks or cross-build dependency edges. The focused
  `testing/dogfood/manifest-composite-only.json` gate is experimental and does
  not change this ownership boundary
- Kotlin build logic, precompiled Kotlin DSL plugins, Kotlin compiler plugins,
  and Kotlin Gradle plugin task shapes outside the in-flight fixture
- Rich resolution rules, custom transforms, unsupported GMM  
- Tooling API and IDE model breadth  
- Anything not on the preview matrix  

## Feature flags (common)

| Flag / property | Role |
| --- | --- |
| `org.gradle.rust.substrate.execution.kernel=true` | Strict kernel / authoritative RunBuild |
| `runbuild.authoritative` | Compatibility alias for kernel mode |
| Wrapper `--rust-substrate-kernel` | User-facing strict flag |
| `--rust-substrate-direct` / `GRADLEW_RUST_DIRECT_RUNBUILD` | Try warm runbuild before Gradle |
| Per-surface `ENABLE_RUST_*` bridge options | Opt-in wiring while shadowing |

Prefer documented kernel/direct flags over one-off env hacks.

## How to migrate a new slice

1. **Boundary:** strong module edge, serializable contract, easy differential test.  
2. **Fixture:** corpus or dogfood project that needs the shape.  
3. **Shadow:** implement Rust path; compare; keep JVM truth.  
4. **Admission:** reject unsupported variants with a precise reason.  
5. **Authoritative:** enable only when zero-forward + output/hash/archive (and
   graph parity if applicable) pass on the fixture.  
6. **Docs:** add the shape to the preview matrix; one line in `PARITY.md` if
   status changed.  
7. **No scope creep:** do not widen “approximate support” to green a build.

## Proto / bridge

- Protos: `substrate/proto/v1/`  
- Sync Java: `./gradlew :rust-bridge:syncProtos`  
- Bridge: `platforms/core-execution/rust-bridge/`  
- Maintenance: `architecture/rust-substrate-maintenance.md` (schema, ownership)

## Validation

See `PARITY.md` for the full command list. Minimum before calling a slice done:

```bash
cargo test -p gradle-substrate-daemon --lib
# fixture-level authoritative corpus or dogfood with zero JVM forwards
python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative \
  --tasks clean build --timeout 300
```

## History note

Older revisions of this file accumulated per-session agent logs. Those logs are
not migration history. Durable progress is the phase table above plus the
turbopack plan checkpoints and preview evidence artifacts.
