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

For first-minute demo evidence, run:

```bash
python3 tools/demo/first_60_seconds.py --output build/first60.json
```

That reports daemon socket readiness, Rust dependency-transport/store/checksum
smoke timing, URL metadata-store warming through Rust transport, opt-in static
Maven artifact prefetch through Rust `ResolveDependencies`, Gradle artifact/POM
read-through from the Rust cache, isolated real-build remote requests avoided
when a local Gradle-under-test install is present, listener static artifact
prefetch for direct and transitive artifacts in that installed build, and
native file-watch first-event latency without relying on full-build timing. The
Gradle
read-through checks share one focused test invocation so the demo stays inside
the first minute while still proving artifact, metadata, and uncached-download
Rust seams.
