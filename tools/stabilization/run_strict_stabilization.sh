#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

MODE="${1:-full}"
if [[ "$MODE" != "full" && "$MODE" != "quick" ]]; then
  echo "Usage: $0 [full|quick]"
  exit 2
fi

step=1
run_step() {
  local title="$1"
  shift
  echo
  echo "== [${step}] ${title}"
  "$@"
  step=$((step + 1))
}

echo "Strict stabilization mode: ${MODE}"

run_step "Run upstream drift checks (proto + map + metadata)" ./tools/upstream_map/check_drift.sh

run_step "Compile Java bridge and bridge test classes" ./gradlew -q :rust-bridge:compileJava :rust-bridge:testClasses
run_step "Type-check Rust daemon" cargo check -p gradle-substrate-daemon

echo
echo "== [${step}] Run critical Rust regression tests"
critical_tests=(
  "server::work::tests::test_evaluate_input_hash_changes_when_only_keys_change"
  "server::config_cache::tests::test_validate_config_ignores_hash_order"
  "server::build_layout::tests::test_add_subproject_rejects_string_prefix_outside_root"
)
for t in "${critical_tests[@]}"; do
  echo "  -> ${t}"
  cargo test -p gradle-substrate-daemon "$t" -- --exact
done
cargo test -p gradle-substrate-daemon --test hash_compatibility_test
cargo test -p gradle-substrate-daemon --test build_plan_ir_golden_test
cargo test -p gradle-substrate-daemon --test build_plan_shadow_test
cargo test -p gradle-substrate-daemon --test differential build_plan_shadow_differential_test::
step=$((step + 1))

if [[ "$MODE" == "full" ]]; then
  run_step "Build release daemon binary" cargo build -p gradle-substrate-daemon --release
  run_step "Run daemon e2e smoke test" ./substrate/scripts/e2e-smoke-test.sh "$ROOT_DIR/target/release/gradle-substrate-daemon"
fi

echo
echo "Strict stabilization checks passed."
