# Migration Status: substrate

**Status:** `native`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Complete)
- Original Gradle execution engine on JVM
- All 38 services previously ran on JVM
- Baseline behavior captured through differential tests

### Phase 2: Mixed (Current)
- Rust daemon kernel implements 38 protocol services
- 1152 tests passing (1090 unit + 50 integration + 12 differential)
- `JvmHostService` callback protocol enables Rust→JVM communication
- Shadow mode available via `-Dorg.gradle.rust.substrate.*` feature flags
- Hash compatibility suite validates cross-language digest behavior
- Native DAG executor implemented and tested
- 9 criterion benchmarks tracking performance

### Phase 3: Native (In Progress)
- Task execution engine (DAG executor authoritative)
- File watching service (proto defined, shadow not yet authoritative)
- Build operation event fan-out (validated)
- Configuration cache service (proto defined, not yet wired)
- Artifact state service (hash compatibility suite passing)
- Remaining JVM dependencies: task action execution, worker processes, plugin lifecycle

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `TaskExecution` | `TaskExecutionService` | authoritative | DAG executor in Rust |
| `BuildOperation` | `BuildOperationService` | shadow | Event fan-out validated |
| `FileWatching` | `FileWatchingService` | shadow | Proto defined, not authoritative |
| `TaskArtifactState` | `ArtifactStateService` | shadow | Hash compatibility passing |
| `ConfigurationCache` | `ConfigurationCacheService` | pending | Not yet wired |
| `WorkerDaemon` | `WorkerDaemonService` | pending | Worker lifecycle not modeled |
| `JvmHost` | `JvmHostService` | authoritative | Callback protocol stable |
| `Hashing` | `HashingService` | authoritative | Hash compatibility validated |
| `PersistentCache` | `PersistentCacheService` | authoritative | Cache layer in Rust |
| `ScopedPersistentCache` | `ScopedCacheService` | authoritative | Scoped cache in Rust |

## Blockers

- Task action execution requires JVM classloaders
- Worker daemon protocol is JVM-specific
- Plugin lifecycle hooks have no Rust equivalent
- Dependency resolution semantics not fully parity-complete

## Next Steps

1. Expand shadow validation across all remaining services
2. Flip individual services to authoritative mode after parity validation
3. Implement worker daemon abstraction in Rust
4. Complete configuration cache wiring
5. Reduce rust-bridge compile exclusions incrementally
