# Migration Status: ide

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `IdeaProject` and `IdeaModule` generation
- `EclipseProject` and `EclipseClasspath` generation
- IDE tooling model serialization
- Source set mapping and output directory configuration

### Phase 2: Mixed (Not Started)
- Planned: IDE model generation as first candidate
- Model serialization is pure computation, good migration target
- Requires stable project model protocol over gRPC

### Phase 3: Native (Target)
- IDE model generation (XML/JSON output)
- Source set mapping computation
- Remaining JVM dependencies: project model construction, source set discovery

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `IdeaProjectGenerator` | — | pending | XML generation is pure computation |
| `EclipseProjectGenerator` | — | pending | XML generation is pure computation |
| `SourceSetMapping` | — | pending | Computation, depends on project model |
| `ToolingModelBuilder` | — | jvm-only | Tied to tooling API |

## Blockers

- Project model construction depends on full Gradle runtime
- Tooling API protocol is JVM-specific
- Source set discovery requires filesystem scanning

## Next Steps

1. Define IDE model generation protocol
2. Implement IDEA XML generator in Rust
3. Shadow model output against JVM baseline
