# Upstream Map

This directory is the source of truth for Rust migration syncability metadata.

- `modules.toml`: tracked migration modules, owners, upstream path mappings, parity commands.
- `templates/`: required structure for module-level `UPSTREAM.md` and `PARITY.md`.

Validation command:

```bash
python3 tools/upstream_map/validate_map.py
```

Scaffold missing module metadata:

```bash
python3 tools/upstream_map/sync_metadata.py
```

Discover unmapped top-level modules:

```bash
python3 tools/upstream_map/discover_modules.py
```

Validate proto contract fingerprint lock:

```bash
python3 tools/upstream_map/check_proto_lock.py
```

Update proto contract fingerprint lock after intentional proto changes:

```bash
python3 tools/upstream_map/check_proto_lock.py --update
```

Full drift check (proto + map + metadata):

```bash
./tools/upstream_map/check_drift.sh
```

Pull and rebase from upstream with automatic safety + validation:

```bash
./tools/upstream_map/sync_from_origin.sh origin/master quick
```

Disable bead tracking for a run:

```bash
BD_TRACK=0 ./tools/upstream_map/sync_from_origin.sh origin/master quick
```
