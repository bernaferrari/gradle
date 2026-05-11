# Contributing to the Rust Substrate

## Setup

```bash
# Install Rust stable
rustup install stable
rustup default stable

# Build
cd substrate
cargo build

# Test
cargo test
cargo clippy
```

## Code Conventions

### Shared State
Use `Arc<DashMap<...>>` for services shared between dispatchers and tonic. Note that `DashMap::clone()` is a deep copy — use `Arc` to share.

### Scope Newtypes
Use typed newtypes for IDs in `scopes.rs`: `BuildId`, `SessionId`, `TreeId`, `ProjectPath`.

### Proto Mapping
- Proto `int64` → Java `long`
- Proto `int32` → Java `int`
- Proto `bytes` → Java `ByteString`
- Use `google.protobuf.Timestamp` for timestamps, not custom int64 fields

### Integration Tests
Use fully qualified proto clients: `foo_service_client::FooServiceClient::new(channel)`.

### Sorting
Use `sort_unstable` / `sort_unstable_by_key` instead of `sort` / `sort_by` — faster and sufficient when equal elements don't need to preserve original order.

### Error Handling
Use `SubstrateError` from `error.rs` instead of `String` errors in service implementations.

## Adding a New Service

1. **Define proto** — Add `substrate/proto/v1/your_service.proto` with service and messages
2. **Register proto** — Add `"proto/v1/your_service.proto"` to `substrate/build.rs`
3. **Implement server** — Create `substrate/src/server/your_service.rs` implementing the generated trait
4. **Register module** — Add `pub mod your_service;` to `substrate/src/server/mod.rs`
5. **Wire into main** — Add `.add_service(YourServiceServer::new(your_service))` to `substrate/src/main.rs`
6. **Add subsystem** — If authoritative: add field to `SubsystemModes` in `authoritative.rs`, update `control.rs`
7. **Add tests** — Unit tests in `your_service.rs`, integration test in `tests/integration_test.rs`
8. **Sync to Java** — Run `./gradlew :rust-bridge:syncProtos`
9. **Update proto metadata** — Run `python3 tools/upstream_map/check_proto_lock.py --update` and `python3 tools/upstream_map/proto_version.py --update`

## Shadow Mode

Subsystems run through: off → shadow → authoritative.

Enable shadow mode: `-Dorg.gradle.rust.substrate.<subsystem>.shadow=true`
Flip to authoritative: `-Dorg.gradle.rust.substrate.<subsystem>.authoritative=true`

## Proto Change Workflow

1. Modify proto file in `substrate/proto/v1/`
2. Run `cargo build` to regenerate Rust types
3. Update server implementation to match new proto
4. Run `./gradlew :rust-bridge:syncProtos` to sync Java bridge
5. Run `python3 tools/upstream_map/check_proto_lock.py --update`
6. Run `python3 tools/upstream_map/proto_version.py --update`
7. Verify: `./tools/upstream_map/check_drift.sh`, focused tests, `cargo test`, `cargo clippy`

See `architecture/rust-substrate-maintenance.md` for schema compatibility,
supported semantic slice, unsupported fail-closed gate, PARITY, and Beads
evidence rules.

## Commit Conventions

- Keep commits focused and atomic
- Reference issue IDs when applicable
- Run `cargo clippy` before committing — must be clean
- Run `cargo test --lib` before committing — all tests must pass
