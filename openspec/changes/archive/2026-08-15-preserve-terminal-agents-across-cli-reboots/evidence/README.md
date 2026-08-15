# Verification evidence

Records for the two tasks whose completion is an observation rather than code:

- `8.4-staging-exercise.md` — the staging exercise: forced WebSocket loss,
  CLI replacement with terminal adoption, failed update preparation, project
  stop, and project deletion, with the process IDs recorded at each step.
- `8.5-rollout-verification.md` — the rollout checks: deploy ordering, health,
  mixed-version behaviour against a pre-capability CLI, and the pane-host
  audit showing no abandoned hosts.
- `8.5-pane-host-audit.txt` — raw output backing that audit.

Both exercises ran against a local server and an isolated CLI config so nothing
touched production; the harness itself was scratch and is described, not kept.
Two limits are stated in the records rather than glossed: the mixed-version
warning was verified at the protocol level without a browser screenshot, and
the 8.4 rig used stub provider binaries so that process lifecycle could be
measured without spending agent quota.
