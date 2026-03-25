# Upstream Mapping

## Module

- `module_id`: `substrate-daemon`
- `module_path`: `substrate`
- `owner`: `substrate`

## Upstream Source Paths

- `platforms/core-execution/execution`
- `platforms/core-execution/file-watching`
- `platforms/core-execution/hashing`
- `platforms/core-execution/persistent-cache`
- `platforms/core-execution/scoped-persistent-cache`
- `platforms/core-configuration/configuration-cache`

## Last Synced Upstream Commit

- `commit`: _pending_
- `sync_date_utc`: _pending_

## Sync Notes

- This module is the Rust daemon kernel and does not map 1:1 to one upstream Gradle project.
- Keep behavior parity through differential tests and protocol compatibility tests.
