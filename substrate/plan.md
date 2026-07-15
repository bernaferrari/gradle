# Substrate roadmap

Short live roadmap for the Rust substrate. Long-form strategy, architecture
target, and phase narrative:

**[`docs/rust-substrate-turbopack-plan.md`](../docs/rust-substrate-turbopack-plan.md)**

Also:

- Product contract / matrix: [`docs/rust-substrate-preview.md`](../docs/rust-substrate-preview.md)
- Parity snapshot: [`PARITY.md`](PARITY.md)
- Warm-path detail: [`WARM_PATH_ROADMAP.md`](WARM_PATH_ROADMAP.md)
- Dogfood: [`docs/rust-substrate-dogfood.md`](../docs/rust-substrate-dogfood.md)

## Goal

Gradle-compatible front door; **Rust owns the hot engine path** for supported
builds. JVM stays the compatibility frontend for DSL, plugins, and unsupported
semantics—measured and isolated, not hidden fallback after kernel admission.

Engine modes that matter:

- `rust-hot-path-direct-warm` — valid cached graph, no JVM configuration, no JVM task forwards  
- `rust-hot-path-via-compat-frontend` — JVM captured the graph; Rust executed with zero forwards  
- `jvm-compatibility-frontend-fail-closed` — unsupported; reject rather than approximate  
- `jvm-compatibility-runtime` — legacy JVM execution still required  

Metric: **native no-forward builds**, not service count.

## Phases done (checkpoint summary)

From the turbopack plan completion notes (supported Java preview slice unless noted):

| Phase | Outcome |
| --- | --- |
| **0 – Gates green** | Bridge/substrate changeable; flags explicit; focused tests kept green |
| **1 – FS ownership** | Watch, snapshots, hashing, file-hash cache, tree ops; Copy/Sync/Delete/archives native on admitted contracts; strict kernel disables JVM forwards |
| **2 – Execution verticals** | JavaCompile, resources/copy/sync, archives, Test, Exec, JavaExec, Javadoc, start scripts, lifecycle, static reports; up-to-date + local cache; dogfood zero-forward on supported set |
| **3 – Warm Rust-first** | `warm_runner` + `gradle-substrate-runbuild`; fingerprint invalidation; direct warm dogfood across supported local projects |
| **4 – Dep resolution (progress)** | Bounded Maven/GMM fixtures; fail-closed ambiguous artifacts and unsupported GMM fields; external corpus graph parity on supported cases |
| **5 – Configuration IR (progress)** | `CanonicalConfigurationGraph` beside shadow; input/env invalidation; native replay only for supported built-in plugin config |
| **6 – Plugin ABI (progress)** | Native contracts for base/java/java-library/application; external plugins fail closed |
| **7 – Shrink JVM shell (progress)** | Rust-primary daemon launch; endpoint launch-mode metadata; wrapper opt-in direct warm before Gradle |
| **8+ – Stable identity / further** | Ongoing; see turbopack plan |

Preview is **showable and measured**, not universal Gradle replacement.

## Next priorities

1. **Warm-path excellence**  
   Endpoint freshness / binary identity in `runbuild`, dry-run plan validation,
   first-class demo/dogfood short commands, publish cold→warm timing vs full
   Gradle. Details: `WARM_PATH_ROADMAP.md` Tier 1.

2. **Expand native-ready surface only with fixtures**  
   Additional CopySpec/filter/archive/Test shapes that appear in real corpus or
   dogfood—and only with authoritative zero-forward evidence. Never approximate.

3. **Dependency resolution vertical**  
   Finish common Maven/Ivy + GMM paths that unblock real projects; keep
   fail-closed for rules/transforms/composites until modeled. Promote only with
   external corpus + dogfood proof.

4. **Configuration IR → less JVM on warm hits**  
   Broaden safe native configuration replay for built-ins; tighten invalidation;
   keep external plugins on the compatibility island or explicit reject.

5. **Hygiene for reviewability**  
   Strip session-log noise from headers/docs, quarantine generated noise that
   blocks review, keep focused `cargo test` / bridge / corpus gates green after
   upstream merges. Prefer vertical authority over new shadow scaffolding.

## Non-goals

- Full class-by-class Gradle rewrite on a deadline  
- Perfect Groovy AST as a blocker (string parser is the production path for now)  
- Arbitrary task actions or rich unmodeled plugin behavior in Rust  
- Silent task-by-task JVM fallback after authoritative admission  
- Treating “more services” or shadow reporters as done without no-forward proof  
- Full dependency-resolution or composite-build parity before IR and fixtures exist  

## Working rules

- **Shadow → authoritative:** land comparison first; promote only with corpus/dogfood evidence.  
- **Fail closed:** unsupported diagnostics beat approximate success.  
- **Additive contracts:** extend protos/IR carefully; version durable state.  
- **Evidence:** command + artifact path when claiming a gate; update `PARITY.md` for durable status only.  
- **Scope:** one vertical slice at a time; keep behavior of the Rust engine stable unless the ticket says otherwise.

## Quick validation

```bash
cargo test -p gradle-substrate-daemon --lib
python3 tools/corpus_runner/run.py \
  --manifest testing/corpus/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-authoritative --tasks clean build --timeout 300
python3 tools/dogfood_runner/direct_warm.py \
  --manifest testing/dogfood/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-binary target/debug/gradle-substrate-runbuild \
  --output-dir build/direct-warm-dogfood-current
tools/demo/rust_substrate_demo.sh --quick
```

Full command set: `PARITY.md` and `docs/rust-substrate-preview.md`.
