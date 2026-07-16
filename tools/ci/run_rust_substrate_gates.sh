#!/usr/bin/env bash
# Rust substrate CI gates: full daemon package tests, build, optional direct-warm dogfood.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

MANIFEST="${DOGFOOD_MANIFEST:-testing/dogfood/manifest.json}"
OUTPUT_DIR="${DOGFOOD_OUTPUT_DIR:-build/ci-direct-warm}"
DAEMON_BIN="${DAEMON_BINARY:-target/debug/gradle-substrate-daemon}"
RUNBUILD_BIN="${RUNBUILD_BINARY:-target/debug/gradle-substrate-runbuild}"
# When 1, missing under-test / dogfood failure fails the job. Default 0 so Linux CI stays light.
CI_REQUIRE_DOGFOOD="${CI_REQUIRE_DOGFOOD:-0}"

step=1
run_step() {
  local title="$1"
  shift
  echo
  echo "== [${step}] ${title}"
  "$@"
  step=$((step + 1))
}

resolve_under_test() {
  if [[ -n "${GRADLE_UNDER_TEST_BIN:-}" && -x "${GRADLE_UNDER_TEST_BIN}" ]]; then
    printf '%s\n' "${GRADLE_UNDER_TEST_BIN}"
    return 0
  fi
  if [[ -n "${GRADLE_UNDER_TEST:-}" ]]; then
    local candidate="${GRADLE_UNDER_TEST%/}/bin/gradle"
    if [[ -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  fi
  if [[ -x "build/gradle-under-test/bin/gradle" ]]; then
    printf '%s\n' "build/gradle-under-test/bin/gradle"
    return 0
  fi
  return 1
}

expected_supported_count() {
  python3 - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
data = json.loads(manifest.read_text(encoding="utf-8"))
projects = data.get("projects") or []
print(sum(1 for p in projects if p.get("expectation") == "supported"))
PY
}

assert_dogfood_summary() {
  local summary_json="$1"
  local expected="$2"
  python3 - "$summary_json" "$expected" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
expected = int(sys.argv[2])
payload = json.loads(path.read_text(encoding="utf-8"))
summary = payload.get("summary") or payload
passed = int(summary.get("direct_warm_supported_count") or 0)
supported = int(summary.get("supported_project_count") or 0)
failed = summary.get("failed_projects") or []

print(
    f"Dogfood summary: {passed}/{supported} supported direct-warm "
    f"(expected supported={expected}); failed={failed}"
)

if failed:
    raise SystemExit(f"dogfood failed projects: {failed}")
if supported != expected:
    raise SystemExit(
        f"supported_project_count {supported} != manifest expected {expected}"
    )
if passed != expected:
    raise SystemExit(
        f"direct_warm_supported_count {passed} != expected {expected}"
    )
if expected == 7 and f"{passed}/{supported}" != "7/7":
    raise SystemExit(f"expected 7/7 dogfood summary, got {passed}/{supported}")
PY
}

echo "Rust substrate gates (CI_REQUIRE_DOGFOOD=${CI_REQUIRE_DOGFOOD})"

run_step "Full package tests (gradle-substrate-daemon)" \
  cargo test -q -p gradle-substrate-daemon

run_step "Build gradle-substrate-daemon (+ runbuild bin deps)" \
  cargo build -q -p gradle-substrate-daemon

# Ensure runbuild binary exists for direct-warm when dogfood runs.
if [[ ! -x "${RUNBUILD_BIN}" ]]; then
  run_step "Build gradle-substrate-runbuild" \
    cargo build -q -p gradle-substrate-daemon --bin gradle-substrate-runbuild
fi

UNDER_TEST=""
if UNDER_TEST="$(resolve_under_test)"; then
  echo
  echo "== [${step}] Direct-warm dogfood (under-test: ${UNDER_TEST})"
  step=$((step + 1))
  mkdir -p "${OUTPUT_DIR}"
  set +e
  python3 tools/dogfood_runner/direct_warm.py \
    --manifest "${MANIFEST}" \
    --daemon-binary "${DAEMON_BIN}" \
    --runbuild-binary "${RUNBUILD_BIN}" \
    --output-dir "${OUTPUT_DIR}"
  dogfood_rc=$?
  set -e

  results_json="${OUTPUT_DIR}/direct-warm-results.json"
  if [[ ! -f "${results_json}" ]]; then
    echo "dogfood did not write ${results_json} (exit ${dogfood_rc})"
    if [[ "${CI_REQUIRE_DOGFOOD}" == "1" ]]; then
      exit 1
    fi
    echo "WARNING: dogfood artifacts missing; continuing because CI_REQUIRE_DOGFOOD!=1"
  else
    expected="$(expected_supported_count)"
    if ! assert_dogfood_summary "${results_json}" "${expected}"; then
      if [[ "${CI_REQUIRE_DOGFOOD}" == "1" ]]; then
        exit 1
      fi
      echo "WARNING: dogfood summary gate failed; continuing because CI_REQUIRE_DOGFOOD!=1"
    elif [[ "${dogfood_rc}" -ne 0 ]]; then
      echo "dogfood runner exit ${dogfood_rc} despite summary pass"
      if [[ "${CI_REQUIRE_DOGFOOD}" == "1" ]]; then
        exit 1
      fi
    else
      echo "Dogfood gate passed (${expected}/${expected} supported)"
    fi
  fi
else
  echo
  echo "== [${step}] Direct-warm dogfood"
  step=$((step + 1))
  reason="build/gradle-under-test/bin/gradle missing and GRADLE_UNDER_TEST_BIN/GRADLE_UNDER_TEST unset"
  if [[ "${CI_REQUIRE_DOGFOOD}" == "1" ]]; then
    echo "FAIL: dogfood required but skipped: ${reason}"
    echo "Install via tools/install_gradle_under_test.sh (or distributions-full:install) and re-run."
    exit 1
  fi
  echo "SKIP dogfood: ${reason}"
  echo "Dogfood runs when under-test is present; set CI_REQUIRE_DOGFOOD=1 to require it."
fi

echo
echo "Rust substrate gates OK"
