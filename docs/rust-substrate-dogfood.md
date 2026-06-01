# Rust Substrate Dogfood Report

This report is the next evidence layer after the curated corpus. It is meant to
answer whether the Rust substrate is showable on realistic Gradle build shapes,
not whether it is a full Gradle replacement.

## Command

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/manifest.json \
  --execute \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood-current \
  --verbose
```

The runner writes:

- `build/dogfood-current/dogfood-results.json`
- `build/dogfood-current/dogfood-summary.md`
- `build/dogfood-current/<project>/result.json`

## Current Evidence

Latest checked run: 2026-06-01.

| Project | Expectation | Mode | Result | What It Proves |
| --- | --- | --- | --- | --- |
| `oss-style-java-library` | supported | strict | pass | Standard Java library lifecycle with sources/Javadoc/report-like outputs can run through the Rust kernel. |
| `java-multiproject` | supported | strict | pass | Rust DAG admission and execution handle a small multi-project Java build. |
| `external-junit-library` | supported | strict | pass | Pinned external dependencies plus JUnit Platform execution remain no-fallback in the supported slice. |
| `external-bom-and-conflict` | supported | strict | pass | Bounded Maven BOM/platform dependency semantics remain admitted in the dogfood path. |
| `javaexec-process-launch` | supported | strict | pass | Rust DAG execution covers Java compilation plus native `JavaExec` process launch with declared output parity. |
| `javadoc-process-launch` | supported | strict | pass | Rust DAG execution covers Java compilation, Jar packaging, and native Javadoc process launch with deterministic output. |
| `unsupported-composite-substitution` | fail-closed | strict | pass | Settings-level `includeBuild(...)` composite substitution rejects before Rust execution instead of approximating included-build semantics. |

Summary from `build/dogfood-phase2-20260601/dogfood-summary.md`:

| Metric | Result |
| --- | ---: |
| Projects matched | 7/7 |
| Supported projects matched | 6/6 |
| Fail-closed projects matched | 1/1 |
| Supported projects with zero JVM forwards | 6/6 |
| Rust RunBuild markers | 7/7 |
| Rust RunBuild executions | 7/7 |
| Task-graph captures | 7/7 |
| Daemon started signals | 0 |
| Daemon reused signals | 7 |
| Upstream observed wall time | 48278 ms |
| Rust substrate observed wall time | 25849 ms |
| Rust bootstrap/RunBuild observed time | 4713 ms |
| Non-Rust/Gradle overhead estimate | 21136 ms |
| Upstream task total | 109 |
| Rust substrate task total | 105 |

The dogfood runner owns one prewarmed Rust daemon for the manifest execution
and passes its shared state directory to each substrate invocation. This removes
per-project Rust daemon startup from the measured substrate path. In the
2026-06-01 Phase 2 completion run, the substrate path was faster than upstream
for the full local dogfood set while still preserving strict parity checks.

The current timing split shows the remaining gap is not primarily inside the
Rust task executor. The measured Rust bootstrap/RunBuild portion is about
4.7s across the whole local dogfood manifest, while the non-Rust/Gradle
invocation and configuration overhead is about 21.1s. The next performance
work should therefore target skipping or amortizing JVM-side configuration for
warm supported runs, not micro-optimizing individual Rust task executors first.

A direct cached-plan prototype now proves that seam for one supported fixture.
Using `gradle-substrate-runbuild --state-dir ... --project-dir ...` against an
existing `oss-style-java-library` build-plan shadow store found the matching
artifact and ran the cached Rust plan in 728 ms with 15 tasks, zero JVM
forwards, and `build-plan-shadow` as the plan source. The resulting build
outputs matched the upstream dogfood output file inventory, non-archive hashes,
and archive entries. This is still a prototype: it requires a previously
generated shadow store and explicit project directory. It now has conservative
mtime invalidation for tracked build-definition files and captured absolute
project input paths, plus SHA-256 content fingerprint validation for new shadow
artifacts. Paths captured as task outputs/local state/destroyables are excluded.
Touching `src/main/java/example/PublicApi.java`, deleting it, or changing its
content while restoring the old mtime rejects before `RunBuild` with a precise
stale-input diagnostic. Mutating a generated class under `build/classes` does
not invalidate the plan because Rust will regenerate it during execution. The
same artifact ran `--task :build` in 54 ms with the selected task's dependency
closure, 14 tasks, zero JVM forwards, and `build-plan-shadow` as the plan
source. Unqualified task names, unknown task paths, incomplete dependency
graphs, and ambiguous artifact matches reject before daemon execution.

The direct warm path is now included in the first-60s demo evidence. The latest
`tools/demo/first_60_seconds.py --mode fast` run wrote
`build/first60-warm-direct.json` and `build/first60-warm-direct.md`; the new
`warm_direct_runbuild` metric passed with 552.7 ms wall time, 48 ms reported
Rust `RunBuild` duration, 14 Rust-executed tasks, 6 validated input
fingerprints, `build-plan-shadow` as the plan source, configuration skipped,
and zero JVM forwards.

Direct warm coverage has also been expanded from one fixture to the supported
local dogfood set. `tools/dogfood_runner/direct_warm.py` performs one strict
Gradle/JVM capture per supported dogfood project and then runs the cached plan
directly through `gradle-substrate-runbuild`. The latest checked run wrote
`build/direct-warm-phase3-20260601/direct-warm-results.json` and
`build/direct-warm-phase3-20260601/direct-warm-summary.md`; all 6 supported
local dogfood projects passed direct warm execution with zero JVM forwards:
OSS-style Java library, Java multiproject, external JUnit library, external
BOM/conflict, JavaExec process launch, and Javadoc process launch. Total direct
warm wall time was 4575.9 ms across the six projects, with every project using
`build-plan-shadow` and validated input fingerprints.

Phase 3 also adds the Rust-first warm runner:
`tools/warm_runner/run.py`. Its checked single-fixture run at
`build/warm-runner-phase3-20260601/result.json` started with an empty state,
classified the first direct attempt as `cache-miss`, performed one strict
Gradle/JVM capture, and then completed direct Rust `RunBuild` with 14 tasks,
8 up-to-date tasks, 4 skipped tasks, zero JVM forwards, and
`build-plan-shadow`. Re-running against the same state wrote
`result-warm-hit.json` and completed as `WARM_HIT` without a capture, with
29 ms reported Rust `RunBuild` duration.

## Boundaries

Rust owns the admitted post-configuration path in this report: build-plan
admission, selected DAG execution, no-fallback enforcement, task parity checks,
and supported native task execution.

Gradle/JVM still owns DSL evaluation, `buildSrc`, arbitrary JVM plugin code,
and reflection-heavy Gradle APIs before the typed contract boundary. Unsupported
semantics must either delegate before strict admission or fail closed with a
precise diagnostic.

## Interpretation

This is a showable technical preview. It is not a 100% compatibility claim and
it does not prove a universal speedup. The current dogfood run proves that
several realistic supported build shapes can execute with zero JVM task forwards
and that one unsupported build fails closed.

## External OSS Gate

`testing/dogfood/oss-manifest.json` pins external repositories by immutable Git
commit and the runner fetches them under `build/dogfood-oss/sources`.

Latest checked run: 2026-05-12.

```bash
python3 tools/dogfood_runner/run.py \
  --manifest testing/dogfood/oss-manifest.json \
  --execute \
  --gradle-command "$PWD/build/gradle-under-test/bin/gradle" \
  --daemon-binary target/debug/gradle-substrate-daemon \
  --output-dir build/dogfood-oss \
  --verbose
