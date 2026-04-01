# Migration Status: native

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `NativeComponentSpec` and variant modeling
- `CppCompile`, `CppLink`, and `Assembler` task types
- Toolchain detection for C/C++/Swift
- Platform-aware dependency resolution for native libraries

### Phase 2: Mixed (Not Started)
- Planned: native toolchain detection
- Compilation task configuration could shadow once toolchain protocol is defined
- Actual compilation/linking remains external process invocations

### Phase 3: Native (Target)
- Toolchain detection and selection
- Compilation task configuration
- Remaining JVM dependencies: actual compiler invocation (gcc/clang), linker execution

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `NativeToolchain` | — | pending | Toolchain detection is pure computation |
| `CppCompile` | — | jvm-only | Requires external compiler |
| `CppLink` | — | jvm-only | Requires external linker |
| `PlatformResolver` | — | pending | Platform detection is computation |

## Blockers

- Actual compilation requires invoking external compilers
- Linker execution is platform-specific
- Header dependency scanning requires filesystem access

## Next Steps

1. Implement native toolchain detection in Rust
2. Define compilation task configuration protocol
3. Shadow toolchain resolution against JVM baseline
