#!/usr/bin/env python3
"""
Discovers Gradle projects in a directory tree and outputs them as a list.

Filters out projects that don't have the expected structure.
"""

import os
import sys
import json
from pathlib import Path

def discover_projects(root: str) -> list[dict]:
    """Walk directory tree and find Gradle projects."""
    projects = []
    root_path = Path(root)
    
    for build_file in root_path.rglob("build.gradle*"):
        project_dir = build_file.parent
        # Skip nested projects without root build file
        if project_dir.name.startswith("."):
            continue
        # Check if it has a settings.gradle
        has_settings = (project_dir / "settings.gradle").exists() or \
                       (project_dir / "settings.gradle.kts").exists()
        # Check src directory exists
        has_src = (project_dir / "src").exists()
        
        projects.append({
            "path": str(project_dir.relative_to(root_path)),
            "name": project_dir.name,
            "has_settings": has_settings,
            "has_src": has_src,
            "is_kotlin_dsl": build_file.suffix == ".kts",
        })
    
    return projects

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: discover.py <corpus_directory>")
        sys.exit(1)
    
    corpus_dir = sys.argv[1]
    projects = discover_projects(corpus_dir)
    print(json.dumps(projects, indent=2))
