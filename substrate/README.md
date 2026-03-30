# Gradle Substrate Daemon

Strangler-fig migration of Gradle's execution substrate to Rust. The daemon communicates with the JVM via gRPC over Unix domain sockets, progressively replacing Java subsystems.

## Quick Start

**Prerequisites:** Rust stable toolchain, Java 17+

```bash
cd substrate
cargo build              # Debug build
cargo build --release    # Release build (4.9MB, LTO + strip)
cargo test               # All 1152 tests
cargo test --lib         # Unit tests only (1090)
cargo test --test integration_test  # Integration tests (50)
cargo clippy             # Lint (must be clean)
cargo bench              # Criterion benchmarks (9)
cargo doc                # Generate docs
```

## Architecture

```
┌──────────────┐     gRPC/Unix Sockets     ┌──────────────────────┐
│   JVM        │ ◄──────────────────────► │  Rust Daemon          │
│   Gradle     │                           │  (substrate/)        │
│              │  38 gRPC services          │                      │
│  Bridge      │  27 proto files            │  tonic + tokio       │
│  Clients     │  11 subsystems             │  rayon (parallelism) │
└──────────────┘                           └──────────────────────┘
         ▲                                         ▲
         │        JvmHostService (reverse RPC)      │
         └─────────────────────────────────────────┘
```

## Service Catalog

| Service | Proto | Implementation | Subsystem |
|---------|-------|---------------|-----------|
| HashService | hash.proto | hash.rs | hashing |
| CacheService | cache.proto | cache.rs | cache_keys |
| ValueSnapshotService | fingerprint.proto | value_snapshot.rs | value_snapshots |
| ExecutionHistoryService | execution.proto | execution_history.rs | execution_history |
| TaskGraphService | taskgraph.proto | task_graph.rs | task_graph |
| FileFingerprintService | fingerprint.proto | file_fingerprint.rs | file_fingerprinting |
| ExecutionPlanService | buildplan.proto | execution_plan.rs | execution_plan |
| ConfigCacheService | cache.proto | config_cache.rs | config_cache |
| ClasspathService | classpath.proto | classpath.rs | classpath |
| FileTreeService | filetree.proto | file_tree.rs | file_tree |
| VersionCatalogService | versioncatalog.proto | version_catalog.rs | version_catalog |
| ParserService | parser.proto | parser_service.rs | — |
| ExecService | exec.proto | exec.rs | — |
| WorkerProcessService | worker.proto | worker_process.rs | — |
| FileWatchService | filewatch.proto | file_watch.rs | — |
| DependencyResolutionService | dependency.proto | dependency_resolution.rs | — |
| IncrementalCompilationService | incremental.proto | incremental_compilation.rs | — |
| ToolchainService | toolchain.proto | toolchain.rs | — |
| TestExecutionService | testexec.proto | test_execution.rs | — |
| RemoteCacheService | cache.proto | remote_cache.rs | — |
| BuildOperationsService | buildops.proto | build_operations.rs | — |
| BuildEventStreamService | reporting.proto | build_event_stream.rs | — |
| ResourceManagementService | resources.proto | resource_management.rs | — |
| BootstrapService | bootstrap.proto | bootstrap.rs | — |
| ControlService | control.proto | control.rs | — |
| BuildLayoutService | buildlayout.proto | build_layout.rs | — |
| ConfigurationService | configuration.proto | configuration.rs | — |
| PluginService | — | plugin.rs | — |
| BuildInitService | — | build_init.rs | — |
| ConsoleService | — | console.rs | — |
| GarbageCollectionService | — | garbage_collection.rs | — |
| IvyParserService | — | ivy_parser.rs | — |
| MetricsService | metrics.proto | build_metrics.rs | — |
| ProblemReportingService | — | problem_reporting.rs | — |
| ArtifactPublishingService | publishing.proto | artifact_publishing.rs | — |
| ParallelSchedulerService | — | parallel_scheduler.rs | — |
| BuildComparisonService | — | build_comparison.rs | — |
| DAG Executor | — | dag_executor.rs | — |

## Authoritative Subsystems

11 subsystems run in shadow mode (off/shadow/on). Controlled via `SubsystemModes` in `authoritative.rs`:

| Subsystem | Feature Flag |
|-----------|-------------|
| hashing | `org.gradle.rust.substrate.hashing.authoritative` |
| cache_keys | `org.gradle.rust.substrate.cache-keys.authoritative` |
| value_snapshots | `org.gradle.rust.substrate.value-snapshots.authoritative` |
| execution_history | `org.gradle.rust.substrate.execution-history.authoritative` |
| task_graph | `org.gradle.rust.substrate.task-graph.authoritative` |
| file_fingerprinting | `org.gradle.rust.substrate.file-fingerprinting.authoritative` |
| execution_plan | `org.gradle.rust.substrate.execution-plan.authoritative` |
| config_cache | `org.gradle.rust.substrate.config-cache.authoritative` |
| classpath | `org.gradle.rust.substrate.classpath.authoritative` |
| file_tree | `org.gradle.rust.substrate.file-tree.authoritative` |
| version_catalog | `org.gradle.rust.substrate.version-catalog.authoritative` |

## Testing

- **1090 unit tests** — service logic, edge cases, error handling
- **50 integration tests** — full gRPC round-trips over Unix sockets
- **12 differential tests** — compare Java vs Rust outputs for correctness
- **9 benchmarks** — criterion benchmarks for hot paths (hashing, fingerprinting, classpath, file tree, cache, DAG, build plan, incremental, configuration)

## Key Files

- `src/main.rs` — Daemon binary, wires all 38 services
- `src/server/` — 51 service implementation files
- `src/client/jvm_host.rs` — Rust client for JVM callback RPC
- `proto/v1/` — 27 proto definitions
- `tests/` — Integration + E2E tests
- `benches/` — Criterion benchmarks

## Java Bridge

- `platforms/core-execution/rust-bridge/` — Java gRPC bridge clients
- `platforms/core-execution/rust-bridge/jvmhost/` — JVM Compatibility Host
- `platforms/core-runtime/build-option/` — Feature flags (`RustSubstrateOptions.java`)
