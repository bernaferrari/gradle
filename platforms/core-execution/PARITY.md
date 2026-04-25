# Parity Status

## Module

- `module_id`: `platform-core-execution`
- `module_path`: `platforms/core-execution`
- `status`: `mixed`

## Supported

- `platforms/core-execution` is the active mixed-mode landing zone for the Rust substrate.
- The repository contains a Rust daemon workspace plus a JVM bridge module that keeps the
  current migration centered on execution, cache, hashing, snapshots, workers, and
  adjacent runtime services.
- The stabilization workflow documents the intended compile, regression, and end-to-end
  gates for this platform.
- The active JVM bridge now covers more than the original minimal subset: lifecycle
  listeners, cache registration, JVM-host project-model wiring, exec shadow classes,
  reflective test-execution shadowing, and the legacy `RustBridgeServices` compatibility
  shim are compile-safe and exercised by the strict stabilization gate.

## Gaps

- The canonical build-plan handoff is still thinner than Gradle's full execution contract.
- The active JVM bridge wiring is still narrower than the full set of bridge sources in
  the module.
- Upstream-sync metadata exists, but some modules still lack concrete last-synced commit
  information.

## Validation

- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses`

## Next Sync Actions

1. Fill in concrete parity updates and upstream sync commits.
2. Add module-specific differential tests where applicable.
3. Keep this file current on every sync PR.
