#!/usr/bin/env python3
"""
Discovers all Gradle upstream modules and maps them to Rust substrate modules.
Generates reports showing coverage status and gaps.

Usage:
  python3 tools/upstream_map/discover_modules.py [--format json|table] [--status all|complete|partial|missing]
"""

import os
import sys
import json
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent

PLATFORMS = [
    "core-runtime", "core-configuration", "core-execution",
    "software", "jvm", "extensibility", "ide", "native", "enterprise",
]

SUBPROJECTS = [
    "core", "core-api", "composite-builds", "build-events",
]

SUBSTRATE_MAP = {
    # core-runtime
    "cli": { "rust": "client/", "status": "partial" },
    "launcher": { "rust": "server/bootstrap.rs", "status": "partial" },
    "daemon-protocol": { "rust": "server/control.rs, server/build_operations.rs", "status": "partial" },
    "logging": { "rust": "server/console.rs", "status": "partial" },
    "process-services": { "rust": "server/exec.rs, server/worker_process.rs", "status": "partial" },
    "serialization": { "rust": "internal (bincode)", "status": "complete" },
    "start-parameter": { "rust": "main.rs -- clap parsing", "status": "partial" },
    "wrapper-main": { "rust": "N/A - JVM wrapper kept", "status": "not-started" },

    # core-configuration
    "model-core": { "rust": "server/platform.rs, server/scopes.rs", "status": "partial" },
    "configuration-cache": { "rust": "server/config_cache.rs, config_cache_ir.rs", "status": "partial" },
    "declarative-dsl-evaluator": { "rust": "groovy_parser/*, ast_extractor.rs", "status": "partial" },
    "kotlin-dsl": { "rust": "JVM host (compat)", "status": "deferred" },
    "model-groovy": { "rust": "groovy_parser/lexer.rs, parser.rs, ast.rs", "status": "partial" },

    # core-execution
    "execution": { "rust": "server/dag_executor.rs, server/parallel_scheduler.rs", "status": "partial" },
    "workers": { "rust": "server/worker_process.rs, server/work.rs", "status": "partial" },
    "file-watching": { "rust": "server/file_watch.rs, server/file_fingerprint.rs", "status": "partial" },
    "hashing": { "rust": "server/hash.rs", "status": "partial" },
    "snapshots": { "rust": "server/value_snapshot.rs, server/file_fingerprint.rs", "status": "partial" },
    "build-cache-local": { "rust": "server/cache.rs, server/cache_orchestration.rs", "status": "partial" },
    "build-cache-http": { "rust": "server/remote_cache.rs", "status": "partial" },

    # software
    "dependency-management": { "rust": "server/dependency_resolution.rs", "status": "partial" },
    "maven": { "rust": "internal in dependency_resolution.rs", "status": "partial" },
    "ivy": { "rust": "server/ivy_parser.rs", "status": "partial" },
    "publish": { "rust": "server/artifact_publishing.rs", "status": "partial" },
    "security": { "rust": "internal", "status": "not-started" },
    "signing": { "rust": "not-started", "status": "not-started" },

    # jvm
    "toolchains-jvm": { "rust": "server/toolchain.rs, server/platform.rs", "status": "partial" },
    "language-java": { "rust": "JVM host (compat)", "status": "deferred" },
    "plugins-java": { "rust": "JVM host (compat)", "status": "deferred" },
    "testing-jvm": { "rust": "server/test_execution.rs", "status": "partial" },

    # extensibility
    "plugin-use": { "rust": "server/plugin.rs", "status": "partial" },
    "test-kit": { "rust": "JVM host (compat)", "status": "deferred" },

    # ide
    "tooling-api": { "rust": "server/ide_lsp.rs, build_model_cache.rs", "status": "partial" },
    "tooling-api-bridge": { "rust": "JVM bridge", "status": "partial" },

    # enterprise
    "enterprise": { "rust": "not-started", "status": "not-started" },
}

def get_status_counts():
    counts = {"complete": 0, "partial": 0, "not-started": 0, "deferred": 0}
    for info in SUBSTRATE_MAP.values():
        counts[info["status"]] += 1
    return counts

def format_table(status_filter="all"):
    counts = get_status_counts()
    total = len(SUBSTRATE_MAP)
    
    header = f"{'Upstream Module':<35} {'Rust Module':<45} {'Status':<12}"
    separator = "=" * 95
    print(f"\n{'Gradle → Rust Substrate Module Map':^95}")
    print(separator)
    print(header)
    print("-" * 95)
    
    for module, info in sorted(SUBSTRATE_MAP.items()):
        if status_filter != "all" and info["status"] != status_filter:
            continue
        print(f"{module:<35} {info['rust']:<45} {info['status']:<12}")
    
    print(separator)
    print(f"\nSummary: {counts['complete']} complete, {counts['partial']} partial, "
          f"{counts['not-started']} not-started, {counts['deferred']} deferred, "
          f"{total} total modules")
    print(f"Coverage: {counts['complete'] + counts['partial']}/{total} modules have Rust implementation")

def format_json(status_filter="all"):
    result = {}
    for module, info in SUBSTRATE_MAP.items():
        if status_filter != "all" and info["status"] != status_filter:
            continue
        result[module] = info
    result["summary"] = get_status_counts()
    print(json.dumps(result, indent=2))

if __name__ == "__main__":
    fmt = "table"
    status = "all"
    args = sys.argv[1:]
    if "--format" in args:
        idx = args.index("--format")
        fmt = args[idx + 1]
    if "--status" in args:
        idx = args.index("--status")
        status = args[idx + 1]
    
    if fmt == "json":
        format_json(status)
    else:
        format_table(status)
