# Migration Status: subprojects/composite-builds

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `IncludedBuild` and composite build orchestration
- `CompositeProjectResolver` and cross-build dependency resolution
- `SubstitutionRule` handling for dependency substitution
- `BuildIdentifier` and build tree management

### Phase 2: Mixed (Not Started)
- Planned: dependency substitution rule evaluation
- Substitution logic is pure computation, good first candidate
- Requires stable composite build protocol over gRPC

### Phase 3: Native (Target)
- Dependency substitution rule evaluation
- Composite project resolution
- Remaining JVM dependencies: included build launch, cross-build communication

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `SubstitutionRule` | — | pending | Rule evaluation is computation |
| `CompositeProjectResolver` | — | pending | Resolution logic is computation |
| `IncludedBuild` | — | jvm-only | Requires launching separate builds |
| `BuildTreeController` | — | jvm-only | Build orchestration is JVM |

## Blockers

- Included build launching requires JVM process management
- Cross-build communication uses JVM-specific protocols
- Build tree management depends on full Gradle runtime

## Next Steps

1. Implement dependency substitution rule evaluation in Rust
2. Define composite build resolution protocol
3. Shadow substitution behavior against JVM baseline
