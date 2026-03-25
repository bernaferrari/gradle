# Parity Status

## Module

- `module_id`: `platform-core-execution`
- `module_path`: `platforms/core-execution`
- `status`: `mixed`

## Supported

- Document behavior that currently matches upstream.

## Gaps

- Document known behavior gaps against upstream.

## Validation

- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses`

## Next Sync Actions

1. Fill in concrete parity updates and upstream sync commits.
2. Add module-specific differential tests where applicable.
3. Keep this file current on every sync PR.
