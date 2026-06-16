#!/usr/bin/env python3
"""Fail if the retired manager-directives runtime channel returns."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (ROOT / "crates", ROOT / "packages")
FORBIDDEN = (
    "manager-directives.jsonl",
    "AddManagerDirective",
    "ManagerDirective",
)
SKIP_DIRS = {
    ".git",
    ".next",
    "coverage",
    "dist",
    "node_modules",
    "target",
}


def iter_source_files(root: Path):
    for path in root.rglob("*"):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        if path.is_file():
            yield path


def main() -> int:
    hits: list[str] = []
    for root in SOURCE_ROOTS:
        if not root.exists():
            continue
        for path in iter_source_files(root):
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for line_no, line in enumerate(text.splitlines(), start=1):
                for needle in FORBIDDEN:
                    if needle in line:
                        rel = path.relative_to(ROOT)
                        hits.append(f"{rel}:{line_no}: contains {needle}")

    if hits:
        for hit in hits:
            print(hit, file=sys.stderr)
        print(
            "The legacy manager-directives channel is retired; use project_goal.md "
            "sync via UpdateProjectGoal / ProjectGoalChanged instead.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
