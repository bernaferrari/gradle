# Migration Status: documentation

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `Asciidoctor` task integration
- API documentation generation (`Javadoc`, `Groovydoc`)
- Sample code extraction and validation
- User guide and DSL reference generation

### Phase 2: Mixed (Not Started)
- Planned: sample code extraction and validation
- Pure computation tasks are good first candidates
- Documentation generation is low priority for migration

### Phase 3: Native (Target)
- Sample code extraction and validation
- Documentation metadata generation
- Remaining JVM dependencies: Asciidoctor rendering, javadoc/groovydoc generation

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `SampleExtractor` | — | pending | File processing, good first candidate |
| `AsciidoctorTask` | — | jvm-only | Requires Asciidoctor JVM |
| `JavadocTask` | — | jvm-only | Requires javadoc tool |
| `DslReferenceGen` | — | pending | Metadata extraction is computation |

## Blockers

- Asciidoctor rendering is JVM-only
- Javadoc/groovydoc generation requires JVM tools
- Low migration priority compared to execution engine

## Next Steps

1. Implement sample code extraction in Rust
2. Define documentation metadata protocol
3. Shadow sample validation against JVM baseline
