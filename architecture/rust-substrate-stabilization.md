# Rust Substrate Stabilization (Strict Order)

This document defines the required order for stabilizing the Gradle Rust substrate integration.
Do not reorder these phases. A later phase is invalid unless all earlier phases pass.

## Phase 1: Protocol Source of Truth

Goal: prevent drift between Rust daemon protocol contracts and Java bridge contracts.

Required checks:

1. Run `:rust-bridge:syncProtos`.
2. Verify `substrate/proto/v1` and `platforms/core-execution/rust-bridge/src/main/proto/v1` are identical.
3. Keep canonical build-plan IR schema in `buildplan.proto` versioned and reviewed.
4. Validate `architecture/upstream-map/proto-lock.sha256` against current proto contents.
5. Validate `architecture/upstream-map/modules.toml` and required module metadata files.

Exit criteria:

1. No proto diffs.
2. Proto sync task succeeds from a clean checkout.
3. Build-plan IR golden fixture fingerprint tests pass.
4. Proto lock fingerprint check passes.
5. Upstream map validation passes with no metadata drift.

## Phase 2: Java Compatibility Compile Gate

Goal: keep bridge compatibility shims compiling before touching daemon behavior.

Required checks:

1. `./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses`

Exit criteria:

1. Bridge main and test source sets compile.

## Phase 3: Rust Daemon Type/Link Integrity

Goal: ensure core daemon code compiles with current protocol and API surfaces.

Required checks:

1. `cargo check -p gradle-substrate-daemon`

Exit criteria:

1. No compile errors in daemon crate.

## Phase 4: Critical Correctness Regressions

Goal: protect high-risk behavior where fragility has already been observed.

Required checks:

1. Work input hash must include keys and values.
2. Configuration cache validation must be order-insensitive for input hash lists.
3. Build layout must reject project paths outside build root even when string prefixes overlap.
4. Cross-language hash compatibility tests must pass.
5. Build-plan shadow differential checks must confirm order-insensitive JVM-model capture
   and per-build artifact partitioning.

Exit criteria:

1. Regression tests for these cases pass.

## Phase 5: End-to-End Process Health

Goal: verify daemon startup/lifecycle behavior in a release-like binary.

Required checks:

1. Build release daemon binary.
2. Run end-to-end smoke test against a real Unix socket daemon process.
3. Verify JVM-host-derived build-plan shadow artifacts can be persisted under the
   Rust configuration-cache directory.
4. Verify JVM-host model/environment and persisted shadow artifacts compare cleanly
   via the differential shadow-plan checks.

Exit criteria:

1. Daemon starts, remains healthy, and passes smoke checks.
2. Build plan shadow capture path is exercised by tests.
3. Shadow differential checks report no mismatches for non-tampered flows.

## One Command

Run strict sequence via:

```bash
./tools/stabilization/run_strict_stabilization.sh full
```

Quick mode (without release smoke):

```bash
./tools/stabilization/run_strict_stabilization.sh quick
```

## CI Enforcement (Mandatory)

The strict quick sequence is enforced on every push and pull request to `master` via:

`/.github/workflows/rust-substrate-drift-check.yml`

That workflow runs:

```bash
./tools/stabilization/run_strict_stabilization.sh quick
```

Any failure in drift checks, bridge compile, daemon type-check, or critical regression tests must block merge.

Run only upstream drift checks:

```bash
./tools/upstream_map/check_drift.sh
```
