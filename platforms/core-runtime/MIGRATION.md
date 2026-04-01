# Migration Status: core-runtime

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `GradleInternal` lifecycle management
- `BuildSessionScope` and `BuildOperation` infrastructure
- `ServiceRegistry` and `ScopedServiceFactory` patterns
- `BuildCancellationToken` and cancellation propagation
- `FileWatching` integration points

### Phase 2: Mixed (Not Started)
- No shadow services defined yet
- Planned: file watching and build operation reporting as first candidates
- Requires stable `JvmHostService` callback protocol

### Phase 3: Native (Target)
- File watching daemon (already partially implemented in `substrate/src/server/file_watching_service.rs`)
- Build operation event fan-out
- Remaining JVM dependencies: classpath resolution, plugin application lifecycle

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `FileWatching` | `FileWatchingService` | pending | Proto defined, shadow not wired |
| `BuildOperationProgress` | `BuildOperationService` | pending | Event fan-out exists in Rust |
| `ServiceRegistry` | — | jvm-only | No Rust equivalent planned |
| `BuildCancellationToken` | — | jvm-only | Cancellation flows through gRPC |

## Blockers

- ServiceRegistry lifecycle has no clear Rust analogue
- Build operation semantics require full DAG context
- Plugin application order affects service initialization

## Next Steps

1. Wire `FileWatchingService` in shadow mode via rust-bridge
2. Validate build operation event ordering against JVM baseline
3. Define cancellation propagation protocol over gRPC
