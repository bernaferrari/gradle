# Parity Status

## Module

- `module_id`: `core-execution-rust-bridge`
- `module_path`: `platforms/core-execution/rust-bridge`
- `status`: `jvm-compat`

## Supported

- Bridge protobuf contracts are sourced from `substrate/proto/v1`, and the checked-in
  bridge tree is expected to stay byte-for-byte aligned with that source.
- The active service descriptor now registers `RustBridgeCoreServices` plus
  `RustBridgeCacheServices`.
- The active mixed-mode wiring includes daemon launch/connect, optional JVM-host attach,
  build result/bootstrap/mismatch lifecycle listeners, cache orchestration, dependency
  resolution, configuration-cache shadowing, reflective project-model handoff, and
  reflective test-execution listener registration.
- The exec shadow path (`ShadowingExecActionFactory`, `RustExecAction`, `RustProcessHandle`)
  is now back in the compile-safe bridge surface.
- The legacy `RustBridgeServices` type is now a compile-safe compatibility shim over the
  validated core service layer instead of a stale duplicated registrar.
- The bridge compile/test gate validates the active service layer, proto-generated
  classes, and the full `:rust-bridge:test` suite in strict full stabilization.

## Gaps

- Some runtime parity specs still need stronger harnesses than direct final-stub mocks,
  especially around gRPC-backed cache behavior.
- The bridge is not yet a full compatibility layer; some services remain shadow-only and
  not every bridge source is active in the service descriptor.
- Validation currently depends on Gradle/plugin resolution succeeding before the compile
  gate can run end-to-end.

## Validation

- `./gradlew -q :rust-bridge:syncProtos`
- `./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses`
- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./tools/stabilization/run_strict_stabilization.sh full`

## Next Sync Actions

1. Remove compile exclusions incrementally and replace them with parity-tested implementations.
2. Keep proto contracts versioned and synchronized per change.
3. Expand integration tests for daemon attach/relaunch and handshake compatibility.
