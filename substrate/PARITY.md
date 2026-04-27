# Parity Status

## Module

- `module_id`: `substrate-daemon`
- `module_path`: `substrate`
- `status`: `native-kernel`

## Supported

- Rust daemon protocol services compile and run in-process tests.
- Hash compatibility suite validates cross-language digest behavior.
- Strict stabilization script validates proto sync and daemon smoke startup.
- Captured JVM-host build-plan shadows can drive a small native Rust Java
  lifecycle with JVM fallback disabled: `JavaCompile`, `ProcessResources`
  lowered to `Copy`, no-action `classes` lowered to `Lifecycle`, and `Jar`.

## Gaps

- Full dependency resolution semantics are not yet parity-complete.
- Real Gradle invocation still uses the JVM task engine as the outer executor;
  Rust no-fallback execution is proven through daemon/bridge contracts, not yet
  as the default `gradlew build` path.
- Kotlin/Groovy DSL evaluation and legacy plugin execution remain JVM-host
  compatibility islands.
- More task types need native-ready contract capture before broad no-fallback
  execution is realistic, especially richer `Copy`/`Sync` specs and `Test`.

## Validation

- `cargo check -p gradle-substrate-daemon`
- `cargo test -p gradle-substrate-daemon --test hash_compatibility_test`
- `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact`
- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./tools/demo/rust_substrate_demo.sh --quick`

## Next Sync Actions

1. Expand differential corpus coverage for dependency and task-graph semantics.
2. Wire real selected Gradle task graphs into `RunBuild` as an opt-in
   no-fallback execution command.
3. Add native-ready contracts for richer `Copy`/`Sync` specs and `Test` enough
   to complete broader Java library/application lifecycles.
4. Reduce bridge source exclusions as APIs are stabilized.
5. Track upstream commit synchronization in this file for each parity push.