```

Summary from `build/dogfood-oss/dogfood-summary.md`:

| Metric | Result |
| --- | ---: |
| Projects matched | 5/5 |
| Supported projects matched | 2/2 |
| Fail-closed projects matched | 3/3 |
| Supported projects with zero JVM forwards | 2/2 |
| Rust RunBuild markers | 5/5 |
| Rust RunBuild executions | 2/5 |
| Task-graph captures | 5/5 |
| Upstream observed wall time | 92849 ms |
| Rust substrate observed wall time | 111151 ms |
| Upstream task total | 41 |
| Rust substrate task total | 19 |

`spring-gs-rest-service-complete` now runs `clean assemble` through strict Rust
RunBuild with `build-plan-cache` as the plan source, eight Rust-executed tasks
including `compileJava`, `processResources`, `jar`, `resolveMainClassName`, and
`bootJar`, zero JVM forwards, and task/output/hash/archive-entry parity.
`spring-petclinic` runs the selected
`clean compileJava` slice with two Rust-executed tasks, zero JVM forwards, and
the same parity checks.
`spring-petclinic-testclasses` is a separate strict fail-closed gate for the
broader `clean testClasses` slice: it now rejects with a precise CycloneDX SBOM
diagnostic because `:cyclonedxBom` and `:cyclonedxDirectBom` declare SBOM
outputs and cannot be treated as lifecycle/no-op tasks. `mockito-main` and
`okio-root` now pass the earlier coarse composite/buildSrc gate and fail closed
at selected-task admission on Kotlin/build-logic tasks such as `KotlinCompile`,
precompiled script plugin generation, and plugin descriptor generation. That is
the narrower honest boundary: Rust has separated configuration-only composite
setup from root task admission, but it still must not approximate Kotlin
build-logic execution. The diagnostics are grouped by missing capability:
native Kotlin compilation or a Rust-controlled Kotlin compiler worker contract,
precompiled Kotlin DSL plugin generation, Gradle plugin descriptor generation,
and Kotlin Gradle plugin diagnostics. Earlier native-ready smoke runs delegated before concrete Rust
execution, so they are no longer counted as Rust-executed support evidence.

CycloneDX was inspected as a possible native promotion target and remains
fail-closed by design. The current captured contract for
`org.cyclonedx.gradle.CyclonedxDirectTask` exposes the resolved artifact files
as path inputs and the two declared report outputs, but it does not expose the
resolved dependency edge graph, component metadata/properties, scope mapping,
CycloneDX plugin options, serial/timestamp policy, or JSON/XML generation
semantics needed to reproduce the upstream SBOM faithfully. The aggregate task
only exposes the direct BOM files as inputs and its aggregate JSON output. The
observed upstream direct and aggregate JSON files differ in serial number,
timestamp, root component type, and dependency ordering. A native implementation
must therefore start with a schema-backed SBOM IR contract; copying or
synthesizing files from artifact paths would overclaim parity.

Related docs:

- [`docs/rust-substrate-preview.md`](rust-substrate-preview.md)
- [`substrate/PARITY.md`](../substrate/PARITY.md)
