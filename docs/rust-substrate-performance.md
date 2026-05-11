# Rust Substrate Performance Evidence

Use the corpus runner first, then render a compact report:

```bash
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative \
  --tasks clean build \
  --timeout 300 \
  --output-dir build/corpus-authoritative-21

python3 tools/performance/rust_substrate_perf_report.py \
  build/corpus-authoritative-21/corpus_summary.json \
  --output build/corpus-authoritative-21/performance.md
```

The generated report records matched projects, no-fallback projects, task-count
parity, observed upstream wall time, observed Rust-substrate wall time, and the
percentage delta. Treat whole-build wall time as trend evidence; use dedicated
benchmarks for subsystem-level claims.
RunBuild corpus claims require `GRADLE_UNDER_TEST` or `--gradle-command` to
point at a local distribution built from this fork; the bootstrap wrapper alone
is not evidence that Rust executed the build.

For first-minute demo evidence, run the fast path first:

```bash
python3 tools/demo/first_60_seconds.py --mode fast --output build/first60.json
```

That reports daemon socket readiness, cold and warm authoritative Rust
`RunBuild` invocations of a small Java-library corpus project with Gradle
configuration-cache reuse, Rust up-to-date skip counts, Rust no-source/skipped
counts, and zero JVM task forwards, isolated real-build remote requests avoided when a local
Gradle-under-test install is present, listener static artifact prefetch for
direct and transitive artifacts in that installed build, and native file-watch
first-event latency.

Fast mode fails the command when the first-minute budgets regress: daemon socket
readiness must stay at or below 2000 ms; the authoritative Rust DAG check must
stay at or below 60000 ms total, with cold at or below 30000 ms, warm at or
below 10000 ms, warm faster than cold, and zero JVM forwards; dependency
read-through must avoid at least 3 first-run remote requests on the second run
and stay at or below 60000 ms; file-watch first event latency must stay at or
below 250 ms.

For the heavier proof harness, run:

```bash
python3 tools/demo/first_60_seconds.py --mode proof --output build/first60-proof.json
```

Proof mode adds Rust dependency-transport/store/checksum smoke timing,
cache-first reuse of Rust warm-cache and persisted transport bytes, URL
metadata-store warming through Rust transport, opt-in static Maven artifact
prefetch through Rust `ResolveDependencies`, and focused Gradle artifact/POM
read-through checks. Use `--mode all` when you want both the fast demo and the
proof checks in one run.
