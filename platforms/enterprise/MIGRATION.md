# Migration Status: enterprise

**Status:** `jvm`  
**Upstream:** See `UPSTREAM.md`  
**Parity:** See `PARITY.md`

## Migration Phases

### Phase 1: JVM (Current)
- All components running entirely on JVM
- Build Scan publication and data collection
- `DevelocityPlugin` integration
- Build cache remote node connectivity
- Test distribution protocol

### Phase 2: Mixed (Not Started)
- Planned: build scan data collection
- Data aggregation is pure computation, good first candidate
- Requires stable build event protocol over gRPC

### Phase 3: Native (Target)
- Build scan data aggregation
- Build cache protocol client
- Remaining JVM dependencies: HTTP publication, authentication, test distribution agent

## Service Mapping

| JVM Service | Rust Service | Status | Notes |
|-------------|-------------|--------|-------|
| `BuildScanCollector` | — | pending | Data aggregation is computation |
| `BuildCacheClient` | — | pending | HTTP client needed |
| `TestDistribution` | — | jvm-only | Requires agent on remote nodes |
| `DevelocityPlugin` | — | jvm-only | Plugin lifecycle is JVM |

## Blockers

- HTTP publication requires TLS and authentication
- Test distribution requires JVM agents on remote executors
- Build scan format is proprietary and versioned

## Next Steps

1. Define build event collection protocol
2. Implement build scan data aggregation in Rust
3. Shadow data collection against JVM baseline
