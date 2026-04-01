# Migration Status: extensibility

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `ExtensionContainer` and dynamic extension registration
- `Convention` mapping and lazy property infrastructure
- `PluginContainer` and plugin application lifecycle
- `ObjectFactory` and managed type creation
- `DomainObjectCollection` and live collections

### Phase 2: Mixed (Not Started)
- Planned: managed type creation and property infrastructure
- Extension registration could shadow once plugin lifecycle protocol is defined
- Deeply tied to JVM reflection and proxy mechanisms

### Phase 3: Native (Target)
- Managed type schema generation
- Property wiring and lazy evaluation
- Remaining JVM dependencies: reflection-based proxy creation, Groovy/Kotlin DSL integration, plugin classloaders

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `ExtensionContainer` | — | jvm-only | Tied to JVM reflection |
| `ObjectFactory` | — | jvm-only | Managed types use JVM proxies |
| `Convention` | — | jvm-only | Property wiring is JVM-specific |
| `PluginContainer` | — | jvm-only | Classloader-based plugin loading |

## Blockers

- Managed type creation relies on JVM dynamic proxies
- Plugin loading uses JVM classloader isolation
- DSL integration (Groovy/Kotlin) has no Rust equivalent

## Next Steps

1. Define extension registration protocol over gRPC
2. Implement managed type schema in Rust (declarative, not reflective)
3. Shadow extension container behavior against JVM baseline
