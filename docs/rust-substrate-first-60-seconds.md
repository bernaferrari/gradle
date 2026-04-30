# Rust Substrate: First 60 Seconds

The visible wins should be things people can feel immediately, before they
inspect a profiler:

- daemon readiness: how quickly the Rust sidecar can accept work
- dependency transport: whether Maven artifact bytes stream through Rust fast and land in the Rust store with checksum evidence
- dependency read-through: whether Gradle can skip remote artifact access when Rust already has the JAR
- file watching: how quickly an edit becomes observable

Run:

```bash
python3 tools/demo/first_60_seconds.py --output build/first60.json
```

The script reports:

- `daemon_socket_ready`: process launch until the Unix socket exists
- `dependency_transport_store_checksum`: local HTTP artifact streaming through the Rust dependency service, persisted store write, cache hit, and checksum verification
- `dependency_artifact_readthrough`: focused Gradle resolver test proving the Rust read-through hook resolves before remote access
- `file_watch_first_event`: native file watcher latency from write to event

These are not full Gradle replacement claims. Gradle DSL and unsupported plugin
semantics still go through the JVM compatibility island. This harness exists to
keep the Rust work focused on perceptible first-minute wins while the broader
authoritative corpus continues to guard correctness.
