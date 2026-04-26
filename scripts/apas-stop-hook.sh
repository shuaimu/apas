#!/bin/sh
# Claude Code "Stop" hook for apas streaming auto-wake.
#
# Configured in ~/.claude/settings.json under hooks.Stop. Claude Code invokes
# this script every time the agent finishes a turn, passing JSON on stdin
# that includes the `session_id`. We extract that id and `touch` a marker
# file at /tmp/apas-stop-marks/<session_id>.
#
# The apas streaming worker's per-pane background-task watcher uses the
# marker's mtime as the authoritative "claude went idle at" timestamp:
# auto-wake prompts only fire for task output that grew AFTER claude stopped,
# so foreground bash output that's already part of the just-finished turn
# doesn't trigger noise.
set -e
mkdir -p /tmp/apas-stop-marks 2>/dev/null || true
# Read JSON from stdin and pull session_id. Use python3 (always present on
# zoo-005 hosts) to avoid a hard jq dependency.
sid=$(python3 -c 'import sys,json
try:
    print(json.load(sys.stdin).get("session_id", ""))
except Exception:
    pass
' 2>/dev/null) || sid=""
if [ -n "$sid" ]; then
    touch "/tmp/apas-stop-marks/$sid"
fi
exit 0
