#!/usr/bin/env python3
"""Verify there is exactly one agent runbook and that AGENTS.md points at it.

APAS used to keep three files beside `CLAUDE.md`: a hand-written `claude.md`
with a second copy of the deployment procedure, an `agent.md` pointing at it,
and an `AGENTS.md` generated from it. Two things went wrong with that.

The copies drifted. `claude.md` lost the nginx rules, the rolling deployment
order, and the system-administrator pre-check, so an agent that read it
deployed by an outdated procedure.

Worse, `claude.md` and `CLAUDE.md` differ only in case. A case-insensitive
filesystem — macOS and Windows by default — cannot hold both, and git writes
the index in byte order, so `claude.md` landed last and overwrote the real
runbook with a 3 KB note. Every clone on those platforms was silently wrong.

So now `CLAUDE.md` is the only runbook and `AGENTS.md` is a symlink to it.
Codex-style agents read AGENTS.md, follow the link, and get the same bytes,
with nothing to keep in sync.

Run with --write to repair the symlink.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CANONICAL_RUNBOOK = ROOT / "CLAUDE.md"
AGENTS_LINK = ROOT / "AGENTS.md"
LINK_TARGET = "CLAUDE.md"

CANONICAL_DECLARATION = "Canonical contributor/agent runbook"

# Names that once held a duplicate runbook. `claude.md` in particular must
# never come back: it collides with CLAUDE.md on a case-insensitive checkout.
RETIRED_RUNBOOKS = ("claude.md", "agent.md")


def check_symlink(*, write: bool) -> list[str]:
    """AGENTS.md must be a symlink resolving to CLAUDE.md."""
    if AGENTS_LINK.is_symlink() and os.readlink(AGENTS_LINK) == LINK_TARGET:
        if AGENTS_LINK.resolve() == CANONICAL_RUNBOOK.resolve():
            return []
        return [f"AGENTS.md points at {os.readlink(AGENTS_LINK)}, which is not the runbook"]

    if not write:
        if AGENTS_LINK.is_symlink():
            actual = f"a symlink to {os.readlink(AGENTS_LINK)}"
        elif AGENTS_LINK.exists():
            # The Windows case: git without core.symlinks writes a regular
            # file whose whole content is the target path.
            actual = "a regular file (a Windows checkout turns the link into one)"
        else:
            actual = "missing"
        return [f"AGENTS.md must be a symlink to {LINK_TARGET}; it is {actual}"]

    if AGENTS_LINK.is_symlink() or AGENTS_LINK.exists():
        AGENTS_LINK.unlink()
    AGENTS_LINK.symlink_to(LINK_TARGET)
    return []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="repair the AGENTS.md symlink instead of only checking it",
    )
    args = parser.parse_args()

    errors: list[str] = []

    if not CANONICAL_RUNBOOK.exists():
        print("CLAUDE.md is missing", file=sys.stderr)
        return 1

    if CANONICAL_DECLARATION not in CANONICAL_RUNBOOK.read_text(encoding="utf-8"):
        errors.append(
            f"CLAUDE.md must declare itself the canonical runbook "
            f"(the phrase {CANONICAL_DECLARATION!r})"
        )

    for name in RETIRED_RUNBOOKS:
        if (ROOT / name).exists():
            errors.append(
                f"{name} is back. CLAUDE.md is the only runbook; a second copy "
                f"drifts, and 'claude.md' also collides with 'CLAUDE.md' on a "
                f"case-insensitive filesystem"
            )

    errors.extend(check_symlink(write=args.write))

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        print(
            "Run `python3 scripts/check_agent_runbooks.py --write` to repair the symlink.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
