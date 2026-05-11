# Rust Substrate Maintenance Guardrails

This document defines the maintenance rules for the Rust Substrate Preview. It
keeps the port contract-shaped: JVM compatibility captures typed data, Rust
admits or rejects that data, and evidence is recorded before claims expand.

## Ownership

JVM bridge owns:

- Gradle settings/project DSL evaluation;
- `buildSrc` and arbitrary JVM plugin execution;
- task/dependency/repository model capture at Gradle-owned lifecycle points;
- conversion from Gradle runtime objects into stable proto/build-plan fields;
- fail-closed markers when a Gradle feature cannot be represented as typed data.

Rust owns:

- daemon protocol services and persisted stores;
- build-plan schema validation, fingerprinting, and quarantine behavior;
- execution-kernel admission;
- dependency graph admission/resolution for supported descriptors;
- native task scheduling, execution, cache/history decisions, and diagnostics;
- rejecting unsupported contracts before no-fallback execution.

No side may silently approximate the other side's responsibilities. If the JVM
cannot emit a stable contract, or Rust cannot faithfully execute it, the feature
must be unsupported until a contract and tests exist.

## Schema Evolution

The canonical proto source is `substrate/proto/v1`. Generated Java protos under
the Gradle bridge are derived artifacts and must not be edited by hand.

Build-plan schema changes must follow these rules:

- increment the persisted build-plan schema version when old artifacts cannot be
  read safely;
- keep additive proto fields backward-compatible where possible;
- treat removed or semantically changed proto fields as compatibility-breaking;
- reject or quarantine persisted build-plan artifacts whose schema, build id, or
  fingerprint does not match the current reader;
- add or update Rust tests that prove old/mismatched artifacts fail closed.

## Proto Workflow

After an intentional proto change:

```bash
cargo build -p gradle-substrate-daemon
./gradlew :rust-bridge:syncProtos
python3 tools/upstream_map/check_proto_lock.py --update
python3 tools/upstream_map/proto_version.py --update
./tools/upstream_map/check_drift.sh
```

Then run focused Rust and Java bridge tests for the changed service. Do not
commit a proto change with stale `proto-lock.sha256`, stale
`proto-versions.json`, or unsynced Java bridge protos.

## Adding a Supported Semantic Slice

1. Name the exact Gradle behavior and the supported subset.
2. Add JVM capture fields or proto fields only for data Rust can consume.
3. Add Rust admission checks before execution/resolution.
4. Add Rust implementation and focused unit tests.
5. Add at least one corpus or integration proof when the behavior affects a
   user-facing build.
6. Run the relevant no-fallback corpus command with declared/resolved graph or
   output/hash/archive parity as appropriate.
7. Update `substrate/PARITY.md` with exact evidence and the command that
   produced it.
8. Update or close the related Beads issue with the same evidence.

## Adding an Unsupported Fail-Closed Gate

1. Detect the unsupported shape as early as possible, preferably during JVM
   capture or Rust admission.
2. Preserve a diagnostic that names the specific unsupported feature.
3. Add a focused unit, bridge, or corpus contract test.
4. If it is a project-level unsupported shape, add or update
   `testing/corpus/unsupported-manifest.json`.
5. Ensure no-fallback execution rejects before task dispatch or dependency
   traversal when the unsupported shape would affect correctness.
6. Record the unsupported status in `PARITY.md`; do not phrase it as supported.

## Evidence Rules

`substrate/PARITY.md` is the current evidence ledger. Every new claim must
include:

- the supported or unsupported behavior;
- the exact test, corpus, or demo command;
- pass counts or key metrics;
- whether evidence is manifest-backed, focused-test-backed, or demo-only;
- any remaining boundary or non-goal.

Beads is the work ledger. Close a Beads issue only after the evidence exists in
code, docs, command output, or artifacts. If a future maintainer needs the
context after compaction, the Beads close reason must say what changed and which
commands proved it.

## Anti-Overclaiming

Avoid phrases like "full Gradle parity", "all dependency resolution", or
"Rust-owned Gradle" unless a gate proves that exact scope. Prefer bounded
phrases such as "static Maven POM slice", "checked-in external corpus", or
"focused repository content-filter tests".

Preview docs must keep saying that DSL evaluation, `buildSrc`, and arbitrary JVM
plugins are JVM-owned compatibility islands.
