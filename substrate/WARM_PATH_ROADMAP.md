# Rust Substrate Warm / Cached-Plan Execution Roadmap

**Status:** Endpoint auto-discovery and stale daemon-binary identity validation
are implemented in `runbuild.rs`. Dry-run plan validation remains outstanding.
This status was verified statically; the added focused tests were not run in
this documentation pass.

## Why This Matters (Highest ROI per my assessment)
- The unique capability no one else has: after one JVM configuration pass that produces a validated native-ready build-plan shadow, Rust can execute the entire DAG with zero JVM forwards, from a tiny daemon, with strong content fingerprint invalidation.
- This is the "first 60 seconds" + repeated dogfood / CI / developer inner-loop killer feature.

## Implemented
1. **P1 Bug Fixed** (gradle-fork-6bfn)
   - Unified HASH file fingerprint entry ID generation through a single canonical `hash_only_path(raw_content_md5_bytes)` helper.
   - Single-file vs directory-child paths now guaranteed identical for the same content.
   - A dedicated regression test and comments are checked in; the test was not
     run in this documentation pass.

2. **Major Warm-Path Usability Win**
   - `gradle-substrate-runbuild` now supports auto-discovery of the daemon TCP endpoint from `state_dir/substrate.tcp-endpoint` (or `state/state/...` layout).
   - `--endpoint` is now optional when `--state-dir` is supplied.
   - New `--no-endpoint-discovery` escape hatch.
   - Discovered endpoint files are validated before connection: the persisted
     daemon binary must still exist and match its recorded modification time
     and size, otherwise runbuild fails closed with a stale-identity error.
   - Focused unit tests cover explicit-endpoint precedence, both discovery
     layouts, stale binary identity rejection, and the discovery opt-out. They
     were added but not run in this documentation pass.
   - Example new short command:
     ```
     target/debug/gradle-substrate-runbuild \
       --state-dir build/my-dogfood-state \
       --project-dir testing/corpus/oss-style-java-library-kotlin-dsl \
       --max-parallelism 8
     ```

## Concrete Next Steps (Prioritized)

**Note (parallel slices)**: Dependency Graph slice (dependency_solver/resolved_graph + hot path wiring in dependency_resolution.rs) first milestone delivered per plan (edge materialization + selection ownership live, "resolved-graph" reporter active). See plan.md.

### Tier 1 — Make Warm Path *Excellent* (do these next)
- [x] Auto-discover persisted endpoints and fail closed when the recorded daemon
  binary identity is stale (`runbuild.rs`; focused tests added but not run this
  pass).
- [ ] Add `--dry-run` / plan validation mode to runbuild (load artifact + fingerprints, print what would run, exit without contacting daemon).
- [ ] First-class support in `tools/demo/` and `tools/dogfood_runner/` for the short runbuild form (update direct_warm.py etc.).
- [ ] Document the "one Gradle run → many pure-Rust warm executions" story prominently in substrate/README.md and docs/rust-substrate-preview.md.
- [ ] Measure + publish "time from project dir change to first task start with warm runbuild" vs full Gradle daemon.

### Tier 2 — Expand Native-Ready Surface (high leverage, stay disciplined)
- Focus only on shapes that appear in real OSS dogfood or the checked-in corpus and that unlock zero-fallback runs.
- Examples worth considering (only after a corpus fixture + authoritative evidence gate):
  - More CopySpec `eachFile` static rewrite forms (already partially supported).
  - Common literal `filter` / `expand` cases beyond simple replace.
  - Additional archive metadata (manifest attributes, etc.) for Jar/War if they block real projects.
  - One or two more TestExec filter shapes if they appear in external corpus.
- Never add "approximate" support. Always fail-closed + precise diagnostic.

### Tier 3 — Polish & Hardening (do after Tier 1+2 show user delight)
- Richer diagnostics in runbuild on fingerprint invalidation (which exact input changed + why).
- Stable build identity that survives minor Gradle version bumps (so cached plans remain usable longer).
- Optional "plan server" mode or integration with Gradle configuration cache so a warm plan can be the default for repeated `clean build` etc.

## Anti-Goals (Do Not Do)
- Full dependency resolution parity in Rust for the warm path (trap).
- Making the Groovy AST parser perfect (current string-based extraction is sufficient and reliable for the narrow contract we need).
- Arbitrary task actions or rich plugin modeling.

## Evidence Gates (never skip)
For any expansion:
1. JVM bridge capture + "native-ready" marker for the exact shape.
2. Rust admission + executor support.
3. Focused unit + bridge test.
4. At least one authoritative corpus run (or dogfood) with zero JVM forwards + output/hash/archive parity.
5. Update PARITY.md + close Beads issue with command + numbers.

## Commands to Drive Progress
```bash
# Fast iteration on warm path
cargo build -p gradle-substrate-daemon --bin gradle-substrate-runbuild

# Existing strong validation
python3 tools/corpus_runner/run.py --manifest ... --runbuild-authoritative ...

# Direct warm (after one Gradle run has populated the shadow)
target/debug/gradle-substrate-runbuild --state-dir ... --project-dir ...
```

This roadmap keeps the project on the extremely disciplined, evidence-based path that has made the substrate as solid as it is today.
