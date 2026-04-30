#!/usr/bin/env python3
"""Render corpus runner summaries into a small performance evidence report."""

import argparse
import json
from pathlib import Path


def percent_delta(upstream_ms: int, substrate_ms: int) -> str:
    if upstream_ms <= 0:
        return "n/a"
    delta = (substrate_ms - upstream_ms) / upstream_ms * 100.0
    return f"{delta:+.1f}%"


def render(summary: dict) -> str:
    upstream_ms = int(summary.get("upstream_duration_ms", 0))
    substrate_ms = int(summary.get("substrate_duration_ms", 0))
    project_count = int(summary.get("project_count", 0))
    matched = int(summary.get("matched_project_count", 0))
    no_fallback = int(summary.get("no_fallback_project_count", 0))
    upstream_tasks = int(summary.get("upstream_task_total", 0))
    substrate_tasks = int(summary.get("substrate_task_total", 0))

    return "\n".join(
        [
            "# Rust Substrate Corpus Performance",
            "",
            "| Metric | Value |",
            "| --- | --- |",
            f"| Projects matched | {matched}/{project_count} |",
            f"| No-fallback projects | {no_fallback}/{project_count} |",
            f"| Upstream task total | {upstream_tasks} |",
            f"| Rust substrate task total | {substrate_tasks} |",
            f"| Upstream observed wall time | {upstream_ms} ms |",
            f"| Rust substrate observed wall time | {substrate_ms} ms |",
            f"| Rust vs upstream wall-time delta | {percent_delta(upstream_ms, substrate_ms)} |",
            "",
            "Wall time is measured by the corpus runner around whole Gradle invocations.",
            "Use it as trend evidence, not a microbenchmark.",
            "",
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("summary", help="Path to corpus_summary.json")
    parser.add_argument("--output", default=None, help="Markdown output path")
    args = parser.parse_args()

    summary_path = Path(args.summary)
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    report = render(summary)

    if args.output:
        Path(args.output).write_text(report, encoding="utf-8")
    else:
        print(report, end="")


if __name__ == "__main__":
    main()
