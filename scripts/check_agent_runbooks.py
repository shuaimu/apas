#!/usr/bin/env python3
"""Require a regular AGENTS.md and reject retired runbook names.

The full contributor runbook lives in the root AGENTS.md. The web package's
AGENTS.md holds Next.js's generated API guidance so `next dev` does not scaffold
another CLAUDE.md pointer. Neither location needs a Claude-specific alias.
Duplicate or case-variant runbooks drift and break case-insensitive checkouts.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CANONICAL_DECLARATION = "Canonical contributor/agent runbook"


def check_runbooks(root: Path) -> list[str]:
    errors: list[str] = []
    for directory in (root, root / "packages" / "web"):
        agents = directory / "AGENTS.md"
        if agents.is_symlink() or not agents.is_file():
            errors.append(f"{agents.relative_to(root)} must be a regular file")
        elif directory == root and CANONICAL_DECLARATION not in agents.read_text(encoding="utf-8"):
            errors.append(f"AGENTS.md must declare itself the canonical runbook ({CANONICAL_DECLARATION!r})")

        if not directory.is_dir():
            continue
        for path in directory.iterdir():
            name = path.name.casefold()
            if name in {"claude.md", "agent.md"} or (name == "agents.md" and path.name != "AGENTS.md"):
                errors.append(
                    f"{path.relative_to(root)} is retired; keep AGENTS.md, not a duplicate or alias"
                )
    return errors


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    errors = check_runbooks(ROOT)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
