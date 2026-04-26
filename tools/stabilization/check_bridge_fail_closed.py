#!/usr/bin/env python3
"""Guard fail-closed semantics for hardened Rust bridge clients.

This audit is intentionally scoped to clients that are expected to surface
Rust unavailability as an explicit SubstrateException. Shadow listeners may
still catch/report those exceptions in non-authoritative mode.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

CLIENTS = [
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/buildinit/RustBuildInitClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/buildlayout/RustBuildLayoutClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/buildops/RustBuildOperationsClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/buildresult/RustBuildResultClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/problems/RustProblemReportingClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/testexec/RustTestExecutionClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/toolchain/RustToolchainServiceClient.java",
    "platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/worker/RustWorkerProcessClient.java",
]

NOOP_RETURN = re.compile(r"if\s*\(\s*client\.isNoop\(\)\s*\)\s*\{\s*return\b", re.MULTILINE)
CATCH_RETURN = re.compile(r"catch\s*\([^)]*Exception[^)]*\)\s*\{(?:(?!\n\s*\}).)*\n\s*return\b", re.DOTALL)


def main() -> int:
    failures: list[str] = []
    for relative in CLIENTS:
        path = ROOT / relative
        text = path.read_text(encoding="utf-8")
        if NOOP_RETURN.search(text):
            failures.append(f"{relative}: no-op client branch returns instead of throwing")
        if CATCH_RETURN.search(text):
            failures.append(f"{relative}: RPC exception branch returns instead of throwing")
        if "SubstrateException" not in text:
            failures.append(f"{relative}: missing explicit SubstrateException failure surface")

    if failures:
        print("Bridge fail-closed audit failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print(f"Bridge fail-closed audit passed: {len(CLIENTS)} clients checked.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
