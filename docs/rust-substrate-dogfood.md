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

Latest checked run: 2026-05-12.

| Project | Expectation | Mode | Result | What It Proves |
| --- | --- | --- | --- | --- |
| `oss-style-java-library` | supported | strict | pass | Standard Java library lifecycle with sources/Javadoc/report-like outputs can run through the Rust kernel. |
| `java-multiproject` | supported | strict | pass | Rust DAG admission and execution handle a small multi-project Java build. |
| `external-junit-library` | supported | strict | pass | Pinned external dependencies plus JUnit Platform execution remain no-fallback in the supported slice. |
| `external-bom-and-conflict` | supported | strict | pass | Bounded Maven BOM/platform dependency semantics remain admitted in the dogfood path. |
| `javaexec-process-launch` | supported | strict | pass | Rust DAG execution covers Java compilation plus native `JavaExec` process launch with declared output parity. |
| `javadoc-process-launch` | supported | strict | pass | Rust DAG execution covers Java compilation, Jar packaging, and native Javadoc process launch with deterministic output. |
| `unsupported-composite-substitution` | fail-closed | strict | pass | Settings-level `includeBuild(...)` composite substitution rejects before Rust execution instead of approximating included-build semantics. |

Summary from `build/dogfood-current/dogfood-summary.md`:

| Metric | Result |
| --- | ---: |
| Projects matched | 7/7 |
| Supported projects matched | 6/6 |
| Fail-closed projects matched | 1/1 |
| Supported projects with zero JVM forwards | 6/6 |
| Rust RunBuild markers | 7/7 |
| Task-graph captures | 7/7 |
| Daemon started signals | 0 |
| Daemon reused signals | 7 |
| Upstream observed wall time | 19784 ms |
| Rust substrate observed wall time | 23112 ms |
| Rust bootstrap/RunBuild observed time | 3740 ms |
| Non-Rust/Gradle overhead estimate | 19372 ms |
| Upstream task total | 109 |
| Rust substrate task total | 105 |

The dogfood runner owns one prewarmed Rust daemon for the manifest execution
and passes its shared state directory to each substrate invocation. This removes
per-project Rust daemon startup from the measured substrate path, but it does
not yet make the full dogfood set faster than upstream.

The current timing split shows the remaining gap is not primarily inside the
Rust task executor. The measured Rust bootstrap/RunBuild portion is about
3.7s across the whole local dogfood manifest, while the non-Rust/Gradle
invocation and configuration overhead is about 19.4s. The next performance
work should therefore target skipping or amortizing JVM-side configuration for
warm supported runs, not micro-optimizing individual Rust task executors first.

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
