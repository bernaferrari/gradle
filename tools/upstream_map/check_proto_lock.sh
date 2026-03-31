#!/bin/bash
# check_proto_lock.sh - Validates proto files against locked checksums
# Usage: ./tools/upstream_map/check_proto_lock.sh

set -euo pipefail

PROTO_DIR="substrate/proto"
LOCK_FILE="${PROTO_DIR}/.proto_lock"

echo "Checking proto lock file against current proto files..."

# Compute SHA256 for all proto files
check_sum=0
for proto_file in $(find "$PROTO_DIR" -name "*.proto" | sort); do
    hash=$(shasum -a 256 "$proto_file" | awk '{print $1}')
    expected=$(grep "$(basename "$proto_file")" "$LOCK_FILE" 2>/dev/null | awk '{print $1}' || echo "NOT_FOUND")
    
    if [[ "$hash" == "$expected" ]]; then
        echo "✓ $proto_file"
    else
        echo "✗ $proto_file (expected: ${expected:0:16}..., got: ${hash:0:16}...)"
        check_sum=1
    fi
done

if [[ $check_sum -eq 0 ]]; then
    echo "All proto files match locked checksums."
else
    echo "Some proto files don't match locked checksums!"
    echo "Run: find $PROTO_DIR -name '*.proto' -exec shasum -a 256 {} \; > $LOCK_FILE"
    exit 1
fi
