# Migration Status: core-execution

**Status:** `mixed`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- `DefaultTaskExecutionPlan` and task execution ordering
- `BuildOperation` progress reporting
- `WorkerDaemon` lifecycle management
- `IncrementalTaskInputs` tracking
- `TaskArtifactState` persistence

### Phase 2: Mixed (In Progress)
- Rust daemon kernel (`substrate/`) implements 38 protocol services
- JVM bridge clients in `rust-bridge/` provide gRPC connectivity
- Shadow mode available via `-Dorg.gradle.rust.substrate.*` feature flags
- `JvmHostService` callback protocol enables Rust→JVM calls
- Hash compatibility suite validates cross-language digest behavior

### Phase 3: Native (Target)
- Task execution engine (DAG executor implemented in Rust)
- File watching daemon
- Build operation event fan-out
- Remaining JVM dependencies: task action execution, worker processes, plugin hooks

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `TaskExecution` | `TaskExecutionService` | shadow | DAG executor in Rust, actions on JVM |
| `BuildOperation` | `BuildOperationService` | shadow | Event fan-out validated |
| `FileWatching` | `FileWatchingService` | shadow | Proto defined, not authoritative |
| `TaskArtifactState` | `ArtifactStateService` | shadow | Hash compatibility suite passing |
| `WorkerDaemon` | `WorkerDaemonService` | pending | Worker lifecycle not yet modeled |
| `IncrementalTaskInputs` | `IncrementalBuildService` | pending | Input tracking in progress |

## Blockers

- Task action execution requires JVM classloaders
- Worker daemon protocol is JVM-specific
- Plugin lifecycle hooks have no Rust equivalent yet

## Next Steps

1. Expand shadow validation to all 38 services
2. Flip individual services to authoritative mode after parity validation
3. Implement worker daemon abstraction in Rust
4. Reduce rust-bridge compile exclusions incrementally
