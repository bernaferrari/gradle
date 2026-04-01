# Migration Status: core-configuration

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `ProjectConfiguration` and `TaskGraph` construction
- `ConfigurationContainer` and dependency resolution orchestration
- `ConfigurationCache` serialization/deserialization
- `ScriptPlugin` application and model rule execution

### Phase 2: Mixed (Not Started)
- Planned: configuration cache read/write as first shadow candidate
- Configuration cache storage already has Rust equivalent in `substrate/src/server/configuration_cache_service.rs`
- Requires `JvmHostService` callback for classpath-dependent resolution

### Phase 3: Native (Target)
- Configuration cache persistence layer
- Task graph serialization
- Remaining JVM dependencies: Groovy/Kotlin DSL evaluation, plugin classloaders

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `ConfigurationCache` | `ConfigurationCacheService` | pending | Proto defined, not wired |
| `TaskGraph` | `TaskGraphService` | pending | DAG executor exists in Rust kernel |
| `ProjectConfiguration` | — | jvm-only | Tied to DSL evaluation |
| `DependencyResolution` | — | jvm-only | Complex classpath semantics |

## Blockers

- DSL evaluation (Groovy/Kotlin) has no Rust equivalent
- Plugin classloader isolation is JVM-specific
- Configuration cache format must remain binary-compatible

## Next Steps

1. Shadow configuration cache write path via rust-bridge
2. Validate cache entry hashes against JVM baseline
3. Implement configuration cache read path in shadow mode
