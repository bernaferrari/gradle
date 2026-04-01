# Migration Status: software

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `SoftwareComponent` model and variant-aware resolution
- `Publication` and `Repository` management
- `DependencyConstraint` handling
- `ModuleMetadata` parsing (Ivy, Maven, Gradle Module Metadata)
- `Transform` and `ArtifactType` resolution

### Phase 2: Mixed (Not Started)
- Planned: module metadata parsing as first candidate (pure computation)
- Dependency resolution graph construction could migrate after core-execution stabilizes
- Requires stable dependency resolution protocol over gRPC

### Phase 3: Native (Target)
- Module metadata parsing and normalization
- Dependency graph construction
- Remaining JVM dependencies: Ivy/Maven repository connectors, artifact download, signature verification

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `ModuleMetadataParser` | — | pending | Pure computation, good first candidate |
| `DependencyGraphBuilder` | — | pending | Depends on resolution protocol |
| `Publication` | — | jvm-only | Tied to JVM artifact transforms |
| `RepositoryConnector` | — | jvm-only | HTTP client on JVM |

## Blockers

- Repository connectors depend on JVM HTTP stack
- Artifact transforms use JVM classloaders
- Signature verification relies on JVM crypto APIs

## Next Steps

1. Implement module metadata parser in Rust (pure computation)
2. Define dependency resolution gRPC protocol
3. Shadow metadata parsing against JVM baseline
