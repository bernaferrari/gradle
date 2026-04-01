# Migration Status: rust-bridge

**Status:** `mixed`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Complete)
- Original Gradle execution with no Rust connectivity
- All services running on JVM with no external daemon

### Phase 2: Mixed (Current)
- gRPC bridge clients for all 38 substrate services
- `JvmHostService` server implementation for Rust→JVM callbacks
- Proto contracts synchronized from `substrate/proto/v1`
- Unix domain socket connectivity to substrate daemon
- Feature flags via `-Dorg.gradle.rust.substrate.*` system properties
- Some shadowing/compatibility paths temporarily excluded while APIs stabilize
- Strict stabilization script validates proto sync and bridge compatibility

### Phase 3: Native (Target)
- Bridge clients become thin pass-through as services flip to authoritative
- `JvmHostService` remains as long-term JVM callback channel
- Bridge module shrinks as more services migrate fully to Rust
- Ultimate goal: minimal bridge layer for JVM-only subsystems

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| Bridge clients (38 services) | Substrate services | shadow | All clients implemented, not all authoritative |
| `JvmHostService` server | `JvmHostService` client | authoritative | Callback protocol stable |
| Proto sync | `substrate/proto/v1` | authoritative | Contracts synchronized |
| Feature flags | `RustSubstrateOptions` | authoritative | Per-service toggle |

## Blockers

- Compile exclusions temporarily disable some bridge paths
- Not all Gradle internal services routed through Rust yet
- API stabilization needed before removing exclusions
- Daemon attach/relaunch handshake needs more integration testing

## Next Steps

1. Remove compile exclusions incrementally and replace with parity-tested implementations
2. Keep proto contracts versioned and synchronized per change
3. Expand integration tests for daemon attach/relaunch and handshake compatibility
4. Flip individual services to authoritative mode after shadow validation passes
