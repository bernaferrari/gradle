# Migration Status: subprojects/core

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `GradleLauncher` and build invocation entry points
- `BuildLifecycleController` and build execution orchestration
- `ServiceRegistryBuilder` and service wiring
- `BuildExecuter` and build action dispatch
- `CommandLineConverter` and CLI argument parsing

### Phase 2: Mixed (Not Started)
- Planned: CLI argument parsing and build configuration
- Build launcher entry point will remain JVM (process boundary)
- Requires stable build lifecycle protocol over gRPC

### Phase 3: Native (Target)
- Build configuration and project loading
- Service registry wiring
- Remaining JVM dependencies: process launch, classloader setup, plugin application

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `CommandLineConverter` | — | pending | Pure computation, good first candidate |
| `BuildLifecycleController` | — | pending | Depends on daemon protocol |
| `ServiceRegistryBuilder` | — | jvm-only | JVM-specific service wiring |
| `BuildExecuter` | `TaskExecutionService` | shadow | DAG executor exists in Rust |

## Blockers

- Process launch and JVM bootstrap are JVM-specific
- Classloader setup has no Rust equivalent
- Plugin application requires JVM classloaders

## Next Steps

1. Implement CLI argument parsing in Rust
2. Define build lifecycle protocol over gRPC
3. Shadow build configuration against JVM baseline
