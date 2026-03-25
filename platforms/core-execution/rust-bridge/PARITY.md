# Parity Status

## Module

- `module_id`: `core-execution-rust-bridge`
- `module_path`: `platforms/core-execution/rust-bridge`
- `status`: `jvm-compat`

## Supported

- Bridge protobuf contracts are synchronized from `substrate/proto/v1`.
- Core bridge source set compiles with current exclusion list.
- Bridge test classes compile as a compatibility gate.

## Gaps

- Some shadowing/compatibility paths are temporarily excluded while APIs are stabilized.
- Not all Gradle internal services are routed through Rust yet.

## Validation

- `./gradlew -q :rust-bridge:syncProtos`
- `./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses`
- `./tools/stabilization/run_strict_stabilization.sh quick`

## Next Sync Actions

1. Remove compile exclusions incrementally and replace them with parity-tested implementations.
2. Keep proto contracts versioned and synchronized per change.
3. Expand integration tests for daemon attach/relaunch and handshake compatibility.
