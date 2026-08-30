# Rust Substrate Status

Last documented evidence: 2026-07-16 at `7eabe3f`
Branch: `rust-substrate`
Scope: preview engine (not full Gradle parity)

The current rebased worktree has **not** been built or tested. The evidence
below is the last documented pre-rebase result, not a claim about current HEAD.

## Product claim

Supported warm builds can:

1. Capture a typed build-plan through JVM configuration once
2. Admit the whole plan in Rust (fail-closed on unsupported semantics)
3. Execute via Rust `RunBuild` with **zero JVM task forwards**
4. Re-run from cached plan through `gradle-substrate-runbuild` without Gradle configuration

An in-flight Kotlin JVM library vertical uses a fixture-backed `KotlinCompile`
executor that launches the host `kotlinc`. It is intentionally narrower than
Kotlin Gradle plugin parity: Kotlin build logic and compiler plugins remain
unsupported. Its dogfood target exempts `*.class` and `*.kotlin_module` content
hashes and checks archives by entry inventory only. The rebased implementation
has not been re-verified.

Settings-level `includeBuild` substitution remains fail-closed. Materialized
classpath files do not represent included-build producer tasks or cross-build
dependency edges, so they are not sufficient grounds for admission.

## Evidence and current manifest

| Evidence scope | Result |
| --- | --- |
| Last documented pre-rebase direct-warm dogfood at `7eabe3f` (2026-07-16) | **8/8** supported entries, zero JVM forwards |
| Last documented pre-rebase full manifest at `7eabe3f` (2026-07-16) | **9/9** total entries: 8 supported + 1 fail-closed |
| Current rebased worktree | **Unverified**; no build or test was run during this pass |

The checked-in manifest currently declares 8 supported targets and 2
fail-closed targets. Those counts are derived from
`testing/dogfood/manifest.json`; they are contract inventory, not execution
results. In particular, the historical 9/9 result is not evidence for the
current 10-entry manifest.

### Full dogfood evidence command

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/manifest.json \
  --execute \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood-current
```

### Direct-warm evidence command

```bash
cargo build -q -p gradle-substrate-daemon
python3 tools/dogfood_runner/direct_warm.py \
  --manifest testing/dogfood/manifest.json \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --runbuild-binary target/debug/gradle-substrate-runbuild \
  --output-dir build/direct-warm-current
```

Supported targets in the current manifest (unverified after rebase):

- `oss-style-java-library`
- `java-multiproject`
- `java-application`
- `external-junit-library`
- `external-bom-and-conflict`
- `javaexec-process-launch`
- `javadoc-process-launch`
- `kotlin-jvm-library` (in-flight, fixture-backed host-`kotlinc` vertical)

Fail-closed (rejection is success):

- `unsupported-composite-substitution`
- `unsupported-component-metadata-rule`

`testing/dogfood/manifest-composite-only.json` retains composite substitution
as a focused experimental fail-closed gate; it is not a support gate.

## Installable under-test image

```bash
./tools/install_gradle_under_test.sh
INSTALL_MODE=bridge-only ./tools/install_gradle_under_test.sh
```

## CI

Workflow: `.github/workflows/rust-substrate-ci.yml`

- cargo check/clippy/tests
- `tools/ci/run_rust_substrate_gates.sh`
- optional `workflow_dispatch` install-under-test job

## Public substrate modes

Prefer `-Dorg.gradle.rust.substrate.mode=`:

| Value | Meaning |
| --- | --- |
| `off` | Disabled |
| `shadow` | Capture/compare only |
| `kernel` | Native-ready authoritative execution (alias of legacy `authoritative`) |

Internal per-subsystem flags remain for CI/experimental matrices.

## Architecture snapshot

- Rust daemon: gRPC (`substrate/`)
- JVM bridge: `platforms/core-execution/rust-bridge`
- Native plugin ABI + dependency projection
- Composite marker capture and path-aware diagnostics; settings-level
  `includeBuild` remains fail-closed until included producers and cross-build
  edges are represented
- In-flight fixture-backed `KotlinCompile` executor via host `kotlinc`

## Roadmap (ordered)

1. Re-run the manifest-derived gates when builds are allowed and record evidence
   for the rebased tree
2. Close or document Kotlin byte-parity gaps before widening beyond the single
   fixture-backed host-`kotlinc` vertical
3. Promote composite only when included-build producers and cross-build edges
   are reliably represented in the admitted DAG
4. God-file splits only when touching hot modules

## Non-goals

- Arbitrary JVM plugin action bodies in Rust
- Full configuration/DSL evaluation in Rust
- Silent task-by-task JVM fallback after kernel admission
- Claiming full Gradle replacement

## Honesty rule

If a gate is not re-run on the current commit, do not claim it.
