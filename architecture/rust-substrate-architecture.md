# Rust Substrate Architecture

## Overview

The Rust substrate replaces Gradle JVM subsystems via a strangler-fig pattern. A Rust daemon runs alongside the JVM, communicating over gRPC over Unix domain sockets. Subsystems are independently migrated from shadow mode to authoritative mode.

## Component Diagram

```
                         ┌─────────────────────────────────┐
                         │         Gradle JVM               │
                         │                                 │
  Build Request ────────►│  Build Operation  ──────────┐  │
                         │  Orchestrator                │  │
                         │                              ▼  │
                         │                    ┌───────────┐│
                         │                    │  Bridge   ││
                         │                    │  Clients  ││
                         │                    │  (Java)   ││
                         │                    └─────┬─────┘│
                         │                          │gRPC  │
                         │                    ┌─────▼─────┐│
                         │                    │  Control  ││
                         │                    │  Service  ││
                         │                    └───────────┘│
                         │                              │  │
                         │  JVM Compatibility    ◄──────┘  │
                         │  Host (reverse RPC)             │
                         └─────────────────────────────────┘
                                    ▲          │
                              JvmHostClient    │ Unix Socket
                                    │          ▼
                         ┌─────────────────────────────────┐
                         │       Rust Daemon                │
                         │                                  │
                         │  ┌──────────────────────────┐   │
                         │  │     gRPC Server (tonic)   │   │
                         │  │     38 services            │   │
                         │  └──────────────────────────┘   │
                         │                                  │
                         │  ┌────────┐  ┌──────────────┐  │
                         │  │  DAG   │  │ File Watch   │  │
                         │  │Executor│  │ (notify)     │  │
                         │  └────────┘  └──────────────┘  │
                         │                                  │
                         │  ┌────────┐  ┌──────────────┐  │
                         │  │  Hash  │  │  Cache       │  │
                         │  │(MD5/   │  │  Orchestration│  │
                         │  │ SHA/   │  │              │  │
                         │  │ BLAKE3)│  │              │  │
                         │  └────────┘  └──────────────┘  │
                         └─────────────────────────────────┘
```

## Service Taxonomy

### Stateless Services (request → response, no persistent state)
HashService, ClasspathService, FileTreeService, ParserService, ValueSnapshotService, FileFingerprintService, VersionCatalogService, ConfigurationService, BuildLayoutService, ToolchainService, IvyParserService, PluginService

### Stateful Services (in-memory or disk-backed state)
- **CacheService** — disk-backed cache entries with LRU eviction
- **CacheOrchestrationService** — coordinates cache reads/writes across tiers
- **ConfigCacheService** — configuration cache IR with validation
- **ExecutionHistoryService** — stores execution results for reuse
- **ExecutionPlanService** — maintains build plan graph
- **TaskGraphService** — task dependency graph
- **RemoteCacheService** — HTTP-based remote cache client
- **BuildModelCache** — IDE model data (populated from JVM)

### Event-Driven Services (streaming/notifications)
- **BuildEventStreamService** — fans out build events to subscribers
- **EventDispatcherService** — routes events to listeners
- **FileWatchService** — filesystem change notifications
- **BuildOperationsService** — tracks build operation lifecycle
- **MetricsService** — collects and reports build metrics

### Execution Services
- **DAG Executor** — native task graph executor with parallel scheduling
- **ExecService** — process execution (fork, signals, I/O)
- **WorkerProcessService** — worker process lifecycle management
- **ParallelSchedulerService** — work-stealing parallel scheduler
- **TestExecutionService** — test discovery and execution
- **IncrementalCompilationService** — compile avoidance and incremental processing

## Shadow Mode Protocol

Each subsystem independently progresses through three modes:

1. **Off** — JVM handles all requests, Rust service is not called
2. **Shadow** — JVM handles requests, Rust service is called in parallel, results compared (differential tests)
3. **Authoritative** — Rust handles requests, JVM result is compared for validation

Controlled via `SubsystemModes` in `authoritative.rs`, configured by `-Dorg.gradle.rust.substrate.<subsystem>.authoritative=true`.

The `ControlService` exposes `GetSubsystemModes` and `SetSubsystemMode` RPCs for runtime control.

## JVM Host Bridge

The Rust daemon can call back into the JVM via `JvmHostClient`:

```
Rust ──► JvmHostClient ──► gRPC ──► JvmHostServer (Java) ──► Gradle internals
```

Used for: build model queries, classloader information, Groovy/Kotlin evaluation, and any operation that requires JVM state.

## Proto Contract

Canonical source: `substrate/proto/v1/` (27 files).

Sync to Java: `./gradlew :rust-bridge:syncProtos`

Validation: `python3 tools/upstream_map/check_proto_lock.py`

Proto `int64` → Java `long`, Proto `int32` → Java `int`. Use `google.protobuf.Timestamp` for timestamps.
