# Migration Status: subprojects/core-api

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- Public API interfaces (`Project`, `Task`, `Settings`, `Gradle`)
- `Plugin` interface and plugin development API
- `Transform` and `ArtifactTransformation` APIs
- `Provider` and `Property` lazy evaluation API

### Phase 2: Mixed (Not Started)
- Planned: provider/property API semantics
- API interfaces themselves remain JVM (consumer-facing)
- Internal implementation could shadow once protocol is defined

### Phase 3: Native (Target)
- Provider/property evaluation semantics
- API contract validation
- Remaining JVM dependencies: all public API interfaces (by design, consumers interact with JVM)

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `Provider<T>` | — | jvm-only | Public API, must remain JVM |
| `Property<T>` | — | jvm-only | Public API, must remain JVM |
| `Plugin<T>` | — | jvm-only | Public API, must remain JVM |
| `Transform` | — | jvm-only | Tied to JVM artifact system |

## Blockers

- Public API interfaces must remain on JVM for consumer compatibility
- Plugin development API is inherently JVM-bound
- Provider/property semantics depend on JVM lazy evaluation

## Next Steps

1. Define provider/property evaluation protocol
2. Implement lazy evaluation semantics in Rust (internal only)
3. Shadow provider behavior against JVM baseline
