#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

proto_a="substrate/proto/v1"
proto_b="platforms/core-execution/rust-bridge/src/main/proto/v1"

# Detect whether proto paths were already dirty before sync.
preexisting_proto_changes=0
if ! git diff --quiet -- "$proto_a" "$proto_b"; then
  preexisting_proto_changes=1
fi

echo "== Proto sync"
./gradlew -q :rust-bridge:syncProtos

echo "== Proto parity"
if ! diff -qr "$proto_a" "$proto_b" >/tmp/rust-bridge-proto-diff.txt; then
  echo "Proto mismatch detected between $proto_a and $proto_b."
  sed -n '1,80p' /tmp/rust-bridge-proto-diff.txt
  exit 1
fi

if [[ "$preexisting_proto_changes" -eq 0 ]]; then
  if ! git diff --quiet -- "$proto_a" "$proto_b"; then
    echo "Proto drift detected: sync generated changes that are not committed."
    git --no-pager diff -- "$proto_a" "$proto_b" | sed -n '1,200p'
    exit 1
  fi
fi

echo "== Proto contract lock"
python3 tools/upstream_map/check_proto_lock.py

echo "== Proto version tracking"
python3 tools/upstream_map/proto_version.py --check

echo "== Upstream map + metadata validation"
python3 tools/upstream_map/sync_metadata.py --check
python3 tools/upstream_map/validate_map.py

echo "Upstream drift checks passed."
