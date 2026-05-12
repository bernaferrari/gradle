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
| `unsupported-custom-task` | fail-closed | strict | pass | Arbitrary JVM task action semantics reject before Rust execution instead of silently forwarding task-by-task. |

Summary from `build/dogfood-current/dogfood-summary.md`:

| Metric | Result |
| --- | ---: |
| Projects matched | 5/5 |
| Supported projects matched | 4/4 |
| Fail-closed projects matched | 1/1 |
| Supported projects with zero JVM forwards | 4/4 |
| Rust RunBuild markers | 5/5 |
| Task-graph captures | 5/5 |
| Daemon started signals | 5 |
| Upstream observed wall time | 14626 ms |
| Rust substrate observed wall time | 16669 ms |
| Upstream task total | 72 |
| Rust substrate task total | 72 |

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
| Projects matched | 3/3 |
| Supported projects matched | 3/3 |
| Supported projects with zero JVM forwards | 3/3 |
| Rust RunBuild markers | 3/3 |
| Task-graph captures | 3/3 |
| Upstream observed wall time | 21331 ms |
| Rust substrate observed wall time | 29769 ms |
| Upstream task total | 28 |
| Rust substrate task total | 28 |

Passing external entries are `spring-petclinic`, `mockito-main`, and
`okio-root`, all in `native-ready-default` mode. `spring-petclinic` now runs
`clean classes`; the runner verifies task/output inventory and hashes while
allowlisting the generated SBOM JSON hash as nondeterministic metadata.
`mockito-main` and `okio-root` remain `help`-level smokes. This proves Rust
kernel admission and no-forward execution for the selected tasks, not full
project build parity.

Related docs:

- [`docs/rust-substrate-preview.md`](rust-substrate-preview.md)
- [`substrate/PARITY.md`](../substrate/PARITY.md)
