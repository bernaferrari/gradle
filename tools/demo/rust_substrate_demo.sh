#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="quick"
RUN_SAMPLE_BUILDS=1
RUN_GRPC_E2E=1
RUN_NATIVE_SHADOW=1
RUN_FIRST60=1
OUTPUT_DIR=""

usage() {
  cat <<'USAGE'
Usage: tools/demo/rust_substrate_demo.sh [--quick|--full] [--skip-first60] [--skip-sample-builds] [--skip-grpc-e2e] [--skip-native-shadow] [--output-dir DIR]

Runs an honest Rust substrate demo:
  - first-60-second visible wins: daemon ready time, authoritative Rust RunBuild with zero JVM forwards, real-build remote requests avoided, file-watch latency
  - strict stabilization gate
  - checked-in offline corpus contract validation
  - external dependency and unsupported corpus contract validation
  - captured build-plan shadow Java lifecycle execution with JVM fallback disabled
  - optional sample Gradle builds from testing/corpus
  - optional no-fallback authoritative RunBuild gate with output inventory/hash parity
  - optional native-ready-default RunBuild gate
  - optional dependency transport smoke test
  - optional Rust daemon gRPC e2e test suite

Use --full when preparing a public demo; it runs the full stabilization mode,
including release daemon build and release smoke coverage.
Sample RunBuild corpus gates require GRADLE_UNDER_TEST_BIN, or GRADLE_UNDER_TEST
pointing at a local distribution built from this fork.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --quick)
      MODE="quick"
      ;;
    --full)
      MODE="full"
      ;;
    --skip-sample-builds)
      RUN_SAMPLE_BUILDS=0
      ;;
    --skip-grpc-e2e)
      RUN_GRPC_E2E=0
      ;;
    --skip-native-shadow)
      RUN_NATIVE_SHADOW=0
      ;;
    --skip-first60)
      RUN_FIRST60=0
      ;;
    --output-dir)
      shift
      OUTPUT_DIR="${1:-}"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/gradle-rust-demo.XXXXXX")"
fi

GRADLE_UNDER_TEST_BIN="${GRADLE_UNDER_TEST_BIN:-}"
if [[ -z "$GRADLE_UNDER_TEST_BIN" && -n "${GRADLE_UNDER_TEST:-}" ]]; then
  GRADLE_UNDER_TEST_BIN="$GRADLE_UNDER_TEST/bin/gradle"
fi

cd "$ROOT_DIR"

echo "Rust substrate demo mode: $MODE"
echo "Output directory: $OUTPUT_DIR"
echo

run_step() {
  local title="$1"
  shift
  echo "== $title"
  "$@"
  echo
}

if [[ "$RUN_FIRST60" -eq 1 ]]; then
  run_step "First-60-second visible Rust wins" \
    python3 ./tools/demo/first_60_seconds.py \
      --mode fast \
      --output "$OUTPUT_DIR/first60.json"
fi

run_step "Strict stabilization gate" ./tools/stabilization/run_strict_stabilization.sh "$MODE"

run_step "Offline corpus contract validation" \
  python3 ./tools/corpus_runner/run.py \
    --manifest testing/corpus/manifest.json \
    --contract-only \
    --output-dir "$OUTPUT_DIR"

run_step "External dependency corpus contract validation" \
  python3 ./tools/corpus_runner/run.py \
    --manifest testing/corpus/external-manifest.json \
    --contract-only \
    --output-dir "$OUTPUT_DIR/external-contract"

run_step "Unsupported corpus contract validation" \
  python3 ./tools/corpus_runner/run.py \
    --manifest testing/corpus/unsupported-manifest.json \
    --contract-only \
    --output-dir "$OUTPUT_DIR/unsupported-contract"

if [[ "$RUN_NATIVE_SHADOW" -eq 1 ]]; then
  run_step "No-fallback captured Java lifecycle via Rust" \
    cargo test -p gradle-substrate-daemon \
      --test build_plan_shadow_test \
      refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback \
      -- --exact
  run_step "Rust dependency transport smoke test" \
    cargo test -p gradle-substrate-daemon \
      download_artifact_streams
fi

