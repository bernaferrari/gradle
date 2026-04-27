# Rust Substrate Demo

This demo is intentionally narrow and repeatable. It shows the Rust substrate
as a hardened mixed-mode kernel, not as a complete Gradle replacement.

## Quick Demo

```bash
tools/demo/rust_substrate_demo.sh --quick
```

This runs the strict quick gate, validates the checked-in offline corpus,
builds the Java library/application samples, and runs the Rust daemon gRPC e2e
tests.

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
- `testing/corpus/` gives two small real builds: a Kotlin DSL Java library and
  a Groovy DSL Java application.
- `cargo test -p gradle-substrate-daemon --test e2e_grpc_test` exercises daemon
  gRPC behavior directly.

## Current Limits

- The demo does not claim full no-fallback Gradle execution.
- DSL evaluation and legacy plugin semantics remain JVM compatibility work.
- The corpus contract gate validates build-plan signals; full upstream-vs-Rust
  output parity still depends on running the corpus with a real substrate daemon
  binary and non-noop bridge mode.
- The Rust-owned path is strongest today around protocol stability, hashing,
  cache/config-cache support, build-plan shadowing, fail-closed bridge clients,
  and daemon/service behavior covered by the strict gate.
