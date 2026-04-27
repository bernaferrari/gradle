#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="quick"
RUN_SAMPLE_BUILDS=1
RUN_GRPC_E2E=1
OUTPUT_DIR=""

usage() {
  cat <<'USAGE'
Usage: tools/demo/rust_substrate_demo.sh [--quick|--full] [--skip-sample-builds] [--skip-grpc-e2e] [--output-dir DIR]

Runs an honest Rust substrate demo:
  - strict stabilization gate
  - checked-in corpus contract validation
  - optional sample Gradle builds from testing/corpus
  - optional Rust daemon gRPC e2e test suite

Use --full when preparing a public demo; it runs the full stabilization mode,
including release daemon build and release smoke coverage.
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

run_step "Strict stabilization gate" ./tools/stabilization/run_strict_stabilization.sh "$MODE"

run_step "Offline corpus contract validation" \
  python3 ./tools/corpus_runner/run.py \
    --manifest testing/corpus/manifest.json \
    --contract-only \
    --output-dir "$OUTPUT_DIR"

if [[ "$RUN_SAMPLE_BUILDS" -eq 1 ]]; then
  run_step "Build corpus Java library sample" \
    ./gradlew -q -p testing/corpus/java-library-kotlin-dsl clean build
  run_step "Build corpus Java application sample" \
    ./gradlew -q -p testing/corpus/java-application-groovy-dsl clean build
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
  - The checked-in Java library/application corpus has deterministic build-plan contracts.
  - Rust daemon gRPC behavior is exercised when --skip-grpc-e2e is not used.

What this does not claim:
  - This is not a full Gradle replacement.
  - Groovy/Kotlin DSL and legacy plugin semantics still run through the JVM compatibility island.
  - Authoritative no-fallback execution is only ready for the subsystems covered by current gates.

Artifacts: $OUTPUT_DIR
EOF
