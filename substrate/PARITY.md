# Parity Status

## Module

- `module_id`: `substrate-daemon`
- `module_path`: `substrate`
- `status`: `native-kernel`

## Supported

- Rust daemon protocol services compile and run in-process tests.
- Hash compatibility suite validates cross-language digest behavior.
- Strict stabilization script validates proto sync and daemon smoke startup.

## Gaps

- Full dependency resolution semantics are not yet parity-complete.
- Some JVM bridge integration paths are still excluded while APIs settle.

## Validation

- `cargo check -p gradle-substrate-daemon`
- `cargo test -p gradle-substrate-daemon --test hash_compatibility_test`
- `./tools/stabilization/run_strict_stabilization.sh quick`

## Next Sync Actions

1. Expand differential corpus coverage for dependency and task-graph semantics.
2. Reduce bridge source exclusions as APIs are stabilized.
3. Track upstream commit synchronization in this file for each parity push.
