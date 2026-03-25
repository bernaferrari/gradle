#!/usr/bin/env bash
# Substrate test runner — runs tests in module batches to avoid
# runaway memory from 832+ tests accumulating malloc'd pages.
#
# Root cause: macOS system allocator does not return pages to the OS,
# so running 832 tests in one process consumes ~150GB RSS even though
# each individual module only uses <20MB.
#
# Usage:
#   ./scripts/run-tests.sh              # all tests, batched by module
#   ./scripts/run-tests.sh hash         # only hash tests
#   ./scripts/run-tests.sh --full       # single-process (CAUTION: ~150GB RAM)
#   ./scripts/run-tests.sh --report     # summary of pass/fail per module

set -euo pipefail

cd "$(dirname "$0")/.."

export RAYON_NUM_THREADS=2
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/substrate-target}"

modules=(
    hash cache parser_service file_watch config
    dependency_resolution parallel_scheduler build_script_parser
    toolchain incremental_compilation worker_process exec plugin
    dag_executor task_graph build_init build_operations build_layout
    build_result build_event_stream build_metrics build_comparison
    cache_orchestration config_cache console authoritative control
    execution_history execution_plan file_fingerprint garbage_collection
    problem_reporting remote_cache resource_management scopes
    test_execution value_snapshot work artifact_publishing bootstrap
)

if [ "${1:-}" = "--full" ]; then
    echo "=== WARNING: single-process run may use 100GB+ RAM ==="
    cargo test -p gradle-substrate-daemon -- --test-threads=1
    exit $?
fi

if [ "${1:-}" = "--report" ]; then
    # Run each module and report pass/fail
    passed=0
    failed=0
    failed_modules=()
    for mod in "${modules[@]}"; do
        result=$(cargo test -p gradle-substrate-daemon "server::$mod::" -- --test-threads=1 2>&1 | grep "test result" || true)
        if echo "$result" | grep -q "ok"; then
            count=$(echo "$result" | grep -oP '\d+ passed' | head -1)
            echo "  PASS  $mod ($count)"
            passed=$((passed + 1))
        else
            echo "  FAIL  $mod"
            failed=$((failed + 1))
            failed_modules+=("$mod")
        fi
    done
    echo ""
    echo "=== Summary: $passed modules passed, $failed modules failed ==="
    if [ ${#failed_modules[@]} -gt 0 ]; then
        echo "Failed modules: ${failed_modules[*]}"
        exit 1
    fi
    exit 0
fi

# Default: run with a filter if provided, otherwise batch
if [ -n "${1:-}" ]; then
    cargo test -p gradle-substrate-daemon "$1" -- --test-threads=1
else
    # Run all tests in module batches (each batch is a separate process)
    echo "=== Running tests in module batches (to limit memory) ==="
    for mod in "${modules[@]}"; do
        echo "--- Module: $mod ---"
        cargo test -p gradle-substrate-daemon "server::$mod::" -- --test-threads=1
    done
    echo "--- Integration tests ---"
    cargo test -p gradle-substrate-daemon --tests -- --test-threads=1
fi
