# Rust Substrate Performance Evidence

Use the corpus runner first, then render a compact report:

```bash
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
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
