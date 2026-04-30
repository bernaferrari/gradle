# Rust Substrate: First 60 Seconds

The visible wins should be things people can feel immediately, before they
inspect a profiler:

- daemon readiness: how quickly the Rust sidecar can accept work
- dependency transport: whether Maven artifact bytes stream through Rust fast
- file watching: how quickly an edit becomes observable

Run:

```bash
python3 tools/demo/first_60_seconds.py --output build/first60.json
```

The script reports:

- `daemon_socket_ready`: process launch until the Unix socket exists
- `dependency_transport_http_stream`: local HTTP artifact streaming through the Rust dependency service
- `file_watch_first_event`: native file watcher latency from write to event

These are not full Gradle replacement claims. Gradle DSL and unsupported plugin
semantics still go through the JVM compatibility island. This harness exists to
keep the Rust work focused on perceptible first-minute wins while the broader
authoritative corpus continues to guard correctness.
