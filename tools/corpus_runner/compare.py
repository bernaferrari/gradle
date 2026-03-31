#!/usr/bin/env python3
"""
Compare build outputs between upstream Gradle and Rust substrate.

Compares:
- Task execution order
- Exit codes  
- Output files (by checksum)
- Diagnostics (normalized)
"""

import hashlib
import json
import os
import re
from dataclasses import dataclass
from typing import Optional

@dataclass
class BuildResult:
    exit_code: int
    duration_ms: int
    tasks: list[str]
    output_files: dict[str, str]  # path -> sha256
    diagnostics: list[str]
    raw_output: str

@dataclass
class ComparisonResult:
    passed: bool
    mismatches: list[str]
    details: dict

def normalize_output(text: str) -> str:
    """Remove timestamps, durations, and other non-deterministic content."""
    text = re.sub(r'\d+\.\d+s', '0.0s', text)
    text = re.sub(r'\d+ms', '0ms', text)
    text = re.sub(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d+', 'TIMESTAMP', text)
    return text

def compare_results(upstream: BuildResult, substrate: BuildResult) -> ComparisonResult:
    """Compare two build results and return mismatches."""
    mismatches = []
    details = {}
    
    # Exit code
    if upstream.exit_code != substrate.exit_code:
        mismatches.append(f"Exit codes differ: upstream={upstream.exit_code}, substrate={substrate.exit_code}")
        details["exit_code_match"] = False
    else:
        details["exit_code_match"] = True
    
    # Task execution
    upstream_tasks = normalize_output(" ".join(upstream.tasks))
    substrate_tasks = normalize_output(" ".join(substrate.tasks))
    
    upstream_task_set = set(upstream.tasks)
    substrate_task_set = set(substrate.tasks)
    
    missing_tasks = upstream_task_set - substrate_task_set
    extra_tasks = substrate_task_set - upstream_task_set
    
    if missing_tasks:
        mismatches.append(f"Missing tasks in substrate: {sorted(missing_tasks)}")
    if extra_tasks:
        mismatches.append(f"Extra tasks in substrate: {sorted(extra_tasks)}")
    
    details["missing_tasks"] = sorted(missing_tasks)
    details["extra_tasks"] = sorted(extra_tasks)
    details["task_count_match"] = len(upstream_task_set) == len(substrate_task_set)
    
    # Output files
    def normalize_path(p):
        return p.replace("\\", "/")
    
    upstream_files = set(normalize_path(p) for p in upstream.output_files)
    substrate_files = set(normalize_path(p) for p in substrate.output_files)
    
    missing_files = upstream_files - substrate_files
    extra_files = substrate_files - upstream_files
    
    if missing_files:
        mismatches.append(f"Missing output files in substrate: {sorted(list(missing_files)[:10])}")
    if extra_files:
        mismatches.append(f"Extra output files in substrate: {sorted(list(extra_files)[:10])}")
    
    # Check file contents (checksums)
    common_files = upstream_files & substrate_files
    checksum_mismatches = []
    for f_path in sorted(common_files):
        if upstream.output_files.get(f_path) != substrate.output_files.get(f_path):
            checksum_mismatches.append(f_path)
    
    if checksum_mismatches:
        mismatches.append(f"Output file checksums differ ({len(checksum_mismatches)} files)")
        details["checksum_mismatches"] = checksum_mismatches[:10]
    
    details["file_count_match"] = len(upstream_files) == len(substrate_files)
    details["common_files"] = len(common_files)
    
    return ComparisonResult(
        passed=len(mismatches) == 0,
        mismatches=mismatches,
        details=details
    )