if [[ "$RUN_SAMPLE_BUILDS" -eq 1 ]]; then
  if [[ -z "$GRADLE_UNDER_TEST_BIN" ]]; then
    echo "RUN_SAMPLE_BUILDS requires GRADLE_UNDER_TEST_BIN, or GRADLE_UNDER_TEST pointing at a local distribution from this fork." >&2
    exit 1
  fi
  run_step "Build corpus Java library sample" \
    ./gradlew -q -p testing/corpus/java-library-kotlin-dsl clean build
  run_step "Build corpus Java application sample" \
    ./gradlew -q -p testing/corpus/java-application-groovy-dsl clean build
  run_step "Build corpus Java multi-project sample" \
    ./gradlew -q -p testing/corpus/java-multiproject-kotlin-dsl clean build
  run_step "Build corpus Java resource expansion sample" \
    ./gradlew -q -p testing/corpus/java-resources-expand-kotlin-dsl clean build
  run_step "Build corpus Sync resource sample" \
    ./gradlew -q -p testing/corpus/sync-resources-kotlin-dsl clean build
  run_step "Build corpus CopySpec pattern sample" \
    ./gradlew -q -p testing/corpus/copy-patterns-kotlin-dsl clean build
  run_step "Build Rust substrate daemon" \
    cargo build -q -p gradle-substrate-daemon
  run_step "Build checked-in corpus with authoritative Rust RunBuild gate" \
    python3 ./tools/corpus_runner/run.py \
      --manifest testing/corpus/manifest.json \
      --gradle-command "$GRADLE_UNDER_TEST_BIN" \
      --daemon-binary target/debug/gradle-substrate-daemon \
      --runbuild-authoritative \
      --tasks clean build \
      --timeout 300 \
      --verbose \
      --output-dir "$OUTPUT_DIR"
  run_step "Build checked-in corpus with native-ready default gate" \
    python3 ./tools/corpus_runner/run.py \
      --manifest testing/corpus/manifest.json \
      --gradle-command "$GRADLE_UNDER_TEST_BIN" \
      --daemon-binary target/debug/gradle-substrate-daemon \
      --runbuild-native-ready-default \
      --tasks clean build \
      --timeout 300 \
      --output-dir "$OUTPUT_DIR/native-ready-default"
  if [[ -f "$OUTPUT_DIR/corpus_summary.json" ]]; then
    python3 ./tools/performance/rust_substrate_perf_report.py \
      "$OUTPUT_DIR/corpus_summary.json" \
      --output "$OUTPUT_DIR/performance.md"
    run_step "Authoritative corpus evidence" \
      python3 - "$OUTPUT_DIR/corpus_summary.json" <<'PY'
import json
import sys

summary = json.load(open(sys.argv[1], encoding="utf-8"))
total = summary["project_count"]
print(f"projects matched: {summary['matched_project_count']}/{total}")
print(f"successful upstream/substrate builds: {summary['successful_project_count']}/{total}")
print(f"no-fallback projects: {summary['no_fallback_project_count']}/{total}")
print(f"exit-code parity: {summary['exit_code_match_count']}/{total}")
print(f"task-list parity: {summary['task_list_match_count']}/{total}")
print(f"output inventory parity: {summary['output_file_inventory_match_count']}/{total}")
print(f"non-archive output hash parity: {summary['output_hash_match_count']}/{total}")
print(f"task totals: upstream={summary['upstream_task_total']}, substrate={summary['substrate_task_total']}")
print(f"observed wall time: upstream={summary['upstream_duration_ms']}ms, substrate={summary['substrate_duration_ms']}ms")
print("performance report: performance.md")
if summary["failed_projects"]:
    print("failed projects: " + ", ".join(summary["failed_projects"]))
if summary["fallback_projects"]:
    print("fallback projects: " + ", ".join(summary["fallback_projects"]))
PY
  fi
fi

if [[ "$RUN_GRPC_E2E" -eq 1 ]]; then
  run_step "Rust daemon gRPC e2e tests" \
    cargo test -p gradle-substrate-daemon --test e2e_grpc_test
fi

cat <<EOF
Demo completed.

What this proves:
  - Rust/JVM proto drift checks pass.
  - Hardened bridge clients fail closed instead of returning hidden defaults.
  - Build-plan IR v2 fingerprints and shadow artifacts are stable.
  - The checked-in Java library/application/multi-project/resource-expansion/compile-options/Copy/Sync/archive/Exec corpus has deterministic build-plan contracts.
  - The external dependency corpus has deterministic dependency/constraint contract coverage without requiring network in the demo contract step.
  - Unsupported JVM/custom work is tracked separately so native execution does not silently claim unsupported parity.
  - A captured JVM-host Java lifecycle build-plan shadow can execute JavaCompile, ProcessResources, classes, and Jar through Rust with JVM fallback disabled.
  - Rust dependency transport can stream artifact bytes over HTTP in the daemon service, and an opt-in Gradle external-resource seam can route uncached repository downloads through Rust before falling back to Java transport.
  - Native Copy/ProcessResources/Sync coverage includes recursive directory sources, declared token expansion, CopySpec include/exclude patterns, nested CopySpec into mappings for Copy and Sync, duplicate destination strategies, empty-directory semantics, and copied file permissions.
  - Native archive coverage includes Zip, Tar gzip+bzip2, War, and Ear tasks with reproducible ZIP timestamps, nested CopySpec into mappings for Zip and Tar, duplicate-entry strategy handling, empty-directory semantics, file permissions, and bzip2 TAR output.
  - When sample builds are enabled, the checked-in Java corpus exercises the explicit no-fallback RunBuild gate from real Gradle invocations and compares stable output inventories, non-archive SHA-256 hashes, and archive entry inventories.
  - The native-ready-default gate is exercised separately and delegates when a selected plan is incomplete.
  - The authoritative corpus summary records matched projects, no-fallback counts, task parity, output parity, and observed upstream/substrate wall-clock timing.
  - Rust daemon gRPC behavior is exercised when --skip-grpc-e2e is not used.
  - First-60-second fast metrics record daemon socket readiness, explicit authoritative Rust RunBuild with zero JVM task forwards, isolated real-build remote requests avoided when a local install is present, and native file-watch first-event latency. The heavier first-60 proof mode keeps Rust dependency transport/store/checksum, static Maven prefetch, and focused artifact/metadata read-through checks available without slowing the visible path.

What this does not claim:
  - This is not a full Gradle replacement.
  - Groovy/Kotlin DSL and legacy plugin semantics still run through the JVM compatibility island.
  - Authoritative no-fallback execution is only ready for the subsystems covered by current gates.

Artifacts: $OUTPUT_DIR
EOF
