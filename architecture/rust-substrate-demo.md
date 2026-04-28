# Rust Substrate Demo

This demo is intentionally narrow and repeatable. It shows the Rust substrate
as a hardened mixed-mode kernel, not as a complete Gradle replacement.

## Quick Demo

```bash
tools/demo/rust_substrate_demo.sh --quick
```

This runs the strict quick gate, validates the checked-in offline corpus,
builds the checked-in offline corpus samples, runs the no-fallback authoritative
RunBuild corpus gate, and runs the Rust daemon gRPC e2e tests.

## Public Demo Gate

```bash
tools/demo/rust_substrate_demo.sh --full
```

`--full` runs the full stabilization mode, including release daemon build and
release smoke coverage from `substrate/scripts/e2e-smoke-test.sh`.

## What To Show

- `tools/stabilization/run_strict_stabilization.sh` proves protocol drift,
  fail-closed bridge behavior, Java bridge compilation, Rust type-checking, and
  focused Rust regression tests.
- `tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --contract-only`
  proves the checked-in corpus contracts are deterministic without network
  access.
- `testing/corpus/` gives small real builds covering Kotlin/Groovy DSL Java
  projects, a multi-project application, Java resource expansion, JavaCompile
  options, standalone Copy/Sync transforms, CopySpec duplicate handling, and
  Zip/Tar archive tasks.
- `tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative`
  proves the checked-in corpus can run through the explicit no-fallback
  RunBuild gate and compare stable output inventories plus non-archive content
  hashes.
- `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact`
  proves a captured Java lifecycle plan can run with JVM fallback disabled.
- `cargo test -p gradle-substrate-daemon --test e2e_grpc_test` exercises daemon
  gRPC behavior directly.

## Current Limits

- The demo does not claim full no-fallback Gradle execution beyond the covered
  Java lifecycle, file operation, archive, test, and resource-expansion gates.
- DSL evaluation and legacy plugin semantics remain JVM compatibility work.
- Archive byte-for-byte parity, arbitrary CopySpec actions, and broad
  dependency-resolution semantics remain outside the demo claim.
- The Rust-owned path is strongest today around protocol stability, hashing,
  cache/config-cache support, build-plan shadowing, file transforms, Java
  lifecycle execution, archive packaging, test execution, fail-closed bridge
  clients, and daemon/service behavior covered by the strict gate.
