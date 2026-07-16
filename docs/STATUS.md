# Rust Substrate Status

Last updated: 2026-07-16
Branch: `rust-substrate`  
Scope: preview engine (not full Gradle parity)

## Product claim

Kotlin JVM library warm path: native `KotlinCompile` via `kotlinc` (not full Kotlin Gradle plugin parity).


Supported warm Java builds can:

1. Capture a typed build-plan through JVM configuration once
2. Admit the whole plan in Rust (fail-closed on unsupported semantics)
3. Execute via Rust `RunBuild` with **zero JVM task forwards**
4. Re-run from cached plan through `gradle-substrate-runbuild` without Gradle configuration

## Current gates (HEAD)

| Gate | Result |
| --- | --- |
| Upstream freshness | Merged; 0 behind `upstream/master` at last integrate |
| `cargo test -p gradle-substrate-daemon --lib` | Green (~1918 tests) |
| Full package cargo tests | Green when run via `tools/ci/run_rust_substrate_gates.sh` |
| Direct-warm dogfood (supported) | **8/8** zero JVM forwards |
| Full dogfood manifest | **9/9** (8 supported + 1 fail-closed) |
| Fail-closed dogfood entries | composite substitution + Kotlin JVM library |
| Beads actionable backlog | Empty of theater; milestone beads tracked |

### Full dogfood evidence command (supported + fail-closed)

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/manifest.json \
  --execute \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood-current
```

Latest observed: **9/9 matched** (8 supported zero-forward, 1 fail-closed).

### Direct-warm evidence command

```bash
cargo build -q -p gradle-substrate-daemon
python3 tools/dogfood_runner/direct_warm.py \
  --manifest testing/dogfood/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-binary target/debug/gradle-substrate-runbuild \
  --output-dir build/direct-warm-current
```

Supported projects (must stay green):

- `oss-style-java-library`
- `java-multiproject`
- `java-application`
- `external-junit-library`
- `external-bom-and-conflict`
- `javaexec-process-launch`
- `javadoc-process-launch`

Fail-closed (rejection is success):

- `unsupported-composite-substitution`
- `unsupported-kotlin-jvm-library`

## Installable under-test image

```bash
# Preferred full install (builds distribution into build/gradle-under-test)
./tools/install_gradle_under_test.sh

# Faster overlay when an install already exists
INSTALL_MODE=bridge-only ./tools/install_gradle_under_test.sh
```

Daemon discovery paths (see `DaemonLauncher.resolveBinary`):

- `build/gradle-under-test/lib/gradle-substrate-daemon`
- `build/gradle-under-test/lib/substrate/gradle-substrate-daemon-<os>-<arch>`

## CI

Workflow: `.github/workflows/rust-substrate-ci.yml`

- Linux/macOS cargo check, clippy (`-D warnings -A dead_code`), unit/integration tests
- `tools/ci/run_rust_substrate_gates.sh` runs full package tests + dogfood when under-test is present
- `CI_REQUIRE_DOGFOOD=1` forces dogfood; default CI skips dogfood only if under-test missing

Local:

```bash
bash tools/ci/run_rust_substrate_gates.sh
CI_REQUIRE_DOGFOOD=1 bash tools/ci/run_rust_substrate_gates.sh
```

## Architecture snapshot

- Rust daemon: gRPC over UDS/TCP (`substrate/`)
- JVM bridge: `platforms/core-execution/rust-bridge`
- Modes: off / shadow / authoritative kernel (`RustSubstrateOptions`)
- Native plugin ABI: builtin java / java-library / application contracts + dependency bucket projection
- Composite `includeBuild`: structured IR (`composite_ir`) with path-aware fail-closed diagnostics
- Kotlin `KotlinCompile`: precise fail-closed diagnostic (not yet executed natively)

## Roadmap (ordered)

1. **Keep gates green** — CI + dogfood 8/8 on every merge
2. **Kotlin vertical** — move from fail-closed to native `compileKotlin` worker contract
3. **Composite execution** — included-build task/classpath model beyond IR diagnostics
4. **Flag collapse** — public modes `off|shadow|kernel`; hide internal knobs
5. **God-file splits** — only when touching `dependency_resolution` / `task_graph` / `dag_executor`

## Non-goals (still)

- Arbitrary JVM plugin action bodies in Rust
- Full configuration/DSL evaluation in Rust
- Silent task-by-task JVM fallback after kernel admission
- Claiming full Gradle replacement

## How to demo

```bash
./tools/install_gradle_under_test.sh   # or INSTALL_MODE=bridge-only
cargo build -q -p gradle-substrate-daemon
./tools/demo/rust_substrate_demo.sh --quick
```

## Honesty rule

If a gate is not re-run on the current commit, do not claim it. Update this file only with commands and counts observed on HEAD.


## Public substrate modes

Prefer `-Dorg.gradle.rust.substrate.mode=`:

| Value | Meaning |
| --- | --- |
| `off` | Disabled |
| `shadow` | Capture/compare only |
| `kernel` | Native-ready authoritative execution (alias of legacy `authoritative`) |

Internal per-subsystem flags remain for CI/experimental matrices.
