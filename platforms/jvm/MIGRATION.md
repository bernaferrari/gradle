# Migration Status: jvm

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- `JavaCompile` task and toolchain management
- `JavaPlatform` and `JvmLibrary` variant modeling
- `AnnotationProcessor` configuration
- `Test` task execution with JUnit/TestNG integration
- `Javadoc` generation and `JavaExec` task

### Phase 2: Mixed (Not Started)
- Planned: Java toolchain resolution and detection
- Compilation task configuration could shadow once toolchain protocol is defined
- Test execution remains JVM-bound (requires actual JVM for running tests)

### Phase 3: Native (Target)
- Toolchain detection and resolution
- Compilation task configuration and incremental compilation tracking
- Remaining JVM dependencies: actual compilation (javac), test execution, javadoc generation

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `JavaToolchain` | — | pending | Toolchain detection is pure computation |
| `JavaCompile` | — | jvm-only | Requires javac invocation |
| `Test` | — | jvm-only | Must run on JVM |
| `AnnotationProcessor` | — | jvm-only | Tied to javac plugin API |

## Blockers

- Actual compilation requires invoking javac (JVM process)
- Test execution must run on JVM by definition
- Annotation processing is a javac compiler plugin

## Next Steps

1. Implement Java toolchain detection in Rust
2. Define compilation task configuration protocol
3. Shadow toolchain resolution against JVM baseline
