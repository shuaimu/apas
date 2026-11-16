# APAS - Autonomous Programming Agent System

> Canonical contributor/agent runbook, and the only one. Keep architecture,
> local development, deployment, and workflow guidance here. `AGENTS.md` is a
> regular file read directly by Claude and Codex-style agents; verify it with
> `python3 scripts/check_agent_runbooks.py`. The former `CLAUDE.md` and its
> `AGENTS.md` symlink have been consolidated into this file.
>
> There used to be a hand-written `claude.md` holding a second copy of the
> deployment procedure, plus an `agent.md` pointing at it and an `AGENTS.md`
> generated from it. Both copies drifted — the duplicate lost the nginx rules,
> the rolling order, and the system-administrator pre-check — and worse,
> `claude.md` and `CLAUDE.md` differ only in case, so a clone on macOS or
> Windows could not hold both and silently overwrote this file with the 3 KB
> note. Do not reintroduce a second runbook under any casing.
>
> `packages/web/AGENTS.md` contains only Next.js's generated, package-scoped API
> guidance, not another copy of this runbook. Keep that managed block: without
> it, `next dev` recreates both a guidance file and a `CLAUDE.md` pointer.

APAS runs coding agents against a project, from a browser or a phone. The CLI
owns local panes and worktrees, the server brokers project/session/job state,
and the web UI exposes interactive panes and the separate Jobs/Attention workflow.

A project is a directory with an `.apas` file. Interactive work happens in
**panes**, each hosting one agent and controlled by a person. A person can
also leave one OMP terminal tab repeating a prompt on **autopilot** (see
"Autopilot on OMP terminal tabs"). **Autonomous
workers** own persistent, human-approved responsibilities and execute bounded
internal jobs with independent verification and draft-PR publication. Recurring
execution is default-disabled. Workers never orchestrate interactive panes or
restore the removed managed-team mode.

**Managed team mode was removed.** Four roles (Manager, Tech Lead, Developer,
Reviewer) used to coordinate through `project_goal.md`, `team-todo.md` and
`.apas-team.jsonl`, dispatching work to each other through an MCP server. It was
the largest feature in the CLI and, at removal, had zero managed panes anywhere
in the deployment. What it cost everything else was a second way to run a
provider: a second spawn path, its own status and review plumbing, and a policy
field carried at three levels. If you find a reference to a team role, the
scratchpad, the TODO queue or `apas mcp-server`, it is stale — say so rather
than reviving it.

Pane **`role` / `goal` / `backstory`** remain identity metadata anyone can set
from the role modal, composed into the system prompt by
`pane_identity::compose_system_prompt`.

**The "Start bot" deadloop is retired too.** Its prompt, provider, cadence and
conversation remain read-only history. Neither loading `.apas` nor reopening
that history restores the loop. There is no bot-to-worker conversion: review the
old responsibility and separately create and approve a new worker. Autopilot is
not the bot restored either: it is a loop a person starts inside one OMP
terminal tab's own session, it shares no code or state with the bot, and a
legacy bot pane cannot be put on autopilot.

**Raw Pi was retired as a provider on 2026-11-13; OMP, which is derived from
it, stays.** `Provider::Pi` and the web value `"pi"` survive only so an older
`.apas`, a stored roster, a stored policy that lists
`terminal:pi:official:default` and a frame from an older client still parse.
Nothing can create a Pi tab, switch a tab to Pi or launch one. If you find
code that launches `pi`, looks for sessions under `~/.pi` or reads `pi_path`,
it is stale. Names such as `PiTranscriptReader` and `PI_CODING_AGENT_DIR` are
not: OMP writes Pi's session format and reads Pi's overrides. See "Pi is
retired; OMP stays" under Terminal panes.

## Architecture

```
┌─────────────────────┐    WebSocket     ┌─────────────────┐    WebSocket    ┌─────────────────────┐
│ CLI client          │ ◄───────────────►│ apas-server     │◄───────────────►│ Web frontend        │
│ (apas binary)       │                  │                 │                 │ (Next.js Overview)  │
└─────────────────────┘                  └─────────────────┘                 └─────────────────────┘
        │                                         │
        │                                         ▼
        │                                ┌─────────────────┐
        │                                │ SQLite + JSONL  │
        │                                │ data/sessions/  │
        │                                └─────────────────┘
        ▼
┌─────────────────────┐
│ Pane processes      │
│ Claude/Codex/etc.   │
│ worktrees + prompts │
└─────────────────────┘
```

The CLI keeps project-local state in `.apas`, plus optional per-pane worktrees.
The server caches and broadcasts machine, session and pane state. The web UI
lets a person open panes, talk to them, inspect their diffs, and hand off to a
PR.

### Autonomous workers: deployment remains qualification-gated

The worker control plane lives in `crates/server/src/workers/` and
`db/workers.rs`; `jobs.rs`, `db/jobs.rs` and `crates/client-cli/src/jobs/` remain
the bounded execution, containment, verification and publication engine.
Canonical contracts are `crates/shared/src/workers.rs` and `jobs.rs`, generated
into `@apas/protocol`. `/projects/` serves their JSON APIs. Workers are not
`.apas` panes, terminal hosts, forever-running provider sessions or managed teams.

**No recurring worker/verifier image or publication profile has been
operationally qualified by this implementation.** Keep deployment recurrence
and host `[autonomous_jobs].enabled` off until the applicable gates in both
`openspec/changes/unify-autonomous-workers/tasks.md` and
`openspec/changes/add-autonomous-jobs/tasks.md` pass. Unit tests, Podman `info`,
synthetic retained records and Expo export do not establish those gates.

#### Responsibilities, backlog and bounded execution

A worker owns a versioned description and standing checklist, an exact approved
machine/profile/base branch, a bounded prioritized backlog, a latest checkpoint
and a durable inspection schedule. Approval covers future runs within those
bounds, not arbitrary instructions or a change of host. Mandate replacement
requires another preview and approval; each execution pins its own committed
base, mandate/backlog revisions, purpose, item and allowance reservation.

An **inspection** receives a read-only checkout and can report `no_work`; it
then becomes Idle with no provider process left running. A selected
**implementation** receives a writable checkout and an immutable item. Only
`candidate_ready` implementations enter independent verification and trusted
draft publication. Inspections, `no_change` and blocked outcomes never masquerade
as delivery. Each run creates a fresh bounded OMP session.

OMP writes one bounded structured result at `/session/apas-worker-result.json`.
The host reads it only after genuine root/child settlement and positive complete
termination. It rejects symlinks, non-regular/multiply-linked files, escaped
paths, oversized content and mismatched worker/revision/reservation identities.
Model proposals cannot overwrite intervening human backlog or mandate edits.
An empty backlog is valid; no default TODO prompt or guessed work is injected.

Rolling **24-hour elapsed time, reported spend and execution-count ceilings**
cover inspection, implementation, delegated work and retries. Admission reserves
the entire per-run allowance atomically. Unknown accounting is not zero, and an
unresolved reservation does not disappear at a window boundary. Per-item
cumulative attempts and consumed allowance survive pause, resume, edits and
restart. A due check waits for cadence, current authority, capacity and budget;
missed checks coalesce rather than replaying a burst.

One worker has at most one unresolved execution chain and one unresolved draft
PR. Waiting for review uses no model calls. A verified external merge permits
later inspection; a closed-unmerged or unexpectedly changed PR blocks automatic
continuation. Failed items require an explicit retry under their retained limits.

#### Browser and native workflow

The desktop tab bar's `+` and the mobile-browser pane sheet create an
interactive agent tab directly: the provider, then the **Run on** step where it
applies. The release of 2026-11-11 made `+` first ask **Interactive agent** or
**Autonomous worker**; that step was removed again at the owner's request. An
autonomous worker is created with **New autonomous worker** in the Overview
tab's Autonomous workers section, or from `/code/workers/`. Both open the
project-fixed mandate/checklist composer; an unavailable preferred machine is
explained, never silently replaced.
`/code/workers/` is the cross-project Workers/Attention surface. The web
reaches it by its address and by links to a worker; since 2026-11-14 the
project list, the collapsed rail and the phone's project list no longer show
Workers and Attention buttons, at the owner's request. Native uses the same
contracts in native screens, not a WebView, and keeps its own tabs for them.

Worker views sit beside interactive panes. **Overview** is visible even with
zero panes and lists responsibilities, backlog/status and paginated history.
Global inventory, project inventory, backlog and run history are independently
paged; a loaded subset is not a whole-project count. Selecting or hiding a view
does not close a terminal or change a worker's lifecycle.

**The Overview tab is one page, tabs first.** Its order is the project's name,
**Tabs** (one card per tab), **Usage**, **Resource use**, **Allowed launch
profiles**, then **Autonomous workers** with Earlier results. Workers used to
open the page, above an "Interactive project" block; they moved to the end
because the tabs are what is running and workers are off until a deployment
enables them. Every worker function is still there: the worker cards, New
autonomous worker, Refresh, load more, and Earlier results. The page has no
"Back to interactive agents" button, since the tab bar above it leads to every
tab; the worker, composer and earlier-result screens keep theirs.

- Tab cards follow the tab bar's order (the roster's), never pane id, and a
  card does not move when its tab's state changes. The card's remove control
  is **Close tab**.
- The Usage table lists open tabs first, most recently active first, then
  usage no tab accounts for, then one folded row for closed tabs that carries
  their sum, so the rows still add up to the totals. Counts use k, M, B or T
  (`lib/formatCount.ts`).
- A tab card's "Last active" and the Usage table's use one formatter
  (`lib/formatRelativeTime.ts`). Under a minute, or a server time ahead of the
  browser's clock, reads "just now".
- Allowed launch profiles are shown by their names from the provider catalog
  (`lib/launchProfileLabels.ts`), with the policy key as the tooltip. A key the
  catalog cannot name is shown as the key. A retired provider's key is left
  out, as on the Machines and admin pages.

The phone's project screen shows the same page with a shorter body: the
project's name, its tabs as rows that open them, then Autonomous workers.
Usage, Resource use and Allowed launch profiles are on the desktop page only.

`ProjectWorkersWorkspace` renders this page from the shared parts in
`components/overview/OverviewElements.tsx`, outside the Jobs stylesheet. The
composer, worker and earlier-result views are the Jobs pages' own screens and
keep that style. Tests outside the Overview's own files find the tab panels by
the region and heading "Interactive project"; the page keeps that heading for
screen readers only, so do not delete it without changing those tests.

**Pause after this run** fences new dispatch but lets the admitted run finish.
**Stop now** also requests whole-unit termination and fences publication; its
receipt is not proof of a stop. Resume preserves budgets and never silently
retries a failed item. **Check now** still respects cadence and allowance.
Questions, candidate checks and publication recovery retain exact worker/run
identities; an older review link never substitutes the latest candidate.

Saved drafts and actor-scoped receipts recover lost replies without automatic
POST replay. Reconnect rechecks authority; offline snapshots are marked stale
and read-only. Account changes and discovered revocation purge protected
browser/native caches. Routine idle checks do not create attention notifications.

`/code/jobs/` remains **Earlier results**, not a second admission/retry surface.
A positively stopped finite job or saved standalone draft may prefill a new
worker proposal, but never transfers its grant or skips fresh approval.
Historical bots cannot be converted or restarted. Their metadata alone does not
veto an unrelated new worker; known-live legacy runtimes must be drained before
rollout. User TODO/team files and unrelated interactive panes are left alone.

#### Provision the host, not a shared home-directory fallback

- Use a non-root Linux account with working rootless Podman, user namespaces,
  delegated cgroup-v2 CPU/memory/PID controllers and a private `XDG_RUNTIME_DIR`.
  A stale runroot belonging to another UID is a provisioning failure, not a
  reason to run privileged or bypass isolation.
- Configure `storage_root` (default `/var/lib/apas/users/<uid>/jobs`) on durable,
  host-local ext4, XFS, Btrfs or ZFS. Control directories must be canonical,
  owner-private and non-symlinked. NFS and tmpfs are not journal fallbacks.
- Provision `workspace_roots` as dedicated finite ext4/XFS filesystems, one
  independently reserved filesystem per active run. Each must have a distinct
  device from the other slots and from control storage; its total capacity must
  fit the profile's workspace allowance. Ordinary directories or multiple bind
  mounts of one filesystem do not provide this boundary. Do not reformat an
  existing user filesystem to satisfy it.
- Set both `podman_storage_root` and `podman_runroot` to private, canonical
  host-local directories outside every workspace slot. APAS uses an isolated
  `vfs` store, not the account's shared-home container database.
- Qualify actual CPU, memory, PID, filesystem and bounded-output exhaustion;
  read-only image and dropped privileges; host/home/metadata access denial;
  complete container/descendant termination; and lease expiry during transport
  loss. There is no unrestricted-process fallback when these checks fail.

#### Pin and qualify each execution profile

`deploy/autonomous-jobs/Dockerfile` requires an immutable
`TOOLCHAIN_IMAGE=registry@sha256:…` plus operator-supplied inputs under
`deploy/autonomous-jobs/inputs/`: a static APAS binary, Bun, and an OMP source
archive containing its already-resolved dependencies. Supply their
`APAS_SHA256`, `BUN_SHA256` and `OMP_SOURCE_SHA256` build arguments. The image
performs no network package installation. The current worker wrapper is pinned
to **OMP 18.18.0**; another version requires fresh lifecycle qualification.

Configure `profiles`, `models`, `verifiers` and `publishers` under the CLI's
`[autonomous_jobs]` configuration; exact fields are in `jobs/config.rs`.
Workers and independent verifiers require immutable image references. Worker
profiles bind model/verifier keys, publisher, finite `JobLimits` and explicit
dependency-host allowlists. Model profiles declare their API, exact model,
qualified context/reasoning capabilities, fixed upstream and per-million token
prices. Verification checks are named argv arrays with finite timeouts, not
worker-authored success claims.

Model and publisher `credential_file` paths must name private operator-owned
regular files, mode `0600`. Use dedicated, least-privilege credentials, never
personal provider/GitHub state inherited from the daemon's home. Publisher
profiles allowlist repositories and protected base refs and require
`ci_qualified`: even a draft PR can execute repository CI.

Each worker's `qualification_file` records the exact resolved profile digest
computed by `worker_profile_digest`, a non-future `qualified_at`, and successful
`omp_rpc`, `child_questions` and `gateway_metering` evidence. Recurring execution
also requires `execution_protocol_version = 2`, the exact `bootstrap_digest`,
and `readonly_inspection`, `inspection_results` and `implementation_results`
evidence. Record these only after exercising the pinned image with actual
root/child work, question confirmation without timeout answers, aggregate usage,
genuine settlement and both result dispositions. Changing the worker,
bootstrap or resolved model/verifier/publisher profile invalidates qualification.
Never create affirmative evidence merely to make readiness green.

The worker has no external network route. Its run-scoped bridge reaches the
trusted gateway over a Unix socket; only fixed model upstreams and explicit
dependency hosts are reachable. Provider credentials remain in the gateway,
and publication credentials are used only by the separate publisher. Neither
worker nor verifier receives the shared repository's `.git`, user home or
daemon control socket. Readiness is timestamped and expires; launch-sensitive
checks repeat rather than trusting a stale green badge.

#### Authority, evidence and recovery

Deployment policy (`/admin/job-policy`) and its separate `recurring.enabled`
grant default disabled. Finite-job enablement never implies recurring authority.
Cluster/project overrides may only narrow enablement, profile allowlists,
capacity and ceilings across every hosting cluster; the minimum inspection
interval can only increase. Widening writes are rejected, not silently clamped.
The system-administrator credential controls deployment policy; hosting operators
control their cluster/project overrides. Project content access alone is not
compute authority. Management forms distinguish requested and effective policy.

A mandate approval pins its branch and target; execution admission resolves and
pins the committed base and immutable task with the current policy fingerprint.
Actor-scoped receipts survive restart. Recover a lost reply from its original
receipt instead of resubmitting a prompt or answer. Unknown ownership becomes
`recovery_required`, never a replacement attempt. Revocation fences future work
and stops active work; restoring authority does not silently resume a worker.
An answer stays pending until the provider confirms it. Recommendations are not
answers, and `ask.timeout=0` forbids timeout defaults.

The independent host enforces a **120-second lease**, renewed every **20 seconds**
while authority remains valid, plus cumulative elapsed/attempt/spend limits.
All root and delegated model requests share gateway accounting. Spend is a
reported-usage threshold, **not a guaranteed invoice cap**: in-flight calls can
overrun it, and missing required metering blocks further dispatch. Do not reset
consumption when a daemon re-execs or a client reconnects.

Only an immutable retained candidate verified in a fresh independent container
can proceed to the credential-separated publisher. Checks bind the exact
candidate SHA; a worker's own test output is insufficient. Publication creates
a draft PR on `apas/job-<uuid>`, never merges, and reconciles uncertain replies
without blindly posting another PR.

Per-run event replay and artifacts retain the existing 30-day policy; clients
display explicit expiry. Worker mandates, checkpoints, accounting and unresolved
execution/publication ownership are not discarded with replay history.
Cancel/stop receipts prove requests, not termination. A run releases its slot
only after positive whole-unit stop evidence. Project deletion also waits for
host purge acknowledgement for every dispatched historical attempt; an offline
host leaves durable cleanup pending across server restart. Never delete its
journal or fabricate an acknowledgement to unblock deletion.

#### Rollout, rollback and observed verification

Roll out **server/shared protocol → web and native clients → CLI/daemon**, with
recurrence and host execution disabled throughout qualification.
`autonomous_workers_v1` gates worker-aware clients; `omp_persistent_worker_v1`
and execution protocol v2 gate new host execution. The older
`autonomous_jobs_v1` / `omp_job_worker_v1` and v1 execution protocol remain for
legacy evidence/drain, not new standalone admission or recurring execution.
Interactive terminal capabilities and historical pane behavior are independent.
Existing mobile bootstrap/mutation/notification/deep-link flags still apply;
device gates are in `docs/mobile-development-and-operations.md`.

Before enabling, positively drain known-live legacy bot/finite runtimes, qualify
the pinned profiles and approve each new responsibility explicitly. Historical
records are not operational qualification. Predecessor task-only requirements
have been reconciled in its specs; synchronize/archive the related changes only
after their remaining acceptance gates are satisfied.

For rollback, disable new recurring admission first. Keep capable controllers
and hosts until active execution is positively stopped and required historical
host purges are acknowledged; preserve SQLite, receipts, journals and evidence.
An unreachable host remains pending cleanup. Only then install older binaries;
hiding UI controls or downgrading never proves a disconnected run stopped.
Production deployment, policy enablement and approval of real workers each
require a separate authorized operational action.

Local verification on **2026-11-08** passed the Rust workspace (761 tests), static
musl CLI build, protocol/terminal tests, web lint/typecheck/715 tests and
production build, native lint/typecheck/81 tests and Android export, and clean
npm/pnpm dependency audits with zero findings. Real Chromium checks covered
320/390/768/1440px layouts, question/composer draft reloads, offline editing,
narrow diff paging, policy editing and immediate revocation/cache purge. A local
nginx instance routed job pages and JSON correctly; real server restarts retained
receipts and refused to erase a synthetic offline-host record without stop
evidence. Minimal legacy WebSocket authentication also succeeded.

Those checks did **not** run an actual qualified worker, upstream model request,
independent verifier or draft-PR publication, and did **not** establish physical
mobile-browser keyboard/safe-area or iOS/Android interaction accessibility.
The test host lacks dedicated workspace filesystems and approved credentials,
images, repository and devices. No qualified image digest or production
enablement is implied by the local checks.

On **2026-11-09**, server and web **26.11.0** were deployed to production with
job admission still disabled. Public job pages/JSON, 21 referenced static assets,
and the Jobs UI passed smoke checks; an existing terminal reattached with the
same running PTY identity. No project daemon was restarted and no native build
or autonomous execution profile was enabled by that rollout.

On **2026-11-11**, the project-scoped chooser and embedded job workspace were
deployed server-first, then as a matching web/protocol pair. The commit-count
version remains **26.11.0**; this release is identified by
**20261111T052521Z**, with rollback artifacts under
`/opt/apas/backups/deploy-20261111T052521Z`.
Clean npm/pnpm graphs, the staged production build, eight public routes,
20 referenced static assets and authenticated project-scoped job JSON passed.
The live desktop/mobile-browser composer and project history worked, with
admission visibly unavailable. All nine project clients reconnected, including
two reporting 26.10.9, and the APAS terminal retained the same running PTY
identity. No project daemon/provider was restarted, no native build was released,
and no job policy or execution profile was enabled. Worker/device qualification
remains incomplete.

On **2026-11-12**, the persistent-worker cutover passed local server/protocol,
browser and native checks, plus production web build and Android export.
Actual Chromium covered 320/390/768/1440px layouts, separate project/global
navigation, zero panes, worker/run pagination, exact earlier candidate links,
offline draft recovery, discovered revocation/cache purge, deliberately delayed
view-preference hydration and unapproved finite-job reuse. Synthetic terminal
frames verified that worker navigation retains the same xterm instance and
sends no pane lifecycle/input commands. No real provider or PR was exercised.

That smoke caught two real integration faults: inline job/worker snapshots
amplified the WebSocket dispatcher's stack past the default Tokio stack, and a
retained mobile terminal stayed visible after another pane was selected.
Snapshot payloads are now boxed without changing JSON; the terminal remains
mounted but is visible only for its selected pane. Behavioral regressions cover
both. Exact check counts and the still-open host/device qualification gates are
recorded in `openspec/changes/unify-autonomous-workers/tasks.md`. This local work
did not deploy, enable a policy, approve a real worker or satisfy those gates.

**Rolled out on 2026-11-14.** Four changes reached production together:
`+` creating an interactive agent directly, the retirement of Pi, the rebuilt
Overview tab, and the prompt templates in the Start autopilot dialog, with the
smaller fixes made beside them (the last `team_available` reads in server, web
and CLI, and the one-machine reboot confirmation). The design record is
`openspec/changes/archive/2026-11-13-refine-interactive-workspace/`.

- **Server** has run the new binary since 00:25 UTC, and **web** release
  `20261114T002349Z` (version 26.11.36, built at 58a548d) since 00:25 UTC.
  Backups are under `/opt/apas/backups/deploy-20261114T002349Z`. The
  dependency graphs had not changed since the previous release and audited
  clean again on the host. nginx did not change: no route was added.
- The first start of the new server rewrote the stored launch policies
  without `terminal:pi:official:default`: the deployment default, one
  cluster default and nine project overrides. None is left empty; the
  smallest allows one profile. All nine project instances and three daemons
  registered again by themselves, and the server logged no error and no frame
  it could not parse.
- Ten public routes answered `200` with no redirect, the 18 static assets the
  pages reference answered `200`, and `/projects/by-remote` and
  `/projects/workers` answered JSON from `apas-server` with and without a
  credential. Viewers still negotiate `tab_autopilot_v1`. No job policy row
  exists, so recurring execution stays disabled. The live Overview shows the
  new layout with the owner's data.
- **CLI:** `master` was pushed at 58a548d. No daemon was restarted. Until a
  machine's daemon is rebooted from the Machines page, it runs the previous
  CLI: that CLI would still start a saved Pi tab when its project starts,
  although the server already refuses to create or relaunch one. Nothing
  else depends on the reboot. No tab of the owner's projects uses Pi.

**What was checked before the rollout.** At 58a548d the complete gates pass:
`cargo test` for shared (92 + 3 + 1), server (457) and CLI (596, 1 ignored),
the static musl CLI build, the protocol's 68 tests and typecheck, the
extension's 137 node tests, the whole web suite (1,242 tests) with lint,
typecheck and the production build, native (136 tests and typecheck),
`openspec validate --all --strict` and the runbook check. No two-host smoke
was run for these changes. While each work stream was still open, its own
checks were:

- Pi retirement: `cargo test` for shared (92 + 3 + 1), server (456) and CLI
  (595, 1 ignored as before); the protocol's 68 tests, the extension's 137
  node tests, and a schema regeneration that changed nothing more; the web
  tests of the tab bar, workspace, phone screen, sidebar rail, store, tab
  types and provider options (441 in 31 files), typecheck and lint. In the read-only viewer at
  1440 px and 390 px, `+` and Add agent list no Pi.
- `+`: the same web tests; the viewer showed `+` opening the provider list at
  once on the desktop, and Add agent opening on Run on and the profiles on a
  390 px phone.
- Overview: its own tests (87 in ten files), typecheck and lint; the viewer at
  1440x900 light and dark and 1024x768 on the real project (768x1024 only
  before the last Overview edits), the worker composer opened from New
  autonomous worker, and the phone's Overview at 390 px in the light theme.

Not exercised:

- **a saved Pi tab anywhere but in unit tests.** No tab of this deployment uses
  Pi and the viewer is read-only, so its notice, its exited state, the Reboot
  CLI refusal and closing it were not seen in a browser or on a real CLI. The
  CLI test of the pane-host cleanup uses hosts whose tmux sessions do not
  exist: it proves a Pi host's runtime files are removed and an OMP host's are
  kept, not that a live Pi agent is ended. Production was not checked for Pi
  tabs;
- worker cards with real workers: they were seen only with stand-in data the
  viewer answered in the browser, and Earlier results only in unit tests;
- in a browser: a project with no tabs, a legacy agent-kind tab, a tab with a
  worktree or a role, the access-changed state, and the phone's Overview at
  360 px;
- a real phone, iOS and the on-screen keyboard, and any real OMP launch.

### Project sidebar ordering

The expanded web sidebar has drag handles on repository headers and, when a
repository contains several projects, on its project rows. Drag a header to
move the whole group; drag a project within its existing group. The blue line
marks insertion before or after the target. A focused handle also accepts the
Up/Down arrow keys. Reordering never opens, moves, or changes access to a project,
and the collapsed icon rail uses the same order.

This is a browser-local, per-account layout preference, stored under
`apas_project_order:<user-id>` by `lib/useProjectOrder.ts`, not server policy.
It survives reloads and updates other tabs in the same browser, but does not
sync between devices. Saved project positions use the stable `.apas` project
id rather than the current representative session id. Unordered lists retain
the existing activity/recency sort; a saved order takes precedence, with newly
discovered entries following known ones. Temporarily missing entries keep their
saved positions without creating phantom projects. The Idle sessions view
continues to use its own waiting/recency ordering.

### Project list pane status

Desktop sidebar rows and mobile All projects cards share `PaneStatusDots`,
with one dot per pane: blue for Working, red for Pending answer, and gray for Idle.
Pending answer takes precedence when both status flags are set. Stopped projects
retain gray dots labeled Not running. Hover titles and accessible labels name each pane
and its status; crowded rows wrap rather than hide panes.
Desktop dots sit beside the activity label; mobile dots follow the hostname
or project path. A tab that is on autopilot has a ring around its dot and
", on autopilot" in its label; the dot's colour still says Working, Pending
answer or Idle. A paused or ended autopilot adds nothing to the dot.

The dots use each displayed session's reported pane roster and canonical
`is_working` / `awaiting_answer` flags, not the project's aggregate working
state or inferred message history. An empty or unreported roster adds no
placeholder dots. Mobile uses bootstrap pane details until a live roster arrives;
an explicitly empty live roster clears the dots instead of restoring stale
bootstrap panes. Desktop shared-project attribution stays on its own line so
the activity label and pane statuses remain visible.

A tab whose machine is not running the project, while another machine runs it,
keeps a gray dot labeled "Not running on <host>" and is left out of the idle
and waiting-agent lists: nobody can act on it until that machine runs the
project again.

## Project Structure

```
apas/
├── crates/
│   ├── client-cli/      # CLI binary (apas)
│   │   ├── src/
│   │   │   ├── main.rs        # CLI entry point and config commands
│   │   │   ├── config.rs      # User/machine config and supported backend settings
│   │   │   ├── project.rs     # .apas project metadata
│   │   │   ├── project_instances.rs # One machine's share of a project: pane tags, adoption, locked .apas merge
│   │   │   ├── project_join.rs # `apas register` joining a checkout by git remote
│   │   │   ├── daemon_registry.rs # Peer daemons and per-machine project claims on the shared config dir
│   │   │   ├── pane_identity.rs # A pane's role/goal/backstory as a system prompt
│   │   │   ├── claude_session_hook.rs # SessionStart hook: which transcript a pane writes
│   │   │   ├── worktree.rs    # Isolated worktree creation/diff/cleanup
│   │   │   ├── claude.rs      # Claude process wrapper
│   │   │   ├── terminal_pane.rs # Pty host for kind:"terminal" panes (portable-pty)
│   │   │   ├── omp_activity.mjs # Extension loaded into every OMP tab: activity, questions, autopilot loop
│   │   │   ├── omp_autopilot.rs # Autopilot: files between the CLI and a tab, request exchange, state relay
│   │   │   └── mode/
│   │   │       ├── dual_pane.rs # Interactive pane runtime and transcript watchers
│   │   │       ├── hybrid.rs    # Legacy single-pane local CLI + streaming
│   │   │       ├── local.rs     # Offline mode
│   │   │       └── remote.rs    # Remote-only mode
│   │   └── Cargo.toml
│   │
│   ├── server/          # WebSocket server (apas-server)
│   │   ├── src/
│   │   │   ├── main.rs      # Server entry point
│   │   │   ├── state.rs     # AppState with DB, sessions, storage
│   │   │   ├── storage.rs   # File-based message storage (JSONL)
│   │   │   ├── db/          # SQLite database
│   │   │   ├── session/     # Session manager (instances.rs: a session's per-machine attachments;
│   │   │   │                #   pane_launches.rs: pane ids and new panes in flight;
│   │   │   │                #   autopilot.rs: each OMP tab's reported autopilot and request routing)
│   │   │   └── routes/
│   │   │       ├── project_join.rs # GET /projects/by-remote and clone-to-join
│   │   │       ├── ws_cli.rs  # CLI WebSocket handler, pane/session replay
│   │   │       └── ws_web.rs  # Web WebSocket handler, Overview/machine actions
│   │   └── Cargo.toml
│   │
│   └── shared/          # Shared types between CLI and server
│       ├── src/
│       │   ├── lib.rs
│       │   ├── autopilot.rs # Autopilot wire types: spec, actions, state, summary, marker
│       │   └── messages.rs  # Shared WebSocket/machine message types
│       └── Cargo.toml
│
├── packages/
│   └── web/             # Next.js web frontend
│       ├── src/
│       │   ├── app/
│       │   │   ├── layout.tsx
│       │   │   └── page.tsx
│       │   ├── components/
│       │   │   ├── overview/         # The Overview page and its shared parts
│       │   │   ├── workers/          # Responsibilities, backlog and exact-run results
│       │   │   ├── tabs/             # Pane tabs, task bars, diff modal
│       │   │   │   └── TerminalPane.tsx  # xterm.js view for kind:"terminal" panes
│       │   │   ├── chat/             # Message display
│       │   │   ├── code/             # Code blocks
│       │   │   └── tools/            # Tool cards
│       │   └── lib/
│       │       ├── store.ts          # Zustand state, WebSocket message handling
│       │       ├── terminalBus.ts    # Pty frame fan-out (deliberately NOT zustand)
│       │       ├── projectInstances.ts # A session's instances, one per machine
│       │       ├── tabHosts.ts       # Which machine runs each tab, and why a tab cannot be used
│       │       ├── machineChoice.ts  # The "Run on" step for a new tab
│       │       ├── autopilot.ts      # Autopilot: frame parsing, the control's model, texts
│       │       ├── autopilotTemplates.ts # Built-in prompts the Start autopilot dialog offers
│       │       ├── launchProfileLabels.ts # Catalog names for launch-profile keys and providers
│       │       ├── formatCount.ts    # Compact counts and money for tables
│       │       ├── formatRelativeTime.ts # "5m ago", one formatter for cards and tables
│       │       └── mobileSelectedPane.ts # Which pane a session screen opens on
│       └── package.json
│
├── data/                # Runtime data (created at runtime)
│   ├── apas.db          # SQLite database
│   └── sessions/        # Message storage
│       └── {session-id}/
│           └── messages.jsonl
│
├── Cargo.toml           # Workspace root
└── AGENTS.md            # This file
```

## Build Commands

```bash
# Build all Rust crates
cargo build

# Build specific crate
cargo build -p apas          # CLI
cargo build -p apas-server   # Server
cargo build -p shared        # Shared types

# Run server
cargo run -p apas-server

# Run CLI (in a project directory)
cargo run -p apas

# Run CLI in offline mode (no server)
cargo run -p apas -- --offline

# Web frontend (from packages/web/)
npm install
npm run dev
```

The web lint gate imports `eslint-plugin-react-hooks` and `typescript-eslint`
directly; it does not use `eslint-config-next`. That unused preset was removed
because its glob dependency brought in an unpatched `braces` vulnerability.
Do not restore the package without an actual configuration need and clean
audits. Keep both lockfiles patched and verify both complete dependency graphs,
including development dependencies, before deployment.

Typography currently pins the vulnerable `postcss-selector-parser` 6.x line.
The scoped npm and pnpm overrides select 7.1.6; both lockfiles also carry
patched `source-map-js` and `sharp` resolutions. Do not remove those fixes
without clean full-graph audits or use the audit tool's forced typography
downgrade as a substitute.

### CLI build target (static musl)

The shipped CLI is a **static musl** binary so a self-update rebuild never
leaves `apas` depending on the build machine's glibc version. `install.sh` and
the self-updater (`crates/client-cli/src/update.rs`) build it with
`--target <arch>-unknown-linux-musl`; the server and plain `cargo build` stay
on the host's glibc target (musl is deliberately **not** a global
`build.target`, which would drag the server's bundled SQLite/ring through musl
too). The CLI's TLS is rustls (not native-tls/OpenSSL) so nothing but ring's
small C shim needs the musl toolchain.

Building the CLI as it ships needs the musl target plus a musl C compiler:

```bash
rustup target add x86_64-unknown-linux-musl   # (aarch64-… on ARM)
sudo apt-get install -y musl-tools            # provides musl-gcc + musl-dev

cargo build --release --target x86_64-unknown-linux-musl -p apas
```

`.cargo/config.toml` points ring's C build at `musl-gcc` for the musl targets.
If the musl toolchain is missing, `install.sh` and the self-updater fall back
to a glibc build so an update can never brick the binary.

## Configuration

### CLI Config
Located at `~/.config/apas/config.toml`:
```toml
[remote]
server = "wss://apas.mpaxos.com"
token = "your-token"

[local]
claude_path = "claude"
```

### Project creation from the web

General **New project** actions in the desktop sidebar, collapsed rail, mobile
home, and Machines page offer **Clone from GitHub** and **Existing folder**.
Repository-group **New instance** remains clone-specific.

- **Clone from GitHub** keeps the existing clone, new-branch, and **Create &
  start** behavior, including shared-cluster provisioning policy.
- **Existing folder** registers a directory in place and does not start it.
  The path belongs to the selected machine, not the browser: use an absolute
  path or `~/` relative to the daemon's OS user. The directory must exist.
  GitHub checkouts, other Git hosts, repositories without an origin, and plain
  non-Git directories all work. Nothing clones, initializes Git, switches
  branches, edits remotes, or changes source files or `.gitignore`.

Only the machine's authenticated owner can register a local folder. Project
access, cluster membership, and permission to provision public clones do not
grant this authority. The target must be connected and advertise
`local_project_registration_v1`; absent capability means unavailable, including
on older daemons. Deploy server → web → CLI before using the flow.

Missing `.apas` metadata is created with zero panes, recovering a registered
identity for that canonical path when one exists. Valid existing metadata keeps
its exact bytes, identity, name, pane configuration, policy, and unknown fields.
Malformed, unreadable, symlinked, or non-regular metadata is rejected, never
repaired or deleted. Directory symlink aliases and repeat requests reuse the
same identity; conflicting ID/path registrations fail under the registry lock
instead of retargeting another project. A failed registry write retains newly
created metadata so a manual retry can complete without changing identity.

Filesystem registration alone is not success. The server binds the result to
the requesting connection and exact daemon, finalizes canonical identity and
direct hosting placement through `authorize_project_registration` under the
project-operation guard, resolves policy, and checks the reported inventory.
Existing ownership and policy are preserved; unavailable or unauthorized
identities fail without deleting local files. No session is fabricated.

The modal retains progress, errors, and the canonical success path. A lost
connection leaves registration unconfirmed rather than replaying the request.
**View on Machines** selects the owning cluster and target project even when a
shared cluster was previously selected. Use the existing **Start** control
there explicitly; saved panes only launch on that action. An already-running
project remains running unchanged.

**Registration is enough to appear in project lists.** The expanded desktop
sidebar, collapsed rail, and mobile **All projects** include registered projects
from owned-machine inventory before their first session. They show **Stopped**
or **Running** from that inventory and link to **View on Machines**, never to a
fabricated session. Opening the link does not start the project. A real session,
when one arrives, replaces registration-only navigation; duplicate registrations
across hosts collapse by stable project ID. A reported running placement wins;
otherwise a stopped registration prefers the machine explicitly selected during
successful registration. Alphabetical host order is only the fallback when no
preference exists or the preferred machine is absent from authorized inventory.

The preference is nullable `project_cluster_placements.preferred_machine_id`,
scoped by project and hosting-cluster owner. It is navigation intent, not a runtime
claim or access grant. Explicit successful registration on another owned machine
can change it; ordinary daemon/session registration and raw heartbeat fields
cannot. The server supplies it to bootstrap and live inventory from an
owner-scoped cache, hydrated at daemon connection and refreshed by ListMachines
or mobile bootstrap. A deliberate database correction therefore takes effect on
refresh without starting a project. The nullable migration and optional wire
field require server then web deployment, not daemon/provider restarts.

Member-only cluster inventory does not seed these entries: compute visibility
does not grant project-content access. Existing authorized shared sessions remain
visible. Mobile uses bootstrap inventory until the first live machine list;
an explicitly empty live list clears registrations rather than resurrecting
stale bootstrap entries. **Idle sessions** still lists sessions only.

### Project Identification
Each project directory gets a `.apas` file with project metadata and restored
pane state:
```json
{
  "id": "uuid",
  "name": "project-name",
  "created_at": "2024-01-01T00:00:00Z",
  "auto_approve_todos": false,
  "auto_merge_prs": false,
  "panes": [
    {
      "pane_id": 440,
      "role": null,
      "mode": "interactive",
      "managed": false
    },
    {
      "pane_id": 612,
      "role": null,
      "mode": "interactive",
      "kind": "terminal",
      "provider": "codex",
      "managed": false
    }
  ]
}
```

A newly created project has **no panes**. The user opens what they want; a
default Claude pane meant every fresh project immediately spawned an agent
process nobody asked for. Three places used to force one into existence and all
three are gone: the `ProjectMetadata` constructors, an "ensure there is always
at least one pane" backfill at CLI launch, and `migrate_legacy`. That last one
was the real blocker — it runs on *every* load via `get_or_create_project` and
refilled any empty pane list with **two** legacy panes, so "no panes" was not
representable at all. It is now gated on the legacy `deadloop_claude_session_id`
/ `interactive_claude_session_id` fields actually being present, which is what
distinguishes a pre-`panes` file from a new project.

**Zero panes is a state, not an absence, and saying so takes three pieces.**
A project starts with none and returns to none when the last pane is closed,
which the UI allows. The trap is that "the roster is empty" and "no roster has
arrived" look identical everywhere unless they are deliberately kept apart:

- `Storage::load_pane_roster` answers `Option`, where `None` is a legacy
  session that never persisted one. `load_pane_list` still flattens both for
  callers that only want whatever panes exist.
- The server sends `PaneList` for a *known* roster even when it is empty, and
  infers panes from message history only when the roster is genuinely unknown.
  Staying silent on empty left web clients showing panes the user had closed;
  inferring on empty fabricated them back.
- The web store's `paneListReceived` is the same distinction client-side. With
  it false the UI synthesizes tabs from messages, as it must before the CLI
  reports; with it true an empty roster means an empty project. Closing the
  last pane with the two conflated rebuilt that pane as a ghost tab from its
  own leftover messages.

Anything that reads `panes.is_empty()` to mean "not reported yet" reintroduces
this. The pane-keyed records (`paneMessages`, `paneStatuses`, `paneHasMore`)
are pruned to the roster for the same reason, and because pane ids are reused.

`auto_approve_todos`, `auto_merge_prs` and `disallowed_tab_types` are
project-level policy flags, settable by the project owner or the operator of the
cluster hosting it. `disallowed_tab_types` restricts which tab types users may
create (see "Tab-type policy"); the other two were read by the team loop and are
now inert, kept so an older `.apas` still parses.

`managed: true` on a pane is likewise vestigial. It marked a pane as belonging
to the managed team rather than to you, and it used to gate real behaviour:
refusing PR creation, exempting the pane from the retired-agent-kind rule,
forcing Claude panes to `effort: max`, and gating relaunch on team availability.
Two boot-time migrations even *created* it, promoting any pane whose role
mentioned "manager", "tech lead" or "reviewer" — so naming a pane "reviewer" in
the role modal silently made it managed. None of that remains; a test asserts
loading a project never marks a pane managed.

New work is created as a terminal pane. `kind` defaults to `"agent"` when absent
solely for compatibility, so `.apas` files written before terminal panes existed
keep loading unchanged — see "Terminal panes" under Key Concepts. A `.apas` may
also still carry `team_enabled`, a `role` like `"team manager"`, or
`managed: true`; all three are ignored, and such a pane loads as an ordinary
pane.

## Message Types

Key message types in `crates/shared/src/messages.rs`:

- **CliToServer**: Register, SessionStart, StreamMessage, UserInput,
  Heartbeat, ProjectFlagsChanged, TerminalOutput, TerminalExited, machine
  config/status updates.
- **ServerToCli**: Registered, SessionAssigned, Input, Signal,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, pane/worktree/suggestion actions.
- **WebToServer**: Authenticate, ListCliClients, AttachSession, Input,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, TerminalAttach, machine/provider config
  actions.
- **ServerToWeb**: Authenticated, CliClients, SessionMessages, StreamMessage,
  UserInput, ProjectFlagsChanged, Machines, PaneDiff, TerminalOutput,
  TerminalSnapshot, TerminalExited.

The `Terminal*` family is the pty byte channel for `kind: "terminal"` panes and
is deliberately separate from `Output` / `StreamMessage` — see "Terminal panes"
under Key Concepts for why.

Running one project on several machines adds `ServerToWeb::SessionInstances`,
`AddPane { machine_id, request_id }` answered by `ServerToWeb::AddPaneResult`,
`ForgetMachinePanes`, `CreateProjectInstance { join_project_id }`, the
`all_instances` flag on project Start/Stop, `DaemonToServer::ProjectRuntimeStarted`,
and the input `MutationAck`. Autopilot adds `ControlPaneAutopilot` (web to
server to the tab's CLI), `PaneAutopilotReceipt` and `PaneAutopilotState` (CLI
to server to web), and `negotiated_capabilities` on `ServerToCli::Registered`. Every `ServerToCli` variant is classified by
`ServerToCli::scope()`, which routes it; see "A project's tabs can run on several
machines".

`WebToServer::UpdateProjectFlags` carries the project policy flags from the web
to the server. The server **rejects the whole message from anyone who is neither
the project owner nor the operator of the cluster hosting it**
(`ws_web::can_manage_project_settings`).
It then forwards `ServerToCli::UpdateProjectFlags` to the CLI for `.apas`
persistence; the CLI emits `CliToServer::ProjectFlagsChanged`, and the server
broadcasts `ServerToWeb::ProjectFlagsChanged`. The CLI also re-broadcasts the
flags from `.apas` every 5s, so a web client attaching mid-session hydrates
without asking.

## Data Storage

- **SQLite** (`data/apas.db`): Users, CLI clients, sessions metadata
- **JSONL files** (`data/sessions/{id}/messages.jsonl`): Chat messages per session
- **Worktrees** (`.apas-worktrees/pane-<id>/`): isolated branches for a pane
  working in isolation

A project that predates the team-mode removal may still hold `project_goal.md`,
`team-todo.md` and `.apas-team.jsonl` on disk. Nothing reads or writes them —
they are left alone rather than deleted, because they are the user's files.

## Development

### Running locally
```bash
# Terminal 1: Server
RUST_LOG=info cargo run -p apas-server

# Terminal 2: Web frontend
cd packages/web && npm run dev

# Terminal 3: CLI (in any project directory). Registers the project and exits;
# start it from the web. `--attach` opens the terminal UI for one already
# running here.
cargo run -p apas
```

### Environment Variables
- `RUST_LOG`: Logging level (e.g., `info`, `debug`)
- `NEXT_PUBLIC_WS_URL`: WebSocket URL for web frontend (default: `wss://apas.mpaxos.com`)

## Deployment

### Production Server

The APAS server and web UI are deployed on an LXC container:

- **Host**: `apas.mpaxos.com` (130.245.173.82)
- **SSH**: `ssh root@apas.mpaxos.com`
- **Edge**: **nginx** on port **80** reverse-proxies everything. The config is
  version-controlled at **`deploy/nginx-apas.conf`** and deployed to
  `/etc/nginx/conf.d/apas.conf` — edit the repo copy, `scp` it, `nginx -t`,
  then `systemctl reload nginx`. `/ws/ /auth/ /admin/ /share/ /health` →
  `apas-server` (`127.0.0.1:8080`), along with `/cluster/ /projects/
  /mobile/`; everything else → the Next.js app
  (`127.0.0.1:3000`). This exists so the web's WebSocket + HTTP API ride the
  standard port 80 (`wss://apas.mpaxos.com/ws/web`) instead of the non-standard
  `:8080`, which mobile carriers/Wi-Fi block — that broke mobile entirely.

  **A proxied `location /X/` shadows the page at `/X`.** nginx answers a
  request for a proxied prefix location *minus* its trailing slash with a 301
  that appends one. So `/share` was redirected to `/share/`, which then matched
  the API prefix and 404ed on `apas-server` — every share invite was dead
  (`/share?code=...`), and the whole `/admin` page with it, while the
  `/share/*` API endpoints looked perfectly healthy the entire time. The fix is
  an exact-match location, which outranks the prefix and is exempt from the
  redirect:

  ```nginx
  location = /share { proxy_pass http://127.0.0.1:3000; ... }
  ```

  `/auth`, `/admin`, and `/share` all have one. **Add another whenever you add
  an API prefix that shares a name with a Next.js page**, and check
  `curl -sL -o /dev/null -w '%{http_code} %{num_redirects}' <url>` — a page
  answering `200 1` instead of `200 0` is this bug.

  **The mirror-image mistake is easier to make: a new API prefix nginx does not
  proxy at all.** Everything unmatched falls through to Next.js, so the route
  answers `200` with the app's HTML instead of JSON, and only the caller
  notices. `/cluster/` shipped this way and broke the entire cluster surface
  until it was added here. **Whenever you add a route prefix to
  `routes/create_router`, add the matching `location` in this file in the same
  change**, and smoke-test it with `curl -s <url> | head -c 40` — an HTML
  doctype where JSON belongs is this bug.
- **apas-server**: `127.0.0.1:8080` — still also bound publicly on `:8080` for
  the **CLI/daemon** (`wss://apas.mpaxos.com/ws/cli`); do not firewall 8080.
- **Next.js web (apas-web)**: `127.0.0.1:3000` (moved off 80 via a systemd
  drop-in `apas-web.service.d/port.conf` → `Environment=PORT=3000`).
- The web's default WS/API URLs are now **port-less** (`wss://apas.mpaxos.com`,
  `https://apas.mpaxos.com`); `NEXT_PUBLIC_WS_URL`/`NEXT_PUBLIC_API_URL` no
  longer need to be set at build time.

#### Directory Structure on Server
```
/opt/apas/
├── apas-server         # Server binary
├── data/
│   ├── apas.db         # SQLite database
│   └── sessions/       # Message storage
├── web/                # Next.js web frontend
└── protocol/           # Matching @apas/protocol source, schemas and runtime deps
```

#### Systemd Services
```bash
# Check status
systemctl status apas-server
systemctl status apas-web

# Restart services
systemctl restart apas-server
systemctl restart apas-web

# nginx edge proxy (port 80). After editing /etc/nginx/conf.d/apas.conf:
nginx -t && systemctl reload nginx

# View logs
journalctl -u apas-server -f
journalctl -u apas-web -f
journalctl -u nginx -f
```

#### Deploying Updates

The web's `file:../protocol` dependency makes **web and protocol one release
pair**. Install the protocol's runtime dependencies separately: npm's linked
package installation does not install them. Audit that runtime graph as well as
both complete web graphs, including web development dependencies. A clean web
audit alone missed an older `fast-uri` in the protocol's own lockfile.

Keep `packages/web/.npmrc` (`install-links=false`) and the parent-directory
Turbopack root in `next.config.ts`. Production npm 9 defaults to packing file
dependencies while the reviewed lockfile describes a link; without the explicit
setting `npm ci` rejects that lockfile. Without the explicit bundler root, a
deployment outside the repository cannot resolve the sibling package.

Build in a new release directory rather than overwriting the running app's
source, `node_modules` or `.next`. Inspect both rsync dry-runs. Preserve the
production environment files rather than copying local development overrides.

```bash
# Run these dependency checks in separate clean sibling-package trees.
(cd packages/web && npm ci --omit=dev --prefix ../protocol && npm audit --omit=dev --audit-level=low --prefix ../protocol && npm ci && npm run audit:npm)
(cd packages/web && pnpm install --frozen-lockfile && pnpm run audit:pnpm)

# A config block is not proof that the administrator can sign in.
ssh root@apas.mpaxos.com "test \"\$(sqlite3 /opt/apas/data/apas.db 'select count(*) from system_admin_credential;')\" != 0"

cargo build --release -p apas-server
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
stage="/opt/apas/releases/${stamp}"
backup="/opt/apas/backups/deploy-${stamp}"
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"

# Private backups: binary, configuration, a consistent SQLite snapshot, and
# the matching web/protocol source and build. Keep production config mode 0600.
ssh root@apas.mpaxos.com "install -d -m 700 '$stage/web' '$stage/protocol' '$backup' && cp -p /opt/apas/apas-server '$backup/apas-server' && cp -p /opt/apas/apas-server.toml '$backup/apas-server.toml' && cp -p /etc/nginx/conf.d/apas.conf '$backup/nginx-apas.conf' && sqlite3 /opt/apas/data/apas.db \".backup '$backup/apas.db'\""
ssh root@apas.mpaxos.com "if test -d /opt/apas/protocol; then tar -C /opt/apas --exclude=web/node_modules --exclude=protocol/node_modules --exclude=web/.next/cache -czf '$backup/web.tgz' web protocol; else tar -C /opt/apas --exclude=web/node_modules --exclude=web/.next/cache -czf '$backup/web.tgz' web; fi"
ssh root@apas.mpaxos.com "find /opt/apas/web -maxdepth 1 -type f -name '.env*' -exec cp -p '{}' '$stage/web/' ';'"

rsync -avn --delete --exclude node_modules --exclude .next --exclude .apas-version --exclude '.env*' packages/web/ "root@apas.mpaxos.com:$stage/web/"
rsync -avn --delete --exclude node_modules --exclude dist packages/protocol/ "root@apas.mpaxos.com:$stage/protocol/"
rsync -av --delete --exclude node_modules --exclude .next --exclude .apas-version --exclude '.env*' packages/web/ "root@apas.mpaxos.com:$stage/web/"
rsync -av --delete --exclude node_modules --exclude dist packages/protocol/ "root@apas.mpaxos.com:$stage/protocol/"
scp target/release/apas-server "root@apas.mpaxos.com:$stage/apas-server"

# Build before touching either live service. prebuild includes the hook lint
# gate; do not bypass a failure. The remote tree has no git version history.
ssh root@apas.mpaxos.com "cd '$stage/protocol' && npm ci --omit=dev && npm audit --omit=dev --audit-level=low && cd '$stage/web' && npm ci && npm run audit:npm && NEXT_PUBLIC_WEB_UI_VERSION='$web_version' npm run build"

# Server first. Refresh the database backup while stopped, then replace the
# executable atomically. On failure, restore the backed-up binary and restart.
ssh root@apas.mpaxos.com "systemctl stop apas-server && sqlite3 /opt/apas/data/apas.db \".backup '$backup/apas.db'\" && install -m 755 '$stage/apas-server' /opt/apas/apas-server.next && mv /opt/apas/apas-server.next /opt/apas/apas-server && systemctl start apas-server"
curl --fail --retry 15 --retry-all-errors --retry-delay 1 --max-time 3 https://apas.mpaxos.com/health

# Retain the previous hashed assets for browsers still running the old bundle.
# Preserve the complete old pair for fast rollback, then activate the staged pair.
ssh root@apas.mpaxos.com "rsync -a --ignore-existing /opt/apas/web/.next/static/ '$stage/web/.next/static/' && systemctl stop apas-web && mv /opt/apas/web '$backup/web' && { test ! -d /opt/apas/protocol || mv /opt/apas/protocol '$backup/protocol'; } && mv '$stage/protocol' /opt/apas/protocol && mv '$stage/web' /opt/apas/web && systemctl start apas-web"

for path in / /login /machines /share /admin /code/jobs /code/jobs/new /code/workers /code/workers/new /health; do curl -fsSL "https://apas.mpaxos.com${path}" >/dev/null; done
# Also verify referenced /_next/static assets, authenticated /projects/workers JSON,
# WebSocket/terminal attachment from the UI, and effective recurring-worker policy
# remaining disabled until qualification. Never print tokens or terminal contents.
ssh root@apas.mpaxos.com "systemctl is-active apas-web apas-server nginx && journalctl -u apas-web -u apas-server --since '5 minutes ago' -p err --no-pager -q"
```

For web rollback, stop `apas-web`, move the failed web/protocol pair aside,
restore the complete pair retained under the selected `deploy-<timestamp>`
backup, then restart and repeat every smoke check. If only `web.tgz` remains,
extract its pair under `/opt/apas`, reinstall the matching protocol runtime and
web dependencies, and perform the versioned build before restarting. An older
release without a protocol directory does not need one. A rollback may restore
older vulnerable dependencies; follow it with a corrected patched deployment.

For server rollback, restore the backed-up binary atomically and restart.
Do not automatically restore an old SQLite snapshot after traffic has resumed:
that can discard acknowledged work. Preserve the current database and evaluate
any schema/data recovery separately. Rotate an administrator credential only
when its bootstrap state requires it, not as a side effect of routine deployment.

## Versioning

Versions are computed at build time as `YY.MM.N`, where `YY.MM` is the current
year and month and `N` is `git rev-list --count --since="<YYYY-MM-01 00:00:00>"
HEAD`.

- The web version is resolved in `packages/web/next.config.ts`.
- CLI and server versions are resolved in their crates' `build.rs`.
- `packages/web/.apas-version` is dead. Nothing reads it, the deploy rsync
  excludes it, and it should not be created or maintained.

A build in a directory without `.git` cannot count commits, which is exactly
the case for `/opt/apas/web` on the server. Pass `NEXT_PUBLIC_WEB_UI_VERSION`
explicitly there, as the deploy commands above do.

## Key Concepts

1. **Pane Runtime**: The CLI restores interactive panes from `.apas` and can
   isolate them in git worktrees. Legacy deadloop entries remain nonexecuting.
2. **Hybrid Mode** (legacy): Single pane with local terminal + streaming.
3. **Project-based Sessions**: Sessions are identified by project directory
   and `.apas` project metadata.
4. **Stream-JSON**: Uses Claude CLI's `--output-format stream-json` for
   structured Claude output; other providers are bridged through their own
   runtime paths.
5. **Real-time Updates**: WebSocket connections broadcast live messages,
   machine config, and pane diffs.
6. **Pane kinds**: `PaneConfig.kind` selects the interactive hosting mechanism,
   independently of `provider`. The old deadloop `mode` survives only for
   history; recurring contained work belongs to autonomous workers, and an
   OMP terminal tab can repeat one prompt on autopilot. See "Terminal panes".

## Tab-type policy (`disallowed_tab_types`)

A project's owner or admin can restrict which tab types users may create. A
"tab type" is a **pane kind plus a provider** — `agent:claude`,
`terminal:codex` (`shared::tab_type_key`). Neither half alone identifies one: a
claude agent tab and a claude terminal tab are different capabilities, since the
terminal runs the real TUI with permission prompts bypassed.

Stored as a **deny** list, presented in the UI as an allow list. An allow list
on the wire would make an absent field mean "nothing permitted", so every
project predating the feature would refuse to open any tab. Empty deny list =
everything allowed = existing projects unaffected. It also means a provider
added later is permitted until an owner says otherwise, rather than vanishing
from their menu.

The catalog is deliberately **not** every `Provider`. It lists exactly
`terminal:claude`, `terminal:codex`, `terminal:opencode` and `terminal:omp`.
DeepSeek is the Claude binary against a different backend, so the add-tab menu
offers it as a Claude model. The retired values (MiniMax, GLM, and Pi since
2026-11-13) still decode and are never offered. `shared::all_tab_types` and
`packages/web/src/lib/tabTypes.ts` must agree; a test in `shared` reads the TS
file and asserts they do.

Enforced in the CLI (`tab_type_allowed_for`), which re-reads `.apas` on every
`AddPane` — the web only hides menu entries, and the same message can arrive
from a stale browser tab whose menu predates the restriction. It fails **open**
on an unreadable `.apas`: the worst case
is a tab an owner meant to block, whereas failing closed would lock everyone out
of the project entirely.

## Virtual clusters and system administration

These are two different jobs and two different surfaces. They used to be one
page behind one `cluster_role = "admin"` flag, which meant running *your own*
machines required deployment-wide authority over everyone else's projects.

**A virtual cluster is derived, not stored.** Every account operates exactly
one: the machines whose client registered under it, plus the projects hosted in
it. The cluster has no row of its own; what is stored is each project's
placement. A project is hosted in an account's cluster when
`project_cluster_placements` holds a row for that project and that account, and
for no other reason (`db::project_in_user_cluster`). Ownership, membership and
session history are not read when the question is asked. A placement is written
when an account's CLI starts or registers the project
(`authorize_project_registration`, which admits only the project's owner, a
member, or an account that already hosts it) and when a cluster member
provisions a project on a shared machine.

The rule used to be read from live data: the account **owns it**, or **at
least one of its sessions was created under it**. Sessions were the durable
evidence of where a project runs, since machines live only in memory and
projects carry no machine column. A one-time migration froze that relation
into placement rows (`legacy_owner`, `legacy_session`), so every project hosted
under the old rule still is, and session history stopped being an authorization
source. So a project owned by `soumojit.dalui` that ran on your daemon, or that
he provisioned on a machine you shared, is in *your* cluster, and you
administer it without being a member of it.

**Belonging to a project is deliberately not hosting it.** Content access is
owner ∨ member ∨ host; administration (lifecycle, stop-runtime, membership,
ownership, policy) is host only. Otherwise anyone you shared a project with
could suspend it. Running it on your own machine *does* make you a host — that
is the point, not a loophole.

The cluster surface is `/machines` (kept at that route so links still work),
available to every active account with no role check: `routes/cluster.rs`
scopes every request through the hosting predicate. `routes/admin.rs` backs the
deployment-wide surface and both call the same DB operations, so the two cannot
drift.

**The page is six views, redesigned on 2026-11-15.** `/machines` ("My
Cluster") used to be one column about 10,000 px tall. It is now split by
`?view=`: **Machines** (the default), **Projects**, **Sharing**, **Usage**,
**Policies** and **Activity**; a shared cluster shows Machines and Projects.
Every function it had is still there.

- **Addresses.** `?view=projects&selected=<id>` opens that project.
  `?machine=&project=` always opens Machines, unfolds that machine's stopped
  projects and focuses the row; nothing starts. The page writes the view with
  `history.replaceState`. The helpers (`parseClusterView`, `clusterViewUrl`,
  `sortMachines`: connected first, then by name, used by every list of
  machines on the page) are in `lib/clusterView.ts`; the views are in
  `components/cluster/`.
- **Machines.** One card per machine with a Connected or Offline chip, at
  most one amber chip ("Update available", else "Older APAS"), its version,
  the "Providers:" line and "r of t projects running". Rows are ordered
  errors, running, stopped, and more than three stopped projects fold into
  "Show N stopped projects". Start and Stop still never send
  `all_instances`. Reboot stays per machine; its dialog title names the
  machine, and "Reboot to update" and `rebootEffectNote` keep their wording.
- **The DeepSeek API key is never shown, prefilled or kept in browser
  state.** It used to sit in plain text in an input on every machine card.
  One "DeepSeek keys" list says whether each machine has a key and offers Set
  key, Replace key and Clear key (which asks first). **The server still sends
  the key to the owner's browser** in the `machines` message; the web no
  longer reads it. Removing it from that message is a separate server change.
- **Nothing is claimed that was not sent.** `startMachineProjectCli`,
  `stopMachineProjectCli` and `setMachineDeepseekConfig` return whether they
  sent anything. The page shows "Starting…", "Stopping…" or "Key sent" only
  for a request that went out, and otherwise "Not connected to the server.
  Nothing was sent." It resends nothing.
- **Loading and failure are said per view.** Cluster data is fetched after
  `/cluster/contexts` answers; a view says "Loading…" until its own data
  arrives and "Could not load …" on failure, never an empty state.
- **Projects** is a filterable list with a panel for the selected project
  (usage, runtime, owner, members, project policy, and the worker policy
  closed). Below 1024 px the panel replaces the list. Projects that share a
  name are told apart by machine in button names ("Manage mako on zoo-006").
- **Usage** sorts by tokens, shows 15 rows and folds projects with no usage
  into one line; a reported zero cost reads "$0.00" and an unreported one
  "Unavailable". **Activity** says each event in words, raw code in the
  tooltip. **Policies** keeps the cluster worker policy in a closed section
  that sends no request until opened; unavailable-reason codes are shown as
  sentences (`components/cluster/jobReasonLabels.ts`).
- Every confirmation uses one in-page dialog
  (`components/cluster/ConfirmDialog.tsx`); no `window.confirm` remains.
  Inline editors take focus when opened and return it to their button.
- Shared with other pages, with defaults that leave `/admin` unchanged:
  `PolicyEditor appearance="overview" | "panel"`, `AutonomousJobPolicyEditor
  embedded` / `reasonLabels`, and the Overview's period switch
  (`components/overview/UsagePeriodControl.tsx`). `PolicyEditor` keeps unsaved
  choices until the policy's version or profile list changes.
- **Rolled out on 2026-11-16** with web-only release `20261116T012002Z`
  (version 26.11.59, built at 9ad158b from the committed tree), live since
  01:23 UTC; backups under `/opt/apas/backups/deploy-20261116T012002Z`. The
  server was not restarted. Both dependency graphs audited clean on the host.
  Twelve addresses (the ten public routes and two `?view=` ones) answered
  `200` with no redirect, and 18 referenced static assets answered `200`.
  Gates: web 1,439 tests, typecheck and lint. Checked before the release in
  the read-only viewer on the owner's real cluster at 1440 and 390 px and in
  dark mode. Not exercised on the live site: any write from the new page
  (Start, Stop, Reboot, a key change, sharing and policy edits), since the
  viewer forwards none, and a shared cluster's member view in a browser.

**Cluster sharing is owner-administered and immediate.** The owner adds an
existing active APAS account directly; the ordinary web flow has no invitation
or acceptance step. Each active membership carries either an explicit machine
allowlist or the legacy `NULL = all current and future machines` value. Machine
projection, runtime access, project creation, and project-provisioning
finalization all enforce that allowlist. The membership may also name the launch
profile applied to projects that member creates; provisioning snapshots it and
creates a one-profile project policy override. Members create projects from the
web with a public GitHub clone URL and one of their allowed machines. Only the
machine owner can instead register an existing local folder; membership does
not grant that capability. Invitation tables and endpoints remain only for
compatibility with older clients and outstanding links.

### Project shares and cluster shares are independent

A **project share** grants conversations and other durable content for that
project. When the selected session is hosted by the canonical project owner,
the same explicit project role also grants live, project-scoped runtime use.
It never grants machine inventory, project provisioning, provider
configuration, cluster administration, or access to another project.

A **cluster share** grants compute only on the membership's machine allowlist
and applies its configured default launch profile to projects that member
creates there. It never grants conversation or policy visibility for a project
the member neither owns nor explicitly belongs to.

The two grants compose for third-party hosting. If a session's host is neither
the caller nor the canonical project owner, the caller needs both ordinary
project access and active membership in the hosting cluster with permission for
that session's exact machine. Revoking the cluster or machine grant stops later
attachments and runtime mutations without deleting the project role or its
persisted history.

| Caller authority | Project content | Owner-hosted runtime | Third-party-hosted runtime | Shared machines / provisioning |
| --- | --- | --- | --- | --- |
| Project owner or explicit project user | Yes | Yes, for that project only | Only with host-cluster membership and exact-machine permission | No |
| Cluster member only | No | No | No; project access is also required | Allowed machines only |
| Project user plus matching host-cluster grant | Yes | Yes, from the project role | Yes, on the permitted machine | Allowed machines only |

Resolve this per selected session and on every attach or runtime mutation; a
project can have owner-hosted and third-party-hosted placements with different
answers. `Database::check_session_runtime_access` is the common predicate.
Content-only reads continue to use ordinary project access.

Web navigation is also authorization-transactional. `attach_session` keeps the
requested session pending; only a matching `session_attached` may change the
active or persisted workspace, restore its cache, or request catch-up. Expected
denials use `session_attachment_rejected` with the requested session and a safe
project-access, host-machine-access, unavailable-project, or missing-session
reason. Background and out-of-order results never become foreground navigation.

**System administration is a credential, not an account.** One per deployment,
stored in `system_admin_credential` outside the `users` table, seeded from
`[system_admin]` in `apas-server.toml` only when no row exists — so editing the
config later cannot revert a rotation. Its token carries `sub =
"system-admin"`, `token_kind = "system_admin"`, and the credential version;
rotating the password bumps that version and invalidates every outstanding
token. An account token is rejected by `/admin/*` and this token is rejected
everywhere else. No UI can grant it, and `bootstrap_admin_email` is gone.

**An empty `bootstrap_password` is silent and total.** Seeding is skipped when
the value is blank, and the only report is one `WARN` at startup. With no
credential nobody can enter `/admin`; `/admin/users/invitations` is the sole
code path that creates a registerable invitation, so nobody can be invited
either. This deployment shipped that way and could not create a single account
between 2026-08-08 and 2026-09-09. **Checking that the `[system_admin]` block
exists is not enough** — the block was present the whole time with an empty
password. Verify the credential row instead, which is the thing that actually
decides:

```bash
ssh root@apas.mpaxos.com "sqlite3 /opt/apas/data/apas.db \
  'select count(*) from system_admin_credential;'"   # 0 means /admin is unenterable
```

**Two ways to get an account, and only two.** An **invitation** admits exactly
the address it names, from any domain, and is issued by the system
administrator. **Self-signup** admits an address whose domain the deployment
lists in `[auth] self_signup_email_domains`:

```toml
[auth]
self_signup_email_domains = ["cs.stonybrook.edu", "stonybrook.edu"]
```

The allowlist **is** the switch. There is deliberately no separate
`open_registration` boolean, because one could be set while the list is empty
and admit the entire internet; empty or absent means invitation-only, which is
both the default and what every config file written before this setting already
says. Matching is **exact** — a parent domain does not admit its subdomains, so
`mail.stonybrook.edu` must be listed in its own right. Widening is a config
edit plus a restart, never a rebuild, and the server names the live policy at
boot (`Registration is open to these email domains…`, or
`Registration is invitation-only…`), so the state is never silent again.

**The `/admin` login must stay inline on the page.** nginx proxies the whole
`/admin/` prefix to `apas-server`, so a Next.js route at `/admin/login` would
never be served. The page renders its own form when it holds no token, and
keeps that token in `sessionStorage` under its own key — never in the zustand
store, never in `localStorage`, so it dies with the tab. Nothing in the ordinary
interface links to it.

The admin Users table shows **Last login** immediately after **Created**.
`users.last_login_at` records successful web and mobile password sign-ins,
signup's automatic sign-in, and CLI device-code token issuance as a UTC
RFC3339 timestamp; the browser displays it in local time. Failed or suspended
sign-ins, bearer-token checks, reconnects, and mobile token refreshes do not
advance it. The separate system-administrator credential is not a user login.
Existing accounts retain `NULL` until their next successful sign-in and show
`—`; historical login times are not inferred from account creation or activity.

**`cluster_role` no longer means anything.** Every account migrates to `user`,
and the column plus its wire field survive only so older web and mobile builds
keep parsing identity responses. Two deployment-wide reads went with it: web
machine listings no longer have an all-machines branch, and no account can
revoke another account's mobile device.

**Policy resolves over three levels** — deployment default (system
administrator) → cluster default (`cluster_default_policies`, one row per
account) → project override. The launch-profile allowlist **narrows
monotonically**: each level may only restrict what the level above allows, and
a widening write is rejected rather than silently clamped. It intersects over
*every* hosting cluster, not just the owner's, so your cluster default governs
the foreign-owned projects on your machines; intersection is order-independent,
so a multi-hosted project still has one answer.

`team_available` is **vestigial**. It is still on the wire and in stored policy
so web and mobile builds that predate the team-mode removal keep parsing what
they are sent — the same reason `cluster_role` survives — and it decides
nothing. Do not reintroduce a read of it; a test asserts a policy carrying it
behaves exactly like one without it. Four leftovers were still reading it
until 2026-11-13: the policy editor on `/machines` and `/admin` showed a
"Team launch available" switch, the web store refused to resume, reboot or
switch a `managed` pane while it was off, the server listed a stored
`managed` pane as outside policy while it was off, which is its default, and
the CLI's AddPane check refused a new pane marked `managed` while it was off.
All four are gone, so no UI offers it. The CLI's part reaches a machine only
when its daemon is rebooted to update. A policy save sends back the value the
editor was given, so saving the launch profiles cannot change it.

**Audit carries an actor kind and a cluster.** `admin_audit_events` was rebuilt
once (create-copy-drop-rename, guarded by `schema_migrations`) because
`actor_user_id` referenced `users(id)` with `PRAGMA foreign_keys=ON`, which made
a non-account actor unrecordable. `cluster_invitations.created_by` was rebuilt
for the same reason. `project_members.invited_by` keeps its key, so a
system-administrator membership change is attributed there to the project owner
while the audit row records who actually did it. An operator's audit view also
applies the live hosting predicate, so a project hosted in several clusters is
visible to each of them and pre-attribution rows still land where they belong.

## Projects run inside the one instance

A host runs one `apas` process for a user, and the projects run *in it* as
supervised tasks. The daemon used to spawn each project as `apas --headless`
into its own tmux session and then could not see it, which is why running state
was inferred from `/proc` and why a restarted daemon came back owning nothing.
A host is now one `apas` plus one **pane host per terminal pane**.

**Pane hosts are deliberately not merged.** They own the PTYs so a provider
survives the CLI being replaced — which is also what keeps this arrangement
safe, since the blast radius is the supervision layer rather than running
agents.

**Stopping a project sets a flag it observes; it is never an aborted future.**
Aborting would strand roughly thirty threads and every pane child. The flag
ends the project through the same teardown an ordinary stop takes, and the wait
is bounded at 30s so one project that will not stop cannot hold the instance
the others are running in.

**Failure containment rests on unwind.** `panic = "abort"` is not set, so a
panicking project unwinds its own task, is reported as stopped, and leaves the
others alone. A test fails if anyone ever sets it, because that would turn any
project panic into a host-wide outage with nothing else looking different. The
three `process::exit` calls that used to mean "stop this project" are gone;
`run_inner` returns a `ProjectOutcome` and the caller decides.

**Blocking work must stay off the runtime.** Each project's blocking readers
already run on their own threads. New code that blocks in async would stall
every other project, and a two-project test will not show it.

**Two things the process boundary gave for free had to be replaced.** Each
project carries a `project` span so its records are identifiable — the
per-project tmux session and stderr log are gone, and the default `Full` log
format prints the span fields. And `exec` now takes the projects with it, so an
upgrade writes what was running to a manifest in the *runtime* directory
(volatile, so a machine reboot starts nothing nobody asked for; cleared on read
so a crash cannot retry forever) and starts them again afterwards. Pane hosts
survive the `exec`, so a prompt resume lands inside their adoption grace.

**A PATH trap comes with the merge.** The old model passed
`env PATH=<login shell PATH>` on every spawned project's command line, because
a daemon started from a minimal environment cannot find nvm/cargo-installed
providers. In-process, projects inherit the daemon's PATH, so the daemon
applies the login shell PATH to itself before any project starts.

**The rollout is not additive.** An older instance's projects are separate
processes this one cannot supervise, so a starting instance stops the
process-per-project leftovers before starting anything — otherwise one `.apas`
and one set of worktrees get two owners. It finds those by their `-d <path>`;
it cannot find one a person started by running `apas` in a directory before
that became register-and-exit, because those carry no arguments and nothing
distinguishes them from an `apas --attach` in use. Those stay a manual step.

`apas --headless -d <path>` survives as a way to run one project alone for
debugging. Nothing spawns it, and a project with an external headless run is
never given a second owner.

## One instance per user per host

Running `apas` in a project directory **registers that project and exits**. It
does not open a terminal UI and does not start the project — projects are
started from the web. A host runs one APAS instance per user, and by the time
a launch gets there it already exists, because `ensure_daemon_running` started
it.

This is the existing `apas daemon` rule applied to the command people actually
type. `detect_running_daemon` already reads a pid state file, verifies the
process really is `apas`, and deletes a stale record; `apas daemon` already
prints and returns when one is running. The gap was that plain `apas` went
straight into `dual_pane` without asking, so typing it in a directory the
daemon was already running produced two owners of one project, one `.apas`,
and one set of worktrees — `is_headless_running_for` guards only the daemon's
own spawns.

Registration is all a launch needs to do: the daemon reads the shared registry
(`list_registered_projects`) on every heartbeat and reports what it finds, so
the project appears on the Machines page with no IPC and no start request.

**`apas register [path]` is the explicit form**, defaulting to the current
directory. It uses the strict `register_local_project` path behind the web's
**Existing folder** flow rather than a launch's lenient one: the folder must
exist, valid `.apas` bytes and identity are kept, malformed metadata is
rejected instead of regenerated, an id/path conflict fails, and a registry
write failure is an error. It then ensures the resident instance is running,
since that is what reports the project to the server. It never starts the
project. A checkout whose git remote matches one of your projects can join that
project instead of becoming a project of its own (`--project`, `--new`,
`--yes`); see "A project's tabs can run on several machines".

**Creating a project from a local directory is still a thing a launch does.**
`apas` in a directory that is not yet a project creates and registers it.
The web's **Existing folder** flow now offers register-only creation to machine
owners too. Neither path implicitly starts the project.

**Headless workers are exempt.** They are `apas` processes, but the daemon's
children rather than instances a user launched; applying the rule to them
would stop the daemon running more than one project.

`apas --attach` opens the terminal UI for a project already running here. It
is a local view for when the web is unreachable, it renders little beyond pane
names, and it is expected to be removed if nothing uses it.

## macOS hosts

A Mac can run the daemon and host tabs. Until 2026-11-15 it could not do so
reliably: the running-daemon check read `/proc`, which macOS lacks, so every
`apas` command started another daemon. The owner's MacBook Air had three,
sharing one machine id and knocking each other off the server, and an OMP tab
there never showed an agent (change
`openspec/changes/support-macos-hosts-and-install-omp/`).

- **Process checks** (`process_info.rs`) use `kill(pid, 0)` and, off Linux,
  `/bin/ps` or `/usr/bin/ps` with a 5 s timeout. Linux still reads `/proc`.
- **`apas daemon` holds `<runtime dir>/daemon.lock` for its life.** A second
  one prints "Daemon already running (pid N, version V)." and exits. A
  requested restart keeps the lock's pid. Daemons from an older APAS take no
  lock: a new daemon names them ("Other APAS daemons are running on this
  host …") and never stops them.
- **Runtime dir on macOS** without `XDG_RUNTIME_DIR` is
  `~/Library/Application Support/apas/run` (daemon state, lock, resume
  manifest, pane hosts and their logs). `$TMPDIR` differs between a GUI
  session, ssh and launchd, and the system cleans it. The first update from an
  older APAS also takes the resume manifest and lists the pane hosts the old
  one left under `$TMPDIR/apas-<uid>/apas`. Config is under
  `~/Library/Application Support/com.apas.apas`.
- The daemon merges the login shell's PATH (`-lic`) with its own and logs
  which tmux it found. The pane host checks its controller with
  `getpeereid`. A socket path over 104 bytes fails with that reason.
- **OMP tabs work on macOS as on Linux** since the start-time check was
  dropped (see "OMP activity is not conversation text"): live status,
  questions answered from the web, and autopilot. A Mac tab whose agent was
  started before that update keeps transcript status and terminal-only
  questions, and its Autopilot control reads "Reboot this tab's agent to use
  autopilot. Rebooting ends its turn in progress." Reboot the agent once.
  Claude and Codex run from their configured paths; provider copies stay in
  `/var/tmp`.
- The machine id is derived from user@hostname, so a Mac whose hostname
  changes (a Bonjour `-2`, DHCP) appears as a new machine. "Reboot to update"
  needs git and a Rust toolchain on the Mac.

**First update of a Mac that ran APAS 26.11.54 or older, by hand:** stop every
`apas … daemon` (`pgrep -fl 'apas .*daemon'`, then `kill` each pid, keeping
none); update the CLI; start one daemon (`apas daemon`); check the machine
shows once on the Machines page without reconnecting; start the project again
from the web. Do not use Reboot to update there: the new image can read the
old `daemon.json`, find another old daemon alive, and exit.

None of the macOS-only code was compiled or run before it first shipped, and
three bugs reached the owner's Mac that way (the installer's progress
callback, the pane host's non-blocking connections, and this section's own
first limits). Since 2026-11-15 the CLI type-checks for macOS on the Linux
build host: `scripts/macos-check/check.sh` runs `cargo check -p apas
--all-targets --target aarch64-apple-darwin` with four stub C headers so
ring's build script finishes. Nothing is linked or run, so it finds compile
errors only. **Run it before pushing a CLI change that touches `cfg`-gated or
Unix-specific code.** Nothing has run on a Mac except by the owner. The OMP
extension's tests simulate a host without `/proc`, and a real OMP 18.20.0 and
18.20.2 ran it on Linux, also with its own `/proc` entry unreadable. First
checks on the Mac, after a daemon reboot and one agent reboot:
`omp-activity.json` appears in the tab's runtime directory within seconds; a
question is answered from the web; autopilot starts and stops. If the
extension stays silent, the daemon's log says so once per agent after 60
seconds.

## Machines report their providers; OMP is installed when missing

Each daemon reports, in `MachineInfo.providers`, whether it finds Claude,
Codex, OpenCode and OMP the way a launch would, their versions, and whether
APAS can install them (only OMP, and only where a launch would). It sends a
quick list at registration and refreshes it every five minutes, after an
install and at daemon start (`provider_availability.rs`). A missing list means
an older daemon and changes nothing in the UI.

When a tab is created, the provider choice for the machine that will get it
reads "Installs OMP first" where the machine will install OMP, and is disabled
with "<Provider> is not installed on <host>." where the provider is missing
and cannot be installed (`providerOfferOn` in `lib/machineChoice.ts`). The
machine itself is never disabled for this. The Machines page and the phone's
Machines card show "Providers: Claude, Codex, OMP", versions in the tooltip.

**The OMP install** (`omp_install.rs`) pins one checked version, **18.20.2**,
with the SHA-256 and size of the darwin-arm64, darwin-x64, linux-x64 and
linux-arm64 assets. It downloads the GitHub release asset, requires the
release's `SHA256SUMS.txt` to list the pinned hash and the file to match, on
macOS requires `codesign --verify --strict` and team `VTLHGQC72S`, checks
`--version`, and publishes `~/.local/bin/omp` with a hard link, so it never
replaces an existing file. It never runs OMP's `install.sh`, never installs to
a custom `omp_path`, and edits no shell rc file. An Intel APAS under Rosetta
gets the arm64 build; musl hosts and Windows are refused with the reason.
Progress shows in the tab's terminal. One install runs per machine (a lock
beside the host-local store, 20 min wait); the 15-minute download limit
starts once the lock is taken; partial `.omp.download.*` files older than
that are removed by the next install. A failure ends the tab with the cause,
`curl -fsSL https://omp.sh/install | sh` and `apas config set omp_path`.
After the install the host-local copy and its `omp update` take over as
before. Any user with runtime access to a project on a machine can trigger
the install there. **To move the pin:** take the new release's
`SHA256SUMS.txt` and sizes, update `OMP_VERSION` and `PINNED_ASSETS`, and run
the real-OMP check on a minor-version change.

**A failed terminal launch is reported** (`launch_status.rs`). Creating,
restoring, rebooting or switching a tab whose provider cannot start sends an
exited terminal state with the reason, a terminal exit and a conversation
line, at once and again after every reconnect. Until then such a tab, like the
Mac's OMP 4, reported nothing and stayed blank.

**Rolled out on 2026-11-15.** Server and web release `20261115T230257Z`
(version 26.11.55, built at aa3f42b) since 23:05 UTC; backups under
`/opt/apas/backups/deploy-20261115T230257Z`. Dependencies had not changed and
audited clean on the host. Ten public routes and 17 referenced static assets
answered `200` with no redirect, `/projects/by-remote` answered `401` from
`apas-server`, and the server logged no error and no frame it could not
parse. `master` was pushed for the CLI; each machine gains it at its daemon
reboot (a Mac with several old daemons by hand, as above). Gates at aa3f42b:
shared 107 + 4 + 1, server 464, CLI 670 (2 ignored), static musl build,
protocol 80 and the extension's 137, web 1,353 with lint, typecheck and build
(one worker test timed out under load and passed alone), native 136,
`openspec validate --all --strict`. Not yet seen: any of it on a Mac, a real
OMP download and install, the providers line with a real daemon's report.

**The first real install failed** on the owner's Mac with "the installer
stopped unexpectedly". The download ran inside a runtime and called the tab's
progress callback there, which sends with `blocking_send`; that panics inside
a runtime, so the first progress line ended the install. The tests' callbacks
only collected lines. The download now runs on a thread of its own and its
lines reach the callback on the caller's thread, and a test drives `install`
with a `blocking_send` callback (it fails on the old code with tokio's
"Cannot block the current thread from within a runtime").

**The second failure** was the pane host: its listener is non-blocking, and
on macOS an accepted connection inherits that, so the host dropped its
controller at once ("pane-host controller disconnected: failed to fill whole
buffer"). Fixed at 9fc277b by making each accepted connection blocking.

**The shared installed CLI was updated by accident on 2026-11-16 at 01:59
UTC.** While working on the extension change, an agent's shell command
contained `apas` inside backquotes and ran it in the repository on zoo-005.
That is a normal launch: it pulled pushed `master` (834a71e) into
`~/.local/share/apas/source`, built it and replaced `~/.local/bin/apas`
(26.11.60) on the NFS home zoo-005 and zoo-006 share, and found the project
already registered. No daemon was restarted and `.apas` did not change; both
daemons kept running their old image (zoo-005's holds it open as an `.nfs`
file). The effect is the one any machine's update has on the others: new pane
hosts on both machines start from the new binary under daemons still on
26.11.49. Never put a command name in backquotes inside a double-quoted shell
string.

## The daemon is replaced only when asked

Replacing the daemon used to be nearly free: it owned no long-lived children, so
a **15-minute self-upgrade tick** was a good trade — unattended hosts stayed
current and nothing running noticed. zoo-002 sitting nine versions behind was
the problem it solved.

Projects run **inside** the daemon now. Replacing it is the same act as stopping
every project on the host and starting them again, so nothing does it on a
schedule any more. **The tick is gone**, along with the `stat`-based gate it
needed (`apas_binary_fingerprint`, `newer_installed_version`). Installing a
binary to the shared path no longer propagates on its own.

**A launch does not replace it either.** `apas` in a project directory, and
`apas daemon`, both used to stop an older daemon and start a fresh one. That was
invisible when the daemon owned nothing; now it ends every project on the host —
and by `stop_daemon_process`'s SIGTERM/4s/SIGKILL, so nothing is saved, no
resume manifest is written, and nothing comes back. Whoever types `apas` in a
directory is a bystander to that work. Both paths now report that the running
instance is older and point at the Machines page; `stop_daemon_process` is gone
with its last caller. `plan_launch_daemon` holds the decision so it is testable
away from the spawning, the way `plan_daemon_restart` does.

**The requested restart is the whole upgrade path**, from the Machines list on
desktop and mobile (`WebToServer::RebootDaemon { machine_id }` →
`ServerToDaemon::RebootDaemon`). It is addressed by **machine**, not through a
project: a daemon is per-machine, and a host running nothing still has one worth
restarting. The server authorizes the machine against the requester's own daemon
registrations — the same check `StartMachineProjectCli` uses — and reports an
offline daemon rather than dropping the request.

The confirmation for one machine says what that costs: "Every project running
on <host> restarts with it. Terminal agents normally keep running during the
restart, because their hosts are separate processes." (`rebootEffectNote` in
`lib/daemonVersion.ts`, used on the Machines page and on the phone). It used
to say that projects, panes and agents keep running because the
daemon does not own them, which stopped being true when projects moved into
the daemon. "Normally", because a pane host whose runtime files were lost is
not found again and its tab gets a fresh agent.

A requested restart **applies an available update first**, via
`prepare_cli_restart`, so `check_for_update_available` syncs the git repo and
pull/build/install all complete while the current daemon is still serving; a
failure leaves that daemon working rather than the machine with none. It is what
lets a CLI update roll out from a phone instead of an SSH session. A version
that only touches the web frontend or docs skips the rebuild
(`pending_update_needs_rebuild`). Progress past "requested" is deliberately not
reported — the daemon replaces its own process image, so anything further would
have to outlive the process that would report it.

It upgrades only. Equal, older, or unparseable versions replace in place;
an accidental downgrade across a cluster sharing one NFS home would be painful
to unpick.

**The cost of removing the tick is that a host nobody visits keeps its version
forever** — exactly the zoo-002 problem, deliberately reaccepted. What makes it
tolerable is that the machine lists now *show* it: each machine displays the
version its daemon reports, and its restart control reads "Reboot to update"
when that version is behind the newest one the client can see. The old failure
was that nothing surfaced it.

**It re-execs rather than spawn-and-exit**, which matters for three reasons:

- the pid is preserved, so `daemon.json` stays correct and
  `detect_running_daemon` is not briefly fooled into starting a second daemon,
- the session is preserved, so it stays `setsid`-detached,
- destructors do **not** run, so `RegistrationGuard` never withdraws the host
  record or releases project claims. Spawn-and-exit would open a window where a
  peer daemon sees those projects unclaimed and could spawn a duplicate CLI
  against the same `.apas` and worktrees — the race the claim system exists to
  prevent.

**Stopping it stops its projects.** The signal handler is registered with
`ctrlc`'s `termination` feature, so a SIGTERM sets the same shutdown flag a
SIGINT does. Without that only SIGINT is handled, and a `setsid`-detached daemon
essentially never receives one — the teardown that saves each project's pane
roster and ends its agent subtrees would be unreachable, and nothing would look
any different. A test fails if the feature is ever dropped, for the same reason
one fails if `panic = "abort"` is ever set.

**Claims are reconciled at daemon startup.** `reconcile_running_claims` claims
every project this host is already running, because claims are otherwise only
taken in `start_project` — so a restarted or self-upgraded daemon would come
back owning nothing while its project CLIs kept running. During that gap a peer
sees the projects unclaimed, and its `is_headless_running_for` only reads its
*own* `/proc`, so a `StartProjectCli` there would spawn a second CLI against the
same `.apas` and worktrees. A peer holding the project's guard without the
instances marker, for something running here, is logged, not seized: that
means a runtime that cannot share the project is live beside this one, which is
exactly what the operator needs to see. Another machine's instance of the same
project is not such a peer: capable daemons each hold a per-machine claim and
share the marked guard (see "A project's tabs can run on several machines").

Relatedly, `claim_project` adopts the **current pid** when refreshing a claim
owned by this hostname. A claim written by a previous daemon carries that
daemon's pid, and `refresh_own_claims` only refreshes claims whose pid matches
the running process — so keeping the old pid left the claim un-refreshed, stale
within `STALE_AFTER_SECS`, and free for a peer to take while we were actively
running the project.

**Pane hosts are untouched.** They own the PTYs in their own tmux sessions, so
nothing about replacing the daemon's process image reaches them, and a
replacement that lands inside their adoption grace picks the terminals back up.

## A project's tabs can run on several machines

One project can run on several machines at once, and each tab runs on the
machine chosen when it was created. The project keeps **one session**, one
history, one sharing and one policy; what multiplies is the **instance**, one
machine's runtime of the project inside that machine's daemon. (This is not the
per-host APAS process of "One instance per user per host": a daemon can run one
instance each of many projects.) Tabs never move between machines. The design
record is `openspec/changes/archive/2026-11-13-run-project-tabs-across-machines/`;
the doc comments on the contract types in `crates/shared/src/messages.rs` are
the binding statement of the wire.

**The server keeps every instance as an attachment of the one session**
(`session/instances.rs`). The **primary** is the earliest still attached. It
supplies the single-valued fields older code reads (`cli_client_id`, the DB
row's host and working dir, the summary hostname) and receives controls that
name no machine. When it leaves, the next-earliest takes over and keeps the
role after the first returns. Only the last instance to leave marks the session
inactive; a disconnect otherwise touches only the departing machine's
terminals, statuses and inventory. Capable viewers get the instance set as
`SessionInstances` on every attach, detach and handover, with `runtime_access`
evaluated per machine for that viewer.

**Every pane records its machine** in `PaneConfig.machine_id`/`hostname`. A
capable instance restores, reports and saves only its own panes, tagged, and
the server re-tags whatever an instance reports from its authenticated
attachment, so no CLI can claim another machine's tabs. Rosters merge per
machine: `Some(list)` replaces that machine's panes, `Some([])` means it has
none, `None` changes nothing, the same empty-versus-unknown rule as "Zero panes
is a state". An untagged pane predates instances and belongs to no machine
until an instance reports it; the server stamps an **attributed host** on it
for display and routing only, never as ownership.

**Tags are permanent, by the owner's decision.** Starting a project on zoo-006
after stopping it on zoo-005 used to resume zoo-005's tabs there. Now they stay
listed as stopped on zoo-005, with their ids, until zoo-005 runs the project
again. While any instance is attached, controls and closes for such a tab are
refused with "<host> is not running this project. Start it on <host> first.";
the web dims the tab and offers **Start on <host>**, and the sidebar's pane dots
read "Not running on <host>" and leave it out of idle and waiting lists.

**Routing is by message scope, and a new variant does not compile until
someone classifies it.** `ServerToCli::scope()` matches every variant with no
wildcard arm, and a test pins each classification against the variants read
from the type's schema. `route_to_cli` sends a pane's controls only to the
instance that owns the pane, a new pane to the machine the server assigned it,
project-wide messages (policy, flags) to every instance, and a reboot or
lifecycle request to the machine it names, else the primary. A request naming a
machine that runs no instance is refused, never delivered elsewhere, and names
that machine's host only to a requester who knows the machine. Inbound
pane-scoped frames are accepted only from the owning instance, which also
closes the old hole where a replaced CLI kept writing into a session.

**A shared home shares one `.apas`, so every write is a locked merge.** Writes
take an exclusive lock on `<config dir>/locks/<project-id>.lock`, re-read the
file, replace only this machine's panes and keep everything else as read. The
lock is deliberately not a sidecar in the project directory: repositories
ignore `.apas` by exact name, so an `.apas.lock` would show in `git status`
where an agent could commit it. Periodic reads (the 5 s flag rebroadcast, the
reconnect policy snapshot) use a reader that never deletes or regenerates
`.apas`. Untagged panes are adopted at start, under the lock, only by the
earliest live claimant, so two machines starting together tag each pane once.

**Claims are per machine and released on stop.** A capable daemon claims
`claims/<project>/<machine>.json` and also holds the legacy guard
`claims/<project>.json`, marked `"instances": true`. Older daemons ignore the
marker and see a live peer, which keeps them out; capable daemons treat a marked
guard as compatible and an unmarked live one as a runtime that must run alone.
Claims are released on every stop path, which fixes the old bug where a project
stopped on one host stayed claimed and could not start on another. A refused
start is reported as the project's `last_error`, never skipped silently, and a
start carrying a `request_id` is answered with exactly one
`ProjectRuntimeStarted` on every outcome path. The heartbeat refreshes each
running project's claims in the shape its start took.

**An absent claim is believed only after a second look.** Releasing on stop
means a host has often just removed a claim when another host writes it again,
and the NFS client caches that absence for as long as it trusts the directory's
attributes: 30 s on the shared home. `daemon_registry::read_json` therefore
opens the nearest existing directory above a missing claim, which makes the
client revalidate it (close-to-open), and reads again; a listing needs nothing
extra, because `read_dir` opens the directory. Without it, the 2026-11-12 smoke
run saw a host that had just released a project's guard write its own over a
peer's guard written a second earlier. Any new cross-host file whose absence
decides something needs the same treatment.

**Choosing a machine.** The `+` menu on desktop and the mobile-browser pane
sheet gain a **Run on** step, hidden only when exactly one eligible machine
already runs the project: running instances first, then registered machines
("Starts the project first"), then owned machines without the project
("Clone here"), then offline or older machines disabled with their reason. The
request is `AddPane { machine_id, request_id }`, answered by `AddPaneResult`
(`Starting`, then `Routed` or `Failed`) to the requesting connection only.
`Routed` is delivery, not creation: the tab appears when its instance reports
it. A stopped target is started first (`StartProjectCli { request_id }`) and
the tab is routed once that instance's `SessionStart` has been handled,
failing at once on a refusal and after 90 s without an attachment. A launch
that never hears a final result is shown as unconfirmed and **never resent**:
a replay can create a second tab. Native task launches follow the same rule
once a capable instance runs the project.

**Project-level Start and Close act on every machine; the Machines page does
not.** The project view sends `all_instances: true`. Start starts every online,
capable machine that owns tabs of the project (or only the given machine when
none does). Close stops the given machine, every attached instance, and every
connected daemon of the project's own accounts that reports running it while
its instance is not attached (still starting, or reconnecting). The Machines
page's per-machine Start and Stop keep `all_instances: false` and must never
reach other machines.

**Forget is roster-only.** **Forget <host>'s tabs**, for a machine that will not
come back, removes its tabs from the server roster and tells no instance:
deleting them from a shared `.apas` would remove exactly the entries that
machine restores its tabs from. Their ids are remembered in
`sessions/<id>/forgotten-panes.json` and avoided by new tabs, and the tabs come
back with their ids if that machine runs the project again. UI copy must say
so and never promise deletion. The server refuses it while that machine runs an
instance, or while a CLI without instance support runs on its host.

**A refused send is settled, never resent.** Typed input refused because of its
tab's machine is answered with `MutationAck { mutation: input, accepted: false }`
carrying the input's `client_msg_id`, to the sending connection only and only
when it negotiated `project_instances_v1`, in addition to the `Error`. The
server records only the ids of input it delivers, so a resend would be judged
afresh and, once that machine runs the project again, typed into the terminal
minutes after the user saw it refused. The web drops the send from its pending
queue in memory and storage and marks its bubble "Not delivered" with the
reason and the text kept; native retires that message id and keeps the id of a
send that was merely never acknowledged, so a retry is still recognized.

**A separate checkout joins by git remote.** Daemons report each project's
canonical `git_remote`, read in the background, so it reaches the server from
the heartbeat after registration; it also files a registration-only entry under
its repository in the sidebar instead of "(no remote)". `apas register` asks
`GET /projects/by-remote` (under the already proxied `/projects/` prefix) for
the caller's projects with the same remote. One match is offered with a
prompt; several need `--project <id>`; `--new` skips matching; `--yes` or
`--project` is required when no one can answer. Only a pristine local identity
(no tabs, no server session) is ever re-identified, so no history is orphaned,
and a checkout the shared registry already holds at another path is refused:
start it there instead. From the web, **Clone here** sends
`CreateProjectInstance { join_project_id }` to an owned machine whose daemon is
capable. That daemon clones without a new branch into `~/apas_projects/<repo>`
(suffixed if taken), writes `.apas` with the project's id and no tabs,
registers it, reports it in a heartbeat before acknowledging, and does not
start it; the web then adds the tab, which starts the instance.

**A project only a registration knows can be started from the web.** `apas
register` and a bare `apas` tell the server nothing, so such a project has no
record or placement, and every Start used to fail as "Machine not found". A
Start naming the requester's own machine that lists the project now creates
the record and placement first, through `authorize_project_registration`.
Native launch still cannot offer such a project, because its targets come from
project policy, which needs that record: start it once from the web or the
Machines page.

### Capability and mixed fleets

`project_instances_v1` is negotiated separately by CLIs, daemons and viewers.
A host whose `config.daemon.machine_id` is set never advertises it: that id is
shared through the NFS home and cannot tell machines apart.

- **A CLI without it** can only be a session's sole instance. Its
  `SessionStart` is refused while another machine's instance is attached, and
  a capable instance's is refused while it runs. A Start of the project on
  another machine is refused before any daemon frame with the text that
  machine's `SessionStart` would get: "This project is running on <host> with
  an APAS CLI that cannot share it with other machines; stop it there, or
  reboot <host> from the Machines page to update it, then start it here".
- **An older daemon** can still start a project on a shared home whenever no
  capable instance runs it. It restores every tab in `.apas`, other machines'
  included, and strips the tags when it saves: the old host migration, bounded
  by the claim guard and repaired when the next capable instance adopts the
  untagged tabs. Nobody should be surprised when an un-rebooted host picks up
  another machine's tabs. While an instance runs the project, a Start naming
  such a daemon's machine is refused before any daemon frame with "<host> runs
  an older APAS that cannot run this project beside other machines. Reboot
  <host> to update it.", the reason a tab targeted there gets: the daemon would
  otherwise skip it without a word on a shared home. The check reads attached
  instances, so just after a server restart, before the running instance has
  reattached, such a Start still reaches the older daemon.
- **A capable daemon relies on instances only while the server echoes
  `project_instances_v1` in `ServerToDaemon::Registered`**, and decides again at
  every registration. On a server that does not echo it, the daemon refuses a
  project another live host runs and runs anything else as the project's only
  runtime, as an older daemon would: every tab, saved untagged, the capability
  not advertised, and the unmarked guard alone, so a capable peer stays out even
  after the server is upgraded. Nothing running is changed by a later
  registration. The resume manifest records whether the server kept one CLI per
  session, so a requested reboot resumes such a project alone where no other
  host runs it and otherwise as an instance; on an upgraded server, that reboot
  is what turns it back into an instance.
- **Older viewers** never receive `SessionInstances`, `AddPaneResult` or the
  input acknowledgement: older native builds drop the socket on a frame their
  bundled schema rejects. They read the instance list from `SessionInfo`, and
  receive only the primary's lifecycle inventory.

### Rollout and rollback

Deploy **server → web → native → CLI/daemon**. A new server with only older
CLIs behaves as before, and existing `.apas` files need no migration: a
pre-upgrade roster's untagged tabs are shown under the session's recorded host
and are tagged when a capable instance adopts them. Hosts gain the capability
when their daemon is rebooted to update from the Machines page, which restarts
every project on that host; do that only with the machine owner's go-ahead.
After deploying the server, check the join route through nginx:
`curl -s -o /dev/null -w '%{http_code}' 'https://apas.mpaxos.com/projects/by-remote?git_remote=github.com/x/y'`
must answer `401` from `apas-server`, not `200` with the app's HTML.

To roll back, first stop the extra instances (stop the project on its
non-primary machines), then roll back CLI/daemon, web and server. An older CLI
ignores tags and an older server treats a multi-machine roster as one list,
which is safe once only one instance runs. A capable daemon re-registering with
a rolled-back server starts no further instance beside another live host, but
never stops one already running. An instance left running under a rolled-back
server reports only its own tabs, which that server takes as the whole roster;
the other machines' tabs stay in their `.apas`, and starting the project there
runs it as its only runtime, which lists them again.

### Verification

**Verified on 2026-11-12 and rolled out on 2026-11-13** (see "Rollout record"
below). At the change's final commit the Rust
workspace (shared 83, server 371 and CLI 562 tests), the protocol
(54 tests and typecheck), web (883 tests, lint and typecheck) and native
(113 tests, lint and typecheck) checks pass, as does
`openspec validate --strict`.

A two-host smoke ran on zoo-005 and zoo-006 with isolated servers and daemons
on disposable projects, beside the production daemons, which it never touched.
It covered shared-home and separate checkouts, headless Chromium at desktop and
phone widths, a server rolled back and forward, and pane-host adoption.

- Tabs on two machines each took input on their own machine.
- Closing one machine's last tab, freezing one machine's daemon until the server
  dropped it, and rebooting a secondary instance's daemon all left the other
  machine untouched.
- A project stopped on one machine started on the other, with every tab staying
  with its machine.
- Forgotten tabs came back when their machine ran the project again.
- Checkouts joined by remote and by clone.
- Older daemons and CLIs were kept out.

The smoke found four defects. All four were fixed and re-run on both hosts:

- the stale NFS read of a claim described above. The stale view appeared in all
  nine re-run trials, with marked and unmarked guards, and the peer's guard held
  every time;
- an older daemon silently skipping a Start;
- `apas register` printing a refusal twice;
- a stopped machine's tab label and terminal line.

The re-run found two more issues: a phone's tab chip hid its machine's state,
and a terminal left open across a server restart showed no retained-output
line. Both were fixed afterwards. They are covered by unit tests and a
Chromium layout check of the chip at 360–390px, not by another two-host run.

Not exercised:

- the interactive `[Y/n]` join prompt;
- **Clone here** in a browser (only its protocol frames ran);
- an older CLI's `SessionStart` rejected beside an instance;
- the web's handling of a refused send (the UI disables sending to such a tab);
- the start-then-add failure paths;
- the native app on a device.

### Rollout record

On **2026-11-13** the change reached production in two steps.

- **Server and web** have run release `20261113T013925Z` since 01:48 UTC. It
  was built at e02c0f7, so it carries this change together with the worker
  cutover, and it was deployed with that cutover, not separately. Its backups
  are under `/opt/apas/backups/deploy-20261113T013925Z`.
- **CLI:** `master` was pushed at e386a1d, and again at 79c909a with two
  documentation commits. No daemon restarts by itself. Each machine takes the
  new CLI at its next reboot from the Machines page, which stays its owner's
  action. Until then a machine keeps the October CLI and cannot share a
  project with other machines.
- **zoo-005 and zoo-006** were rebooted from the Machines page at 07:43 UTC
  and run 26.11.16. zoo-005 built the CLI in about a minute and zoo-006
  reused that build. Each daemon registered again within a second, all nine
  project instances re-attached, and the server logged no error. On zoo-005
  the new daemon holds a control connection to each of its 18 terminal
  hosts, so every running agent there was kept. On zoo-006 two terminals
  were not kept, for the `/tmp` reason described under "Persistent pane hosts
  and CLI lifecycle": each got a fresh provider, and the two old ones ended
  with their hosts' reboot lease 15 minutes later.

Checks at the pushed commit:

- The join route answers `401` with JSON from `apas-server` through nginx.
- The Rust workspace (1,072 tests), the protocol (57 tests and typecheck),
  native (136 tests, lint and typecheck) and the web lint and typecheck pass.
  The web tests passed 954 of 956 in a run that shared the host with two
  builds. The two failures were worker-run component tests that timed out, and
  they pass when run alone.
- The CLI builds as the updater ships it (static musl, release).
- On zoo-005 and zoo-006, with an isolated server and daemons beside
  production, the end-to-end run passed: both machines ran one project, and a
  tab was routed to its machine and took input.
- Every restart phase of the pane-host scenario passed there too (183 checks):
  three reboot paths of the secondary machine, a crash and restart, and a
  reboot of the primary, each taking the same live terminal back.

The last phase of that scenario failed, and the reason matters in operation.
**A machine can look offline for about 30 seconds after its daemon is
rebooted, and a Start, Stop or project-wide Close aimed at it in that window
does not reach it.** In the run, a project-wide Close sent four seconds after
rebooting zoo-005 stopped zoo-006's instance and left zoo-005's running.
zoo-005's project had re-attached at once, but its daemon's own channel
re-registered 30 s later; the other restarts took under 0.2 s. The suspected
cause is the registry lock: `with_project_registry_lock` takes a blocking
`flock` in the shared config directory, and on this NFS 4.1 mount a lock that
the server denies is retried only after 30 s. That code predates this change
and is in the October CLI too. The cause is not confirmed and nothing is
fixed. Until it is, wait for a rebooted machine to show as online before
acting on its projects.

## Pane work summaries: which summarizer

The desktop summary drawer can be resized from its left divider, by dragging
or using the arrow keys. Its width is a browser-local preference under
`apas_layout_global_summary_width`, shared across tabs and projects. It fits
the available workspace when the window or project sidebar changes size;
the phone's summary sheet keeps its existing layout.

The resizable drawer was deployed on **2026-11-14 at 18:29 UTC**, web release
`20261114T182726Z`, version **26.11.50**, built at `8dbba50`. Backups are under
`/opt/apas/backups/deploy-20261114T182726Z`. Both dependency graphs audited
clean, and the staged production build passed. Ten public routes, 23 referenced
assets and authenticated worker JSON passed smoke checks. On the live
chiral-network OMP 2 tab, dragging, arrow-key resizing and reopening at the saved
width worked. Its original terminal process stayed running. This was a web-only
release; the server, daemons and job policy were unchanged.

A tab's work summaries are written by a separate, headless run of a
summarizer on the tab's machine, configured in the CLI's `[summaries]`
section (`apas config set summary_enabled|summary_adapter|summary_model|
summary_allow_cross_provider ...`; the settings are read when a project
starts, so a change needs a project restart or a daemon reboot). Adapters:
`claude`, `codex` and, since 2026-11-14, `omp` (`crates/client-cli/src/
summary_runner.rs`). A job for a tab whose provider differs from the
summarizer's is refused with "Cross-provider summarization is disabled on
this CLI host" unless `summary_allow_cross_provider` is true. The check
compares APAS tab types, not backends: an OMP tab running on the OpenAI/Codex
login is still "cross-provider" to the Codex summarizer.

The owner's choice (2026-11-14) is `adapter = "omp"` with cross-provider
allowed, so every tab is summarized by OMP with OMP's own default model and
backend. The OMP adapter runs `omp --print` with the prompt on stdin, in a
fresh temporary directory, with `--no-session --no-tools --no-extensions
--no-skills --no-rules --no-context-files --no-lsp --no-pty --no-title
--approval-mode always-ask-cached`, a fixed system prompt, and a settings file
passed with `--config` that turns off MCP and custom-tool discovery
(`disabledProviders`), magic keywords, memory, the advisor, stream rules,
model fallback and telemetry. `@` and `^` in the summarized records are
escaped, because OMP would otherwise read a mentioned file into the prompt.
The run gets its own process group, which is killed on timeout or when it
ends. At startup the CLI checks `omp --help` for every flag it needs and
leaves the summarizer off, with a log line, if one is missing.

What remains, and what the warning printed by `apas config set summary_adapter
omp` says: tools of OMP plugins installed under `~/.omp/plugins` still load,
and one allowed in OMP's `tools.approval` would run; a model served by OMP's
`opencode` or `cursor` provider is not used, because those names are in the
disabled list; and OMP's detached daemon broker may outlive a job until its
idle grace ends. Checked on OMP 18.20.0 with DeepSeek V4 Flash in the isolated
harness (`~/.cache/apas-autopilot-check/NOTES.md`): a parsable summary in 4 to
8 s, no session file, no context file or extension loaded, no built-in tool
offered, mention escaping effective, and failures (no key, wrong key, unknown
model, timeout) returned as failures. Not tried: credentials stored in
`agent.db` (the owner's OAuth login) with the final settings file, and
`~/.claude.json` MCP servers. Re-check when OMP's minor version changes.

## Terminal-pane history: read the provider's transcript

An agent pane is *observed* — the CLI parses its stream-json and knows every
turn without cooperation. A terminal pane hosts the provider's real TUI on a
pty, so there is nothing structured to parse, which is why terminal panes had
no history and no usage counters.

**Self-reporting via MCP was tried first and does not work.** A `record_turn`
tool was added and the requirement stated in the MCP server's `initialize`
instructions. Tested against both providers: each connects to the server and
*will* call the tool when told to directly, but neither acts on the
`initialize` instructions — an ordinary task ("what is 17 times 23?") recorded
nothing at all. Those instructions are advisory and both clients treat them as
such. Do not reach for that mechanism again expecting a guarantee.

So the CLI reads the transcript each provider already writes. It needs no
cooperation, cannot be skipped, and carries token usage the agent would
otherwise have had to volunteer.

**Locating the file differs by provider, and so does the confidence:**

- **claude** — **the provider reports its own transcript.** Each Claude pane is
  spawned with `--settings <pane file>` installing a `SessionStart` hook that
  runs `apas session-hook`; Claude passes it `transcript_path` on stdin, and it
  records that under
  `${XDG_RUNTIME_DIR}/apas/panes/<project>/<pane>/claude-session.json` (0600,
  written temp+rename so a poll cannot read a half-written file). The watcher
  reads that file.

  The deriving path — `--session-id <pane's session_id>` pinned at spawn, and
  `~/.claude/projects/<cwd with / replaced by ->/<session-id>.jsonl` — remains
  as the fallback, and both halves of it are wrong as soon as the user acts.
  Claude Code can move a session into `.claude/worktrees/<name>` and then writes
  under **that** directory's slug; and `/resume` onto another session appends to
  **that session's** file, so the pinned id names a conversation the pane has
  left. What used to cover the gap — follow the newest transcript in the
  directory — cannot tell our pane switching files from an unrelated `claude`
  running in the same directory, and on a live pane it published 607 records of
  someone else's conversation while the pane's own sat unread in a worktree.

  Three things were verified against Claude Code before building on it: the hook
  fires on `startup` **and** `resume` carrying the absolute `transcript_path`;
  `--settings` **merges** with the user's other settings layers rather than
  replacing them, so the pane keeps its owner's model and theme; and the hook
  process **inherits the provider's environment**, which is how `APAS_PANE_RUNTIME`
  identifies the pane. A `claude` a person runs by hand has no such variable, so
  it records nothing and can never be adopted.

  This is deliberately unlike the `record_turn` MCP tool below, which asked the
  *model* to cooperate and was abandoned: a hook is run by the client, not
  chosen by the agent. Never substitute `--continue`: it can select another
  pane's most recent cwd conversation and silently disconnect terminal output
  from APAS's transcript watcher.
- **codex** — cannot be given an APAS-chosen id at creation. On Linux, APAS first
  checks the terminal's process group and its directly launched Codex app-server
  child for an open user rollout. Codex 0.157 writes through that child in a
  separate process group. Only direct `codex app-server` children qualify;
  content-addressed native binaries also qualify when the child runs the exact
  same executable as its TUI. Searching arbitrary descendants can adopt a
  separate Codex session started by one of the agent's tools.

  **Codex 0.159/0.160 can instead use a shared managed daemon.** Its rollout
  descriptors belong to neither the TUI's group nor its children, which left all
  three TLA–RS Codex conversations empty while their terminals worked. APAS
  queries the user's existing Codex control socket for loaded-thread metadata
  and each thread's live `codex_tui` MCP origin. That loopback listener must be
  owned by the pane's process group, and the rollout's id, cwd and user-thread
  metadata must agree. Shared cwd or daemon ancestry alone proves nothing.
  Multiple matching user threads are ambiguous and never selected by recency.
  Queries are batched across panes and bounded to two seconds per poll.
  This reads client-maintained metadata; it neither invokes an MCP tool nor
  asks the model to report itself, and it sends no prompt to the provider.

  **The locator must not change the user's Codex daemon.** It initializes as
  `codex_app_server_daemon` with no capabilities (`initialize_params()` in
  `transcript/codex_daemon.rs`, whose doc comment cites the Codex source).
  Codex treats every other client name as a new originating client and then
  rewrites daemon-wide state: the User-Agent suffix of every later model
  request, including the person's own typed turns, the originator if none is
  set yet, and the automatic-login decision; the `explicitGatewayOauth`
  capability changes login handling whatever the name. Until 2026-11-14 the
  locator used `apas_transcript`, and a private 0.166.0 daemon showed the
  person's next typed turn going out as `(apas_transcript; 0.0.1)`. Re-check
  `NON_ORIGINATING_CLIENT_NAMES` in Codex's `initialize_processor.rs` on
  every Codex upgrade.

  The last verified path survives brief descriptor/control-socket gaps; an
  unambiguous newly owned rollout replaces it on resume/fork. Its
  `session_meta.id` replaces the provisional pane UUID in `.apas`, so future
  restores use `codex resume <id>` directly. An older pane keeps the picker
  until its identity is verified; `--last` is never used because it can select
  a sibling pane in the same cwd. Other platforms fall back to the newest user
  rollout whose cwd matches, but that ambiguous fallback is not persisted as
  pane identity. Restored panes still baseline existing turns instead of
  replaying the whole file: repairing previously missing history requires a
  separate bounded recovery, not resetting every pane's cursor.

  Codex's rollout files, authentication and configuration stay in the user's
  ordinary `CODEX_HOME`, which can be shared by the cluster. Its WAL-mode
  SQLite runtime state cannot: APAS sets `CODEX_SQLITE_HOME` on every Codex
  pane to the private host-local `/var/tmp/apas-codex-sqlite-<uid>` directory.
  The launcher creates or tightens that directory to `0700` and rejects a
  symlink, non-directory or path owned by another user. Never move this path
  back under the NFS-mounted home; different hosts may run Codex concurrently.
- **opencode** — OpenCode owns its `ses_*` identifiers, so APAS asks
  `opencode session list --format json` for the newest session whose directory
  exactly matches the pane cwd, then reads it with `opencode export <id>`.
  Like Codex, two panes sharing a cwd are inherently ambiguous; sessions from
  another directory are never selected.
- **omp** — no `--session-id` exists, so the pane is pinned by its own
  `--session-dir` instead (see "Terminal panes" below). New panes report
  their exact transcript through the pane-local activity extension below.
  Already-running panes use the newest flat `.jsonl` in their private directory.
  `PiTranscriptReader` reads it. The reader keeps the name of the Pi session
  format, which OMP writes; a real OMP capture guards that format. Raw Pi
  sessions under `~/.pi` are no longer located or read: Pi was retired on
  2026-11-13 and its lookup went with it.

Parsing keeps only real conversation. claude transcripts also carry `mode`,
`ai-title`, `last-prompt` bookkeeping; codex carries `developer` messages (the
harness's own injected context), `reasoning`, and tool calls; OpenCode exports
typed reasoning/tool/synthetic parts; OMP sessions, in the Pi session format,
are trees whose `custom` and `custom_message` extension entries, thinking, tool
results, shell executions, and compaction/branch summaries are not turns. None
of those are turns, and rendering them would be noise. For OMP, only the active
parent chain is read, so an abandoned branch never reappears in history. Tool-use-only turns with no text are skipped rather than
recorded blank. In-progress OpenCode assistant messages are held until their
completion timestamp arrives so APAS never advances its cursor over a partial
reply.

**OMP activity is not conversation text.** A background `launch-completion`
or `async-result` can resume work without another user message, and tool-only
assistant rounds contain no displayable text. Inferring activity from chat
left `OMP-mako-rs` Idle for days after an aborted turn, even while it continued
running tools. The watcher now reconciles Working, Pending answer, and Idle
independently of its chat cursor, including on restore and reconnect. Replaying
old conversation is never needed to recover activity.

New OMP launches, on Linux and macOS, explicitly load APAS's embedded
`--extension` in both
direct and hosted PTYs. It observes provider lifecycle events, preserves
Working when `agent_end.willContinue` schedules continuation, and reports an
unresolved `ask` as Pending answer. Reporting starts before OMP creates its
first JSONL, so initial reasoning is no longer invisible. Verified with real
OMP 18.3.2 and 18.6.0 through Working → Pending answer → Working → Idle.
These are client events, not model-chosen MCP calls; `stopReason: "stop"` can
still carry executable tools and is not a substitute.

The extension writes `omp-activity.json` by private temp-and-rename under the
same host-local `apas/panes/<project>/<pane>` runtime tree as Claude's hook.
Its launch id, provider pid and exact session-directory scope must match, and
that provider must still be running; a file left by an earlier launch and a
subagent that inherited the tab's environment cannot report for the tab. The
files stay owner-only (0700 directory, 0600 files, no symlinks).

**The process start time is no longer checked** (owner, 2026-11-15: "we
should trust the user env (if there is a malicious process on the server, we
cannot protect ourselves anyway)"). The launch id and pid checks stay because
they are for correctness, not security. What the start time also told, that
the writer is alive, now comes from `provider_alive`: the pid leads its own
process group (`getpgid(pid) == pid`), the same on Linux and macOS.

- The extension reports only from the process APAS started: `prepare()` sets
  `APAS_OMP_PARENT` to the spawning process's pid and the extension requires
  `process.ppid` to equal it. A wrapper script's child or a subagent process
  stays silent. Anything that starts OMP with the built extension by hand,
  the check harnesses included, must export `APAS_OMP_PARENT=$PPID`.
- A silent extension is logged, once per agent process: when a tab launched
  with `portable_v1` has been running for 60 seconds and no report of it was
  ever read, the daemon warns that the tab has transcript status only,
  terminal-only questions and an autopilot that reads "not reported".
- `process_start` is still written on Linux and never read by this CLI. A new
  pane host runs the installed binary, so a new extension can sit under a
  daemon that has not been rebooted, and that older CLI requires the exact
  value. Do not remove the field until no such daemon is left.
- The launch record carries `"features": ["autopilot_v1", "portable_v1"]`.
  Off Linux, a record without `portable_v1` belongs to an extension that
  turned itself off, so its tab asks for an agent reboot.
- A provider that died while one of its descendants keeps the terminal open
  is not reaped until the terminal closes. On Linux the CLI reads its state
  from `/proc/<pid>/stat` and does not count it as running. Where `/proc`
  cannot be read it counts as running until the terminal closes, and its last
  report can read Working until then; autopilot state is bounded by the
  90-second staleness rule.

Activity reports contain no conversation or
credentials. The separate autopilot state report (`omp-autopilot.json`) does
carry the autopilot's prompt; see "Autopilot on OMP terminal tabs".
No global OMP settings or extensions are replaced.

Existing pane hosts are adopted without restarting providers, so they keep
using active-branch transcript evidence until their next normal provider
launch. Tool-start/result pairs recover pending questions, and autonomous
tool rounds recover Working. This evidence cannot reveal unrecorded streaming
or queued recovery as precisely as the live extension. OMP's extension API
also has no terminal-settle notification for potential background work
cancelled without another lifecycle event; do not fabricate one from elapsed
time or file inactivity.

**A three-second poll must not hide a two-minute parse.** OMP parent chains
are indexed once rather than repeatedly searching all preceding records.
The old reader took 127.8 seconds on a live 158 MB transcript and blocked
every other pane in that project's serial watcher. Parsed snapshots are now
borrowed unchanged until file identity, size, or timestamps change; append,
replacement, and truncation invalidate the cache. Only active-branch records
are decoded, and tool payloads are not all retained in memory.

**Agent questions appear in the conversation view, and can be answered there.**
The parser drops tool-use-only turns as noise — right for `Bash`, wrong for
`AskUserQuestion`, which has no text either but is the one turn the human has
to act on. Across the transcripts on one machine, 170 questions were recorded
and 169 were being discarded, so a terminal pane could sit blocked on a
question nobody could see. A question turn is now published as the `tool_use`
block it already is, so the web's existing `AskUserQuestionCard` renders it
with no new wire message, storage path, or renderer.

Claude answers reuse the `AnswerQuestion` pipeline agent panes use (web →
server → CLI); only the last hop differs, because a terminal pane has no
stream-json control channel. The CLI writes **keystrokes** to the pty instead:
`↑/↓` to navigate and `Enter` to select, which is the contract the picker
prints in its own footer — verified by driving the real TUI (Claude Code
2.1.233), where digits notably do *not* move the selection. The agent's options
occupy positions 1..N with the TUI's own `Type something` / `Chat about this`
beneath them, so stepping down by an option's index can only land on an option
the agent offered.

**The answer is confirmed by reading, never by writing.** A successful pty
write proves only that bytes were accepted, so the acknowledgement is the
`tool_result` the provider records, republished as `User` + `ToolResult` — the
one variant the server's converter reads for tool results, which is exactly why
ordinary non-assistant turns avoid it. A Claude terminal pane has no structured
echo, so its recorded answer arrives as prose (`The user answered: "Q"="A"`)
and is parsed, letting the card settle on what the agent *took* rather than on what
was clicked. Pending state is derived the same way — a question is open while
its `tool_use` has no `tool_result` — which is what makes a blind write safe:
a stale tab, a retransmit, or a question already answered in the terminal all
send nothing.

Terminal conversation sends need the same distinction. `UserInput` is the
server's forwarding echo, and the optimistic bubble is local UI state. Neither
proves that a provider recorded the prompt. A hidden OMP selector consumed a
mobile prompt this way: it ignored the text and treated Enter as consent, while
APAS displayed Working. The server now emits `TerminalConversationRecorded`
with the original `client_msg_id` when its transcript correlation is consumed;
only that observation starts Working and counts a prompt. The web keeps
unconfirmed sends separately, scoped by session, pane and message id, and
persists them for recovery across refreshes. After ten seconds it offers the
raw terminal and draft recovery. Never automatically replay these PTY writes
or infer receipt from matching chat text. A missing acknowledgement remains
unconfirmed, including when a disconnect loses it. Provider menus are still
opaque, so this does not make a blind Enter safe for answering them.

**A pane blocked on that question is Pending answer, not Working or Idle.**
The server caches one canonical pane status when `AskUserQuestion` arrives and
reports it separately in pane/session summaries, so project lists, waiting-agent
lists, pane selectors and conversation status bars all agree. Pending answer
does not advance idle recency and takes presentation precedence over a cached
provider usage limit: it names the action the human can take now. Merely routing
answer bytes does not clear it; the matching transcript `tool_result` is the
proof that moves the pane back to Working until the resumed turn completes.

**OMP's built-in `ask` uses structured answers, never terminal keystrokes.**
Its active-branch questions and native results are normalized into the same
conversation card on desktop and mobile. Native question IDs remain distinct
even when question text repeats; selected option labels and custom text remain
separate, including labels containing commas and empty multi-select answers.
Recommended options, descriptions, and plain-text previews are retained.
Cancellation is an explicit native result, not a fabricated empty answer.

The embedded activity extension mediates the main agent's real
`ctx.ui.askDialog` and leaves the native picker usable. Prepared arguments do
not prove which call is executing: `tool_execution_start` binds the dialog to
its exact call, whether it arrives before or after the picker opens. Ambiguous
overlapping calls fail closed to terminal interaction. The first native or web
response settles the dialog; a late response cannot send stray terminal input.
This is a local extension bridge, not MCP, an external relay, or an RPC-mode
replacement for the TUI.

`AnswerOmpQuestion` carries the session, pane, opaque tool ID, dialog generation,
request ID, and typed answers or cancellation. The server checks live runtime
access and claims the exact pending question. The CLI validates the launch,
the provider's pid and that it is still running, native session, transcript scope, and current dialog
before using bounded, owner-only local files under the pane's activity runtime.
Its two-second exchange waits for native acceptance, not just a successful
write. Acceptance means submitted; only OMP's recorded `tool_result` confirms
the answer. Rejected or uncertain submissions are never automatically replayed.
An explicit corrected retry is offered only when the server confirms the
question is still pending.

Question reconciliation runs independently of the ordinary chat cursor, so
restore, reconnect, and branch changes recover questions without replaying old
prose or usage. Repeated metadata updates upsert the same card. Native terminal
answers, cancellation, and invalidation replace any local submission state;
an unavailable question is never displayed as answered.

The CLI advertises `omp_question_answers_v1`; older CLIs cannot receive this
command. A missing native API or old loaded extension leaves a terminal-only
card. Deploy server → web → CLI, and let existing providers keep running:
adopting a pane host does not reload its extension. The new bridge becomes
available on the next normal provider launch, not by silently rebooting agents.
Verified with real OMP 18.8.7 through mobile and desktop answers, cancellation,
native-terminal precedence, browser refresh, and a server restart that preserved
the provider PID. A production mobile answer was also verified with OMP 18.8.9.
The native UI API remains version-sensitive.

OpenCode has no question interface. Adding another provider requires
verifying its picker contract, not assuming Claude's keystrokes or OMP's
native API.

**The conversation view is writable, and that is the point on mobile.** An
xterm TUI on a phone is close to unusable — no modifier keys, tiny hit targets,
scrolling that fights the page — so the conversation view plus its text box is
the practical way to drive an agent from one. Text goes straight into the pty
via the same `TerminalInput` path the xterm view uses; the TUI cannot tell the
difference. **MCP is not and cannot be involved**: it is agent-pull, so a tool
server can answer a call but can never push a turn into a live conversation.
Multi-line text is sent as a bracketed paste (`ESC[200~ … ESC[201~`) so the TUI
takes it atomically instead of treating the first newline as submit; single-line
text is sent bare, since a TUI that never enabled DECSET 2004 would otherwise
show the wrapper as literal keystrokes. The carriage return is a separate write.
The caveat is that it is sent **blind** — unlike the terminal view you cannot
see whether the agent is mid-turn or sitting in a menu, so the UI says where the
live state is.

**The web can render a terminal pane either way.** A per-pane toggle switches
between the live pty (xterm.js) and the same structured conversation view an
agent pane gets — the captured turns arrive as ordinary pane messages, so
`MessagePane` renders them with no special casing. The two are not equivalent
and the UI says so: the terminal is live and interactive, while the
conversation is a *reading* of the transcript that lags by up to one poll,
shows user/assistant turns plus the questions the agent asks, and sends typed
messages and answers into the same live pty. The terminal stays
mounted-but-hidden behind the conversation view, because unmounting would tear
down the xterm instance and force a re-attach, losing scroll position and focus
on every glance at the transcript.

Each turn is then dressed as the stream message an agent pane would have sent
(`conversation_turn_to_stream_messages`). That is the trick that made this
cheap: no new wire message, no new storage path, no new renderer, and no server
or web change at all. A turn carrying token counts emits a second `Result`
message, because `ws_cli` reads usage only from `extra.usage` on that variant;
an OMP record that carries a real per-turn cost (`usage.cost.total`, from the
Pi session format) has that number used, while providers that report only
tokens keep `total_cost_usd` at 0 rather than inventing a price.

## Terminal panes (`kind: "terminal"`)

New user-created Claude, Codex, OpenCode, OMP, and DeepSeek work uses
`kind: "terminal"`. The structured `kind: "agent"` path is retained only for
historical panes: the CLI runs the provider headlessly and parses stream-json
into structured events. Missing `kind` still deserializes as `agent` so old
`.apas` files remain readable; that compatibility default is not a creation
default, and no `agent:*` profile appears in the supported launch catalogue.

A **terminal pane** instead allocates a pty (`portable-pty`), execs the
provider's *real interactive TUI*, and streams the raw bytes
to xterm.js in the browser. Nothing is parsed, so nothing has to be kept in
sync with a provider's output format — the point is to reuse the CLI as it
ships.

Only `claude`, `codex`, `opencode`, and `omp` can host one
(`terminal_pane::terminal_binary_for`). DeepSeek uses the Claude terminal with
the supported Anthropic-compatible backend environment; Cursor Agent has no
supported launch profile, and Pi is retired (see "Pi is retired; OMP stays").
OpenCode launches with `--auto`; a fresh mobile task uses `--prompt
<instruction>`, while restoration uses `--continue`. Every hostable provider
is launched with its permission-bypass argument.

**OMP pins a pane by directory, not by id.** `omp` has no `--session-id`, so a
pane cannot pin an exact identity by id. It accepts `--session-dir`,
which is stronger: the pane owns a private directory under
`~/.omp/agent/sessions/apas-<conversation id>/`, so a sibling pane or a
provider subprocess sharing a working directory can never select this pane's
session, and `--continue` inside it is unambiguous without consulting recency.
The directory is passed on **every** launch and must precede `--continue`, or
that flag resolves against the shared default store. OMP has approval
prompts, so it is launched with `--auto-approve` like the others.
Verified against omp 18.1.20: a run writes one flat `<ts>_<uuid>.jsonl` there,
`--continue` appends to that same file, and the default store stays untouched.

A new OMP tab from the desktop `+` is labelled "OMP N", and its icon in the tab
bar is titled OMP and coloured violet. Until 2026-11-13 both were Claude's.

APAS does not install Claude, Codex or OpenCode, and does not choose any
provider's model provider or authenticate it. **It installs OMP by itself**
(since 2026-11-15) when an OMP tab is opened, restored or rebooted on a
machine with no `omp` on the daemon's PATH, no verified host-local copy and
nothing at `~/.local/bin/omp`, and only when `omp_path` is the bare name or
`~/.local/bin/omp`; see "Machines report their providers; OMP is installed
when missing". The person still signs OMP in to a model provider in the tab's
Terminal view. The default executables are `opencode` and
`omp`; override nonstandard installations with `apas config set
opencode_path ...` and `apas config set omp_path ...`. Existing explicit
cluster/project allowlists remain opt-in and must add
`terminal:opencode:official:default` or `terminal:omp:official:default`
through the normal policy controls.

On Linux, a newly launched **terminal** pane uses a host-local copy of a
native Claude, Codex, or OMP executable under
`/var/lib/apas/users/<uid>/providers/` instead of mapping the provider binary
from the NFS-mounted home directory. The APAS pane-host executable lives under
`/var/lib/apas/users/<uid>/pane-host/`. These directories are per-host and
persistent, not shared through the user's home directory.
The configured `claude_path`, `codex_path`, and `omp_path` remain the source
installations. Update Claude and Codex through their ordinary launchers
outside APAS; APAS picks up a changed source executable for later pane
launches. An in-pane provider update may instead target its host-local
executable and must not be treated as updating the shared installation.
APAS does not copy script/package-manager launchers whose adjacent dependencies
or update mechanisms require their installation directory.

An administrator must provision this directory **on every host** before the
CLI can use it:

```bash
sudo install -d -o root -g root -m 0755 /var/lib/apas /var/lib/apas/users
sudo install -d -o "$(id -u)" -g "$(id -g)" -m 0700 "/var/lib/apas/users/$(id -u)"
```

The CLI rejects symlinked, misowned, or non-private user storage. Until it is
provisioned, it uses the existing host-local `/var/tmp/apas-providers-<uid>/`
and `/var/tmp/apas-bin-<uid>/` directories, **not NFS**. Once provisioned,
OMP's last verified local version migrates to the new store; the old files
remain available to any running panes. Claude, Codex, and pane hosts copy
fresh from their configured installations at their next launch. Use the UID
rather than a username so a rename does not silently change binary ownership.

For a **native OMP binary**, APAS checks for updates when launching a new OMP
terminal pane and applies them to an isolated **host-local** candidate via
OMP's own `omp update`, never to the shared configured installation. Script
and package-manager launchers still run from their configured paths and must
be updated with their package manager.
Update checks for native OMP are throttled and run in the background, so the
opening pane gets the last verified local executable without waiting on the
network; a successful update is used by later pane launches. Existing panes
continue running their old local image. Failed downloads or checks retain the
last working copy. Do not replace this with `omp update` against
`~/.local/bin/omp`: on NFS, removing an executable mapped by a live pane can
kill it with SIGBUS.

**An explicit OMP agent reboot prepares the update before stopping the agent.**
It bypasses the background-check throttle, waits for an in-flight updater, and
then replaces the provider on the exact verified local executable, preserving
the pane and its conversation directory with `--continue`. It must not silently
re-adopt the old pane host or fall back to an older executable. Preparation
failures leave the current provider running and report the error in the pane's
conversation output. Duplicate requests do not start another preparation;
closing or changing the pane invalidates a pending completion, so a download
cannot resurrect it. A replacement-start failure retains the pane and reports
an exited terminal. Script/package launchers remain package-manager-owned.
This is different from **Reboot CLI**, which deliberately re-adopts hosted
providers without replacing them; ordinary new-pane launches remain nonblocking.

**Two different things share the oh-my-pi name; do not conflate them.**

The npm package `oh-my-pi` is an extension for raw Pi, which APAS no longer
hosts. It has nothing to do with an OMP tab, and installing it does nothing to
one.

`omp` (<https://github.com/can1357/oh-my-pi>) is a **standalone agent and its
own APAS provider**, `Provider::Omp`. It is Pi-derived — same session record
shape, same `PI_CODING_AGENT_*` overrides — but it is a separate binary with
its own store under `~/.omp`, role-based model selection (`--smol`/`--slow`/
`--plan`), subagents, and an ACP mode.

The desktop tab bar, mobile browser pane picker, native mobile task launcher,
server authorization, and CLI local add-tab path all enforce this boundary.
Existing agent panes can still run, receive messages, and reboot; creating or
switching one to another structured-agent profile is rejected. Nothing creates
one any more — managed team roles were the last thing that did, and they are
gone with team mode.

**What terminal panes do not get directly.** Fine-grained tool status and plan
review are built on stream-json events, so a terminal pane has neither. Coarse
working/idle state is reconstructed from user turns and provider-confirmed
completion markers in the transcript. `PaneDiff` is *not* in the unavailable
list despite the pane-kind boundary suggesting it should be: it is computed
from git by `compute_pane_diff`, so it works for any pane with a worktree.

**Conversation history and usage** are recovered by reading the provider's
own transcript — see below.

**Transport.** Raw pty bytes never touch `CliToServer::Output` or
`StreamMessage` — those persist into `messages.jsonl` as chat records, and
ANSI would both bloat the store and break the message renderer. They ride
dedicated `Terminal*` messages, base64-encoded because a pty read splits
both UTF-8 sequences and escape sequences:

- `CliToServer::TerminalOutput` / `TerminalExited` / `TerminalState`
- `ServerToWeb::TerminalOutput` / `TerminalSnapshot` / `TerminalExited` /
  `TerminalState`
- `WebToServer::TerminalInput` / `TerminalResize` / `TerminalAttach`
- `ServerToCli::TerminalInput` / `TerminalResize`

The server keeps a bounded in-memory state entry per `(session, pane)` with
scrollback bytes, the newest sequence, truncation, PTY instance UUID,
lifecycle (`unknown`, `running`, `disconnected`, or `exited`), and optional
exit status. `TerminalAttach` is answered from that entry, including when it
has lifecycle but no bytes, so reattach paints immediately and a process that
exited before producing output still gets an accurate banner.

**PTY lifetime is not WebSocket lifetime.** A transport-only CLI disconnect
changes confirmed-running terminal entries to `disconnected` but retains their
bounded presentation. When the same APAS process reconnects, it reports every
configured terminal before draining queued output. Each spawned PTY has a UUID:
a running report for the same UUID restores `running` without clearing bytes,
while a different UUID replaces the entry and starts with fresh presentation.
Output and exit events from an older UUID are ignored. An already-`exited`
entry and its status stay exited across later transport cleanup. Explicit pane
removal still deletes the entry.

Retention is deliberately non-durable. Terminal bytes and lifecycle remain in
the session manager only, obey `TERMINAL_SCROLLBACK_MAX_BYTES`, and are never
written to SQLite or `messages.jsonl`; a server restart loses them and state is
`unknown` until the CLI reconciles again.

On the web side, frames bypass zustand entirely (`lib/terminalBus.ts`): a
full-screen TUI repaints many times a second, and storing chunks in state
would re-render every subscriber per frame. `TerminalPane` tracks the current
PTY UUID and last rendered sequence locally. Same-instance snapshots at an
already-rendered sequence update lifecycle without duplicating output;
cumulative snapshots that cover missed frames and replacement UUIDs reset
xterm before replay. Snapshot/live lifecycle is authoritative for the
disconnected, unknown, and exited banners, including empty snapshots.

Claude's fullscreen TUI captures the mouse and owns text selection, then sends
the selected text to its terminal through OSC 52. The web terminal loads
`@xterm/addon-clipboard` with a write-only browser provider so that copy reaches
the user's clipboard without allowing a provider process to read it. Clipboard
reads receive an empty reply. The terminal also leaves paste shortcuts to the
browser and leaves copy shortcuts there when xterm owns a selection; Ctrl+C
without a selection still reaches the provider as an interrupt.

**Rolling deployment.** Deploy the server first, then web, then CLI. New fields
are optional/defaulted, so a new server accepts legacy output and exit frames,
and a new web treats a legacy snapshot as `unknown`. Metadata-less output is
still rendered but never proves that a retained process is running across a
reconnect. During rollback, an older server may ignore the new `TerminalState`
variant while continuing to relay the backward-compatible output/exit frames;
continuity then degrades to unknown/disconnected rather than a false confirmed
running state.

Native mobile launch advertises `mobile_task_launch_v2`. The version bump is
intentional: v2 creates a terminal pane and passes the first instruction as a
provider-native CLI prompt; a v1 CLI would otherwise accept the pane and drop
that instruction. OpenCode additionally requires `terminal_opencode_v1` and
OMP `terminal_omp_v1`, so a rolling server/web deployment refuses to route
either of them to an older v2 CLI that can launch Claude/Codex but not that
provider. The server asks the user to update/reconnect instead of pretending
an older launch succeeded. `terminal_pi_v1` was deleted with Pi: an older CLI
may still advertise it, and the server routes nothing on it.

Roll out OpenCode and OMP support in this order: server first, web second,
then upgrade and reconnect project CLIs so they advertise
`terminal_opencode_v1` and `terminal_omp_v1`. Install and authenticate the
provider and opt the intended policies into its profile only after the host
CLI has reconnected.

### Pi is retired; OMP stays

Raw Pi (`pi`) was a hostable provider until 2026-11-13. OMP is the Pi-derived
agent APAS hosts, and nothing about OMP changed. Pi joined the pattern MiniMax
and GLM already use: a decode-only value that is never offered, never launched
and never silently replaced by another provider.

- **The wire value still parses.** `Provider::Pi` and the web value `"pi"`
  remain, deprecated, so an older `.apas`, a stored roster, a stored policy and
  a frame from an older client load. The JSON schema keeps `pi`.
- **Nobody can create one.** Pi is out of `shared::all_tab_types`, the
  supported launch profiles, the desktop `+` menu, the phone's pane sheet and
  the provider switcher. A request to create a Pi tab, switch a tab to Pi or
  relaunch a saved one is answered by server, CLI and web with "Pi is no longer
  supported. Use OMP instead." (`shared::PI_RETIRED_MESSAGE`). A native launch
  that still sends the Pi profile key gets the same sentence.
- **A saved Pi tab stays stopped.** It stays in `.apas` and in the roster until
  someone closes it. No path starts it: `spawn_terminal_pane` refuses before it
  looks for a binary. At every registration its CLI reports it as an exited
  terminal whose status is the Pi sentence. Desktop and phone show "Pi is no
  longer supported. Use OMP instead." above its history, which is read-only:
  nothing can be typed into the tab, and it cannot be resumed or rebooted. It
  can still be closed.
- **Input to it is refused by the server too.** Typed input gets "Pi is no
  longer supported. Use OMP instead. This tab is read-only.", raw keystrokes
  are dropped without an error per key, and a provider switch gets "Tab N uses
  Pi, which is no longer supported. Close it and open an OMP tab." A refused
  switch does not edit the roster. The same input rule now covers old MiniMax
  and GLM tabs ("This pane uses a retired provider and is read-only."); the
  server used to pass their input to the CLI.
- **A Pi agent that was still running is ended by the update.** A retired tab
  is left out of the pane hosts a starting project keeps, so reconciliation
  removes its host instead of leaving the agent alive and unreachable until its
  lease ran out.
- **It blocks the project's Reboot CLI until it is closed**, like any retired
  pane: "Tab N uses Pi, which is no longer supported. Close that tab, then
  reboot the CLI." The Machines page reboot is not blocked.
- **`pi_path` is no longer a setting.** A config file that still has the key
  loads, and the key is dropped at the next save. `apas config set pi_path`
  and `get` answer "pi_path is no longer a setting. Pi is no longer supported.
  Use OMP instead (omp_path)."

The line "[Pi is no longer supported. Use OMP instead. This tab stays
stopped.]" goes to a local `apas --attach` view only. It is never sent to the
server, so do not look for it in a tab's conversation: the web learns the
reason from the exited terminal state.

What stayed, because OMP uses it: `PiTranscriptReader` and the Pi
session-format parser with their tests, the `PI_CODING_AGENT_*` overrides, and
`omp_activity.mjs`. In the Overview, a leftover Pi tab's row in the Usage
table is named "Pi" by `providerDisplayName` in `lib/launchProfileLabels.ts`,
since the provider catalog no longer has a Pi group. Its tab card reads
"Unsupported provider", and it adds no Resource use card.

Most of this section has been seen in unit tests only: no tab of this
deployment uses Pi. See "What was checked for these four" near the top, and
the change's record, `openspec/changes/archive/2026-11-13-refine-interactive-workspace/`.

**Deploying it** follows the usual order: server, then web, then CLI. On its
first start the new server drops `terminal:pi:official:default` from stored
policies and keeps the rest; a changed row gets a new version, an override
that listed only Pi becomes an empty allowlist, and writing the key back is
rejected as a retired profile. An older web that still offers Pi is refused by
the server with the Pi sentence. To roll back, redeploy the older server and
web. The Pi profile is not restored to stored policies automatically. The
work assumed that no tab of this deployment uses Pi. Nobody checked production
for one.

### Persistent pane hosts and CLI lifecycle

On supported Unix project hosts, each new Claude, Codex, OpenCode, or OMP
terminal pane is owned by a hidden `apas pane-host` process in its own
project-scoped tmux session. The replaceable project CLI is only its
authenticated controller.
This removes the CLI process from the provider's lifetime: a transport-only
`Reconnect Server` leaves the CLI, pane hosts, PTYs, queues, and structured
turns untouched, while `Reboot CLI` prepares the update first and then adopts
the same hosted terminal processes after `exec`.

**`Reboot CLI` replaces the whole instance, because the CLI *is* the daemon.**
Projects run inside one process, so there is no smaller unit whose replacement
changes the running version. The project-scoped button and the Machines page's
`Reboot to update` therefore perform the same act, and one project's button
restarts every project on the host — the resume manifest brings them all back,
and pane hosts are separate processes `exec` never touches, so terminal agents
are adopted rather than restarted.

**That adoption depends on one guard: project teardown must skip terminal
cleanup when `reboot_requested` is set.** `detach_for_reboot` only asks a host
to release its controller and deliberately leaves the handle live, so a
`shutdown()` afterwards still runs `terminate_tmux_host` — which kills the tmux
session *and* deletes the runtime directory. Without the guard a reboot
destroys the very hosts it is about to adopt, then finds no descriptor, starts
fresh providers, and still reports the panes as live-adopted, losing whatever
turn was in flight. The guard looks redundant because it once was: a reboot
used to `exec` inside the project's own process and never reach teardown. It is
load-bearing now that projects return to the daemon and the daemon execs
afterwards. For a while it did not: a project's request
restarted only that project's task, so `prepare_cli_restart` installed the new
binary and the old code kept serving. The update looked prepared and never
applied, and the machine stayed on its old version until someone used the
Machines page. The project that asks is also special-cased into the resume
manifest, since it ends its own task to make the request and finished tasks are
otherwise excluded — the one project the user acted on was exactly the one that
would not have come back.

The feature is advertised only when Unix sockets, tmux, the installed
`apas pane-host` subcommand, and secure runtime storage all validate. Otherwise
terminal panes keep using the direct PTY implementation and the lifecycle menu
warns that they must restart/resume. Existing direct PTYs are not migrated in
place; after their first restart under a capable CLI they become host-backed.
Structured `kind: "agent"` panes retain their existing restart/resume behavior
and are never described as live-adopted.

Pane-host state is host-local, volatile, and outside the project directory:

- Root: `${XDG_RUNTIME_DIR}/apas/ph`, or `/tmp/apas-<uid>/apas/ph` when
  `XDG_RUNTIME_DIR` is unavailable. **The `/tmp` fallback loses terminals.**
  The system's cleaner deletes files under `/tmp` after 10 days
  (`q /tmp 1777 root root 10d`), so a host older than that has lost its
  `runtime.json` and `credential` and keeps only its socket. Nothing notices
  while the daemon that launched it keeps its connection. At the next daemon
  reboot the host cannot be found, the tab gets a fresh provider that resumes
  its conversation, and the old provider runs on, detached, until its lease
  ends. zoo-006's daemon was started without `XDG_RUNTIME_DIR` and lost two
  tabs this way on 2026-11-13. Check a daemon with
  `tr '\0' '\n' < /proc/<pid>/environ | grep XDG_RUNTIME_DIR`, and start
  daemons from a session that sets it until the fallback is moved off `/tmp`.
- Logs: `${XDG_RUNTIME_DIR}/apas/pane-host-logs/<project>/<pane>-<runtime>.log`,
  0600, info level, rotated once at 1 MiB and pruned after 14 days. A host's
  stderr is the tmux pane it runs in, which vanishes with the process, and its
  runtime directory is removed when it exits cleanly — so before this a host
  that died left no trace anywhere. The log records start (pid, identity,
  executable), adoption and detach, lease expiry, the provider's exit status,
  panics, and **the fatal signal that killed it** (a handler writes one line
  naming the signal, then restores the default disposition and re-raises).
  The `TerminalState` the CLI reports for a dead host names this file.
- Executable: a host runs from a content-addressed copy under
  `/var/lib/apas/users/<uid>/pane-host/` when provisioned, or the former
  `/var/tmp/apas-bin-<uid>/` on hosts awaiting setup, never directly from
  `~/.local/bin/apas` when a local copy succeeds. Home is NFS and
  every install replaces that file. A process on a local filesystem keeps its
  unlinked executable alive; a process whose executable was unlinked on the
  NFS server has nothing to fault code pages back in from and dies with
  SIGBUS the first time memory pressure evicts one. A host sits idle for days
  and is the process most exposed to that — the Codex pane in `apas` died
  this way on 2026-09-02, eight minutes after a deploy, with no log anywhere.
  Copies are shared by content, pruned a week after being superseded, and
  verified to run before use; any failure falls back to the shared path.
- Project/runtime directories are `0700`; `runtime.json`, `credential`, the
  Unix socket, and reboot `handoff.json` are `0600`.
- `runtime.json` contains identity, protocol, tmux session, and socket paths,
  but no credential or terminal content. The random 256-bit credential is in a
  separate owner-only file and is never sent to providers or the server.
- Raw detached output remains only in the pane-host's bounded in-memory ring;
  it is not written to `.apas`, SQLite, JSONL, or a spool file.

**A host that is gone is reported as exited, not disconnected.** When the
controller's socket to a host closes without an `Exited` frame, the CLI asks
tmux whether the host's session still exists. If it does, the pane is
`disconnected` (the host is alive and re-adoptable, and the web keeps its
retained view). If it does not, the host process itself died, and the CLI
reports `exited` with a status naming the tmux session and the log file, so
the web offers a reboot instead of implying a transport outage that will heal.
Before this, a dead host looked exactly like a CLI reconnect in progress, and
stayed that way forever.

**A launch whose handshake fails tears its host down.** The host spawns the
provider *before* answering `Create` with `Adopted`, and the controller used
to give it 5s and then fall back to a direct PTY — leaving the tmux session it
had already started to go on and spawn its own provider. That put two agents
on one conversation. The handshake now waits 30s, and on any failure after the
tmux session exists the controller kills that session and removes its runtime
directory before falling back.

Unexpected controller loss keeps the provider alive for 600 seconds by
default; an authenticated reboot handoff gets 900 seconds. Configure the
bounded values with:

```bash
apas config set pane_host_adoption_grace_seconds 600  # allowed: 30..3600
apas config set pane_host_reboot_grace_seconds 900    # allowed: 60..7200
```

Pane close, provider switch/reboot, project stop/suspension, and project
deletion bypass grace: they tombstone the project, authenticate shutdown where
possible, terminate the provider process group, kill the exact pane-host tmux
session, and remove local runtime files. An unexpected orphan self-terminates
when its lease expires.

Operational inspection must not print `credential` contents. Safe checks are:

```bash
apas config path
find "${XDG_RUNTIME_DIR:-/tmp/apas-$(id -u)}/apas/ph" -name runtime.json -type f -print
tmux -L "apas-<full-project-uuid>" list-sessions
ls "${XDG_RUNTIME_DIR:-/tmp/apas-$(id -u)}/apas/pane-host-logs/<project>/"
journalctl --user --since '15 minutes ago' | grep -E 'pane-host|lifecycle|tmux-spawn'
```

The `tmux-spawn-<uuid>.scope` records in the user journal are systemd's view
of each tmux pane: `Started … launched by process <pid>` when a host starts,
and a `Consumed … CPU time, … memory peak` line the second every process in
it is gone. That second line, with no `Stopping`/`Stopped` before it, is how a
host that died on its own was dated when it had left no log.

Use the web Machines/Admin project stop action for cleanup; the daemon can
enumerate and terminate pane hosts even when the project CLI is absent. If
manual recovery is unavoidable, first identify the exact session from its
owner-only `runtime.json`, then use `tmux -L <socket> kill-session -t <session>`;
never recursively delete a runtime root while a listed host is still alive.

Roll out in this order: server/shared protocol, web lifecycle menu, then CLI
and daemon. Old CLIs keep the legacy reboot control and never receive a
reconnect disguised as reboot. Rollback disables new host creation; already
running compatible hosts should be allowed to close normally or stopped by a
new daemon. Forcing an old CLI/daemon to clean them up interrupts active
terminal turns.

## Loop prompt on Codex, Claude and OpenCode tabs

Codex, Claude (DeepSeek included) and OpenCode terminal tabs have no
autopilot: APAS cannot run a loop inside them or watch their turns the way it
does in OMP. In the toolbar spot where OMP tabs show Autopilot, and as "Send a
loop prompt" in the phone's Pane actions, they offer **Loop prompt**: a
shortcut that sends **one** message asking the agent to keep working in a loop
by itself. The owner chose this on 2026-11-14 over a full Codex autopilot
driven through Codex's control socket, which was researched and checked and
then withdrawn (findings in `~/.cache/apas-codex-check/NOTES.md`; the plan is
in git history as `openspec/changes/add-codex-tab-autopilot`). Design record:
`openspec/changes/add-loop-prompt-for-terminal-tabs/`.

- **The dialog says what it is before anything is sent:** APAS does not run or
  watch the loop and cannot count rounds, enforce a limit, pause, resume or
  stop it, or tell whether it is still looping; the agent keeps the tab's
  permissions and spends that machine's login, whoever sent it; it is stopped
  with Esc in the terminal.
- **What is sent** (`lib/loopPrompt.ts`): a fixed loop instruction, shown in
  the dialog, then "Round instructions:" and the prompt. The instruction tells
  the agent to repeat the instructions round after round without waiting or
  asking, to read an instruction to stop after one step as the end of a round,
  to read the files again each round, and to stop only when a round ends with
  "Nothing is left to do." or it is interrupted. "Keep the tests green" has no
  such ending and loops until interrupted; the dialog says so.
- **Prompts:** the same templates and the project's saved prompts as the
  autopilot dialog, starting on the project's default (shared code in
  `components/tabs/PromptTemplateChoice.tsx`). Saving prompts is done in the
  autopilot dialog.
- **Sending:** once per click, through the Conversation view's terminal input,
  confirmed only when the provider records it ("Sent."), otherwise "Not
  confirmed", and never resent. It creates no autopilot state, marker, status
  or list entry. It needs no server or CLI capability.
- **Rolled out on 2026-11-14** with web-only release `20261114T174439Z`
  (version 26.11.48, built at d4ba3db), live since 17:45 UTC; backups under
  `/opt/apas/backups/deploy-20261114T174439Z`. The same release removed the
  Workers and Attention buttons from the project list. `master` was pushed
  at the same time with the Codex locator fix and the OMP summarizer, which
  each machine gains at its next daemon reboot. Ten public routes and 18
  static assets answered `200`, and the web logged no error. Not yet seen: a
  loop prompt sent to a real tab.

## Autopilot on OMP terminal tabs

**Autopilot** lets one OMP terminal tab keep working by itself: APAS submits
the same prompt to that tab again and again, until a limit is reached, someone
stops it, or something goes wrong. It is the light, in-tab way to leave an
agent running. It is not an autonomous worker: there is no container, no
independent verification, no budget and no draft PR, and the agent keeps the
tab's ordinary permissions on its machine. The design record is
`openspec/changes/archive/2026-11-13-add-omp-tab-autopilot/`; the doc comments in
`crates/shared/src/autopilot.rs` and `crates/client-cli/src/omp_autopilot.rs`
are the binding statement of the wire and of the files between the CLI and
the tab.

**The loop runs inside the tab's OMP process**, in APAS's embedded extension
(`omp_activity.mjs`), not in the CLI and not on the server. Each iteration is
`pi.sendUserMessage(prompt, { attribution: "agent", deliverAs: "aside" })`, so
it is a real turn of the tab's own conversation and survives a browser being
closed, a daemon reboot and a server restart. Nothing is typed into the
terminal. The CLI passes requests to the tab and relays its state; the server
authorizes, routes and caches.

### Starting one

The pane toolbar of an OMP terminal tab has an **Autopilot** control before
the Terminal/Conversation switch. On a phone it is **Start autopilot** in the
Pane actions sheet. The dialog asks for:

- **The prompt**, at most 16,000 bytes. It is submitted as text: slash
  commands and skills in it are not run. OMP still applies its own keywords,
  `@path` mentions and a `+500k`-style token budget to it.
- **When to repeat**: as soon as each turn ends (five seconds after the tab
  settles), or every N minutes (one minute to one day). An interval is
  measured from one submission to the next and never interrupts a turn.
- **Limits, both optional**: a number of iterations and a length of time.
  **"No limit" is the default for both**, by the owner's decision. There is no
  token or spend limit.
- **Optionally, "end when an iteration makes no tool call"**.

**The dialog has a Template control above the prompt** with four built-in
prompts and **My own text**:

- **APAS worker**, the default since 2026-11-14. It keeps three layers in
  line, one step per iteration. The **whitepaper** (`docs/whitepaper.md`, or
  the files under `docs/whitepaper/` or `docs/book/`, or else the document the
  README or `AGENTS.md` names as the whitepaper, book or main design) belongs
  to a person: the agent reads it and never changes it. What it would change
  there, its questions and the assumptions it made go to
  `docs/apas-worker/report.md`, one entry each with a status line; the person
  answers with an `Answer:` line under an entry, and the next iteration acts
  on it first. A question that blocks all remaining work is also asked with
  OMP's `ask` tool, which holds the turn, and so the autopilot, until someone
  answers it in the Conversation view or the terminal; such an answer is
  written into the report too, and a dismissed question is not taken as an
  answer. An answer never lets the agent edit the whitepaper: an accepted
  change waits until the person makes it. The **OpenSpec specs** are the
  agent's to change as long as no requirement contradicts the whitepaper,
  through OpenSpec changes checked with `openspec validate --strict`; details
  the whitepaper leaves open are the agent's to decide. The
  **implementation** is the agent's to change as long as it does what the
  specs require. Nothing is removed only because the layer above does not
  mention it. It never does a task outside the checkout (deploying, servers,
  accounts) and installs nothing. Its notes between iterations are
  `docs/apas-worker/progress.md`. **For Rust code it proves instead of
  tests** (owner, 2026-11-15): where Verus can check a property, it writes a
  Verus specification and proof taken from the OpenSpec requirement, and keeps
  tests only for files, network, terminal, timing, other crates and code no
  proof models. A proof counts only when the project's verification command
  verifies it, with nothing Verus trusts unchecked (`assume`, `admit`,
  `assume_specification`, `external`/`external_body`, `assume_new`, axioms,
  code moved out of `verus!`). An existing test may be deleted only in the
  commit that adds a verified proof covering it, written as a kept function
  in `verus!` with the test's inputs and asserted results; each judged test is
  recorded in `docs/apas-worker/verus.md` so it is not retried forever. It
  never installs Verus: without one on the machine it asks in the report
  whether to make it available or keep tests, and sets Verus up (pinned, with
  the command written into the README or `AGENTS.md` and CI) only with the
  Verus already there. Live since web release `20261115T194926Z` (version
  26.11.53, built at 9f57514, 19:50 UTC on 2026-11-15); backups under
  `/opt/apas/backups/deploy-20261115T194926Z`. The whitepaper rule is in the prompt only,
  with a `git status` check before each commit that undoes only the agent's
  own edit; a project that wants it enforced can add a commit hook.
  **It syncs its branch with the git remote** (owner, 2026-11-15); the other
  three templates still never push. Only where a whitepaper exists, each turn
  fetches, merges the upstream with `git merge --no-edit` (never a rebase or
  a pull), and pushes with a plain `git push` after every merge and commit:
  its own branch only, never by force, no tag, no pull request. Git runs with
  `GIT_TERMINAL_PROMPT=0` and `GIT_EDITOR=true`. What it does when syncing
  cannot go on:
  - **No upstream, a detached HEAD, no network or credential, a refusal the
    rules do not name, or other people's uncommitted changes in the way:** it
    skips the sync that turn, works locally and says so in its reply.
  - **A merge, rebase or cherry-pick already under way:** it may be a
    person's, so it changes nothing and ends the run.
  - **A conflict only in its notes** (`docs/apas-worker`): both sides'
    entries are kept and the turn goes on, so two workers do not spend every
    other turn merging notes.
  - **A whitepaper conflict, a conflict it cannot resolve or whose checks it
    cannot fix, or a forced update of the upstream:** it stops syncing,
    aborts its merge, writes one report entry, asks the person, and neither
    merges nor pushes until the person answers or the upstream contains the
    branch. It never settles a whitepaper conflict itself.
  - **A push rejected because the upstream moved:** left for the next turn,
    and that reply may not end the run.
  Known limits: two workers on one branch converge but do each step twice, so
  run one worker per branch; a plain push sends every commit on that branch
  in that checkout, also ones other tabs made there, and goes straight to a
  branch that normally takes pull requests if the login may; a forced update
  that something else in the checkout fetched first is not seen. A tab that
  already ran the worker opens with its old prompt, and a saved project
  prompt is a copy: neither gains these rules until the template is chosen
  again. The wording went through two rounds of independent review, with the
  git behaviour tried in throwaway repositories; it has not run on a real
  tab. Live since web-only release `20261116T031901Z` (version 26.11.62,
  built at cbc5f0e, 03:21 UTC on 2026-11-16); backups under
  `/opt/apas/backups/deploy-20261116T031901Z`. Ten public routes and 18
  static assets answered `200`, the served bundle carries the new text, and
  the web logged no error. Gates: web 1,441 tests, typecheck and lint.
  **It changes a graphical interface only where it can look at the result**
  (owner, 2026-11-16). A step that changes what a person sees in a web page
  or an app needs the agent to look at the result in that turn: with a
  browser or computer-use tool, or a screenshot it takes and can read as an
  image. Reading the markup or passing tests does not count, and it installs
  nothing for it. Where it can look, it looks at every state the step
  changed before the commit. Where it cannot, or cannot run the interface
  there, it leaves that part of the design alone, specs included, writes one
  report entry naming the machine and the parts that wait, and takes another
  step; a machine that can look takes them up. Whether an agent says
  honestly that it cannot see an image is untested.
  Live since web-only release `20261116T043732Z` (version 26.11.64, built
  at 2922967, 04:38 UTC on 2026-11-16); backups under
  `/opt/apas/backups/deploy-20261116T043732Z`. The server was not restarted
  and both dependency graphs audited clean on the host. Ten public routes
  and 13 static assets answered `200`, the served bundle carries the new
  text, and the web logged no error. Gates: web 1,442 tests, typecheck and
  lint.
- **Implement from a design document**. Each iteration closes one
  gap between the design and the code, with tests and a commit. It uses
  `docs/design.md`, or else only the one document the README or `AGENTS.md`
  names as the project's current design, architecture or specification. With
  neither it changes nothing. It keeps its notes in
  `docs/autopilot-progress.md` and commits them.
- **Work through TODO.md**. Each iteration does the next open task in
  `TODO.md` that is not marked blocked.
- **Keep the tests green**. Each iteration runs the project's checks and fixes
  the first failure at its cause.

A tab with no earlier autopilot opens with the default template in the prompt
field. A tab that had one opens with that autopilot's prompt, as before.

**A template is only starting text.** What is sent is whatever the prompt
field holds, and the control shows a template only while the field holds that
template's exact text. Choosing one replaces the prompt and nothing else: it
never changes the timing, the limits or the no-tool-call option, and a line
under the control says how that template's run ends with the option as it
stands. Text a person wrote is kept when a template is chosen, and My own text
brings it back in the same dialog; editing the template's text replaces the
kept text, and the dialog says so.

With the Template control the dialog is taller than a laptop window. Its body
scrolls, and Cancel and Start autopilot stay in view below it.

The templates live in `packages/web/src/lib/autopilotTemplates.ts`, whose
header says how such a prompt has to be written. The same text is submitted
every iteration, so each describes one step. Each forbids rewriting history
(stash, reset, clean, amend, rebase, skipping hooks) and touching changes the
agent did not make, and each but the APAS worker forbids pushing. The first three also say where their notes are
(`docs/apas-worker/progress.md`, `docs/autopilot-progress.md`, `TODO.md`)
and how to mark a step that cannot be done, so the next iteration does not
take it again. Keep the tests green
keeps no notes and marks nothing: it relies on the conversation alone to skip
a failure it already tried twice.
A unit test puts every template through the start's own validation.

**The first three templates end a run in two turns, and only if the
no-tool-call option is ticked.** Finding out that nothing is left takes tool
calls. So one turn finds nothing left and ends its reply with "Nothing is left
to do."; the next sees that sentence, calls no tool and replies "Nothing has
changed since I last looked." With the option ticked, that turn ends the
autopilot. With it off, the turn after that looks again, so a timed repeat or
a later run rereads the files and never answers "nothing left" for ever. Keep
the tests green has no end of its own: set a limit or stop it.

**A project can keep its own prompts** in its `.apas` (since 2026-11-14,
change `add-autopilot-prompt-library`). In the start dialog, **Save to
project…** stores the prompt field's text under a name; with a saved prompt
selected, **Make default** / **Clear default** and **Delete** (with a
confirmation) change it. The Template control lists them under "This project",
the default marked "(default)", before the built-ins. A tab's starting text is
its last ended autopilot's prompt, else the project's default, else APAS
worker. None of these actions sends anything to a tab or touches a running
autopilot, and each is answered once (saved, refused with the reason, or not
confirmed) and never resent. Anyone who may start autopilot on the project's
tabs may change them.

- **Storage.** Two optional top-level `.apas` fields: `autopilot_prompts`
  (`[{name, prompt, updated_at}]`) and `default_autopilot_prompt`. Only the
  CLI writes them, through the locked merge, so other machines' panes and
  unknown fields are kept; deleting the last prompt removes both fields. A
  value that does not parse is shown as "This project's saved prompts cannot
  be read.", every change is refused, and the CLI never rewrites it: fix the
  file by hand.
- **Rules** (`crates/shared/src/autopilot_prompts.rs`, checked by the server
  and again by the CLI): a name is 1 to 80 characters after trimming, unique
  in the project ignoring case, and not a built-in template's name; a prompt
  meets a start's bounds; at most 50 per project. A shared test fails if the
  web's built-in template names change without
  `AUTOPILOT_BUILTIN_TEMPLATE_NAMES`.
- **Several machines.** A change goes to the primary instance first; only
  after it accepts does the server send it to the other capable instances, so
  on a shared home the second write cannot turn a save into a refusal. With
  separate checkouts each keeps its own list, and viewers see the primary's.
  A machine not running the project gets a change only with a later save.
  `.apas` is usually git-ignored, so a fresh clone has no saved prompts.
- **Capability** `project_autopilot_prompts_v1`, negotiated by CLIs (echoed in
  `Registered`) and viewers (in `Authenticated`); older viewers get no new
  frame. If the primary instance's CLI is older, a change is refused with
  "<host> runs an older APAS that cannot save prompts. Reboot <host> to update
  it." Deploy server, web, then CLI; each machine gains it at its daemon
  reboot.
- **Rolled out on 2026-11-14.** Server and web release `20261114T044521Z`
  (version 26.11.43, built at ae14eb3) since 04:46 UTC, backups under
  `/opt/apas/backups/deploy-20261114T044521Z`; `master` pushed for the CLI.
  All project instances and three daemons registered again, the server logged
  no error and no frame it could not parse, ten public routes and 18 static
  assets answered `200`, and viewers now negotiate
  `project_autopilot_prompts_v1`. Gates at ae14eb3: shared 104 + 4 + 1,
  server 463, CLI 610 (1 ignored), static musl build, protocol 77 and the
  extension's 137, web 1,285 with lint, typecheck and build, native 136,
  `openspec validate --all --strict`. Not yet seen: a real save and its
  receipt (the browser check used a stand-in list and forwards no request),
  and any machine saving before its daemon is rebooted onto this CLI.

**Anyone who may type into the tab may start, pause, resume and stop its
autopilot** (`check_session_runtime_access`, the check input uses). The agent
then works under that machine's OMP login and spends that login's quota,
whoever started it. One tab has at most one autopilot.

### What pauses or ends it

With no limit set, these are all that stop an autopilot:

- **Stop** ends it and lets a turn in progress finish. **Stop now** also
  aborts that turn; background jobs the agent started keep running. **Pause**
  keeps it for a later **Resume**.
- **Three failed iterations in a row** pause it. After each failure it waits
  60 seconds first. OMP's own retries happen inside one iteration.
- **A person's interrupt pauses it**: Esc during a turn, during a retry wait
  or during an automatic compaction, and a wait for background work that was
  ended in the terminal.
- **Leaving the conversation ends it**: `/new`, `/resume`, `/fork`, `/branch`
  and a promoted `/btw` answer.
- **OMP's own goal, loop or plan mode, or autoresearch**, switched on in the
  terminal, pauses it, and it cannot start or resume while one of them is on
  or paused. APAS shows goal and loop mode but never switches them.
- **A restart of the tab's agent** brings it back paused, never running. If
  the agent process exits by itself, the CLI reports "Paused: the agent
  process exited"; reboot the agent first.
- **The limits**, where set. The time limit lets a turn in progress finish.

**A tab that APAS cannot control stops submitting.** The CLI writes a
heartbeat for each tab every three seconds. The tab holds, without ending,
when the heartbeat is older than 60 seconds, when the CLI has not reached the
server for more than ten minutes, or when the server the CLI last registered
with cannot control autopilot (an older server). That last hold stays when
such a server goes away, until a server that can control the tab has
registered. A held tab continues when control returns, and becomes "Paused:
APAS could not control this tab for more than four hours" otherwise. A CLI
that has not registered since it started says so, and the tab holds: a daemon
restarted in the middle of an outage must not give a tab ten more minutes.

A provider usage limit is a failed iteration like any other, unless OMP's
`retry.waitForUsageReset` setting is on: then OMP waits inside the iteration,
but only for a reset time the provider states, and OMP's own description
advises leaving it off for unattended runs.

### What you see

- The control and a status line show the tab's reported condition: starting,
  running an iteration, waiting (for the next iteration, for the tab to
  settle, for background work), holding, paused with its reason, or ended
  with its reason. An ended reason clears by itself after a later turn.
- Each iteration appears in the Conversation view as one marker line, with
  its number and the prompt on the first one, never as a message a person
  sent.
- A tab on autopilot is marked on its tab, on its pane dot (a ring, and
  ", on autopilot" in the label), on the Overview card and on the phone's tab
  chip. The idle lists gain an **On autopilot** group; paused and ended tabs
  stay among the idle ones with their reason.
- A tab keeps reading **Working** from ten seconds before an iteration is due
  until 15 seconds after it was submitted, so a tab that repeats at once does
  not flicker Idle between turns.
- No push notification is sent for any autopilot event.

A request is answered once: accepted, refused with the reason, or not
confirmed. **A request is never sent again automatically**, also not after a
reconnect: a replayed start could start a second run. A tab whose state is not
reported offers Stop and Stop now only.

### How it is built

- **Tab ↔ CLI** is a handful of private files in the pane's runtime directory
  (`omp_autopilot.rs` lists them): the CLI's heartbeat, a start-or-resume
  request the tab takes by renaming it, a pause-or-stop request the tab never
  deletes, the tab's acknowledgements, its state report, and a lock. Every
  file carries the launch id; reports also carry OMP's pid, and the CLI reads
  them only while that provider is running. A report may carry a
  `process_start`; this CLI does not read it. A start that the tab has not taken within two
  seconds is withdrawn and answered "not taken", never left to fire later.
- **The journal** is `apas-autopilot` custom entries in the OMP session, so an
  autopilot belongs to its conversation. The CLI's transcript reader pairs
  each journaled iteration with the agent-attributed message after it.
- **The server keeps nothing durable.** It caches each tab's last report in
  memory, drops it when the reporting instance detaches, and the CLI reports
  every OMP tab again after each registration. The CLI sends no state to a
  server that did not echo the capability: such a server cannot parse the
  frame and would log every one.
- **State reaches a viewer a few seconds late.** The CLI relays it from the
  three-second pane watcher. The first frame after a reconnect can be older
  still, by about ten seconds in the smoke runs, and the next poll corrects
  it.
- **The state report contains the prompt.** It is sent only to viewers who
  may read the session; session lists and the mobile bootstrap carry a
  summary without it.

### Capabilities, rollout and rollback

Three independent markers gate it. The CLI advertises `omp_tab_autopilot_v1`
and the server echoes it in `Registered`; viewers negotiate
`tab_autopilot_v1`; each provider launch records `"features":
["autopilot_v1"]` in `omp-launch.json`. The minimum OMP is **18.18.2**; an
older one reports itself unsupported.

Deploy **server → web → CLI**, as usual. Then:

- A machine gains autopilot when its daemon is rebooted to update from the
  Machines page, which restarts every project on that host.
- **A tab gains it at its next agent launch.** An agent that was running
  before its machine was updated keeps the old extension, and its control
  reads "Reboot this tab's agent to use autopilot. Rebooting ends its turn in
  progress." Both reboots stay the owner's action.
- Older viewers are sent no autopilot frame; older native builds would drop
  their socket on one. Native has no autopilot controls.

To roll back, **stop running autopilots first**. A server rolled back alone
makes every tab hold and then pause; a web rolled back alone leaves running
autopilots with no control to stop them except the terminal (Esc pauses one).

**Repeat the real-OMP check when OMP's minor version changes.** The loop
depends on OMP's event order, which is not a documented contract. The check
harness is `~/.cache/apas-autopilot-check` on zoo-005 (`NOTES.md` there lists
what OMP 18.20.0 was seen to do).

### Verification

**Verified on 2026-11-13.** At the change's last code commit the Rust
workspace (shared 94, server 453 and CLI 592 tests), the static musl CLI
build, the protocol (67 tests, typecheck, and the extension's 137 node tests),
web (1,146 tests, lint and typecheck) and native (136 tests, lint and
typecheck, unchanged) pass, as does `openspec validate --strict`.

**The built extension ran on OMP 18.20.0** with DeepSeek V4 Flash, in an
isolated agent directory on zoo-005 with a stand-in for the CLI. It covered:

- both timings, the iteration limit, the time limit (during a turn, while
  paused, and ending before the next iteration) and no limit;
- the no-tool-call ending and the pause after three failed iterations;
- Esc during a turn, a retry wait and an automatic compaction; Esc on a wait
  for background work, and a killed background job;
- pause, resume, Stop and Stop now;
- an agent killed and restarted, which came back paused;
- `/new`, a promoted `/btw` answer and a return with `/resume`;
- the three holds, a hold between two timed iterations, and a hold kept
  after its server went away;
- the goal, loop and plan mode rules.

A session file from that run is the CLI transcript reader's test fixture.

**A two-host smoke** ran on zoo-005 and zoo-006 with isolated servers and
daemons in namespaces and real OMP tabs, beside the production daemons, which
it never touched:

- A request reached only its tab's machine, in both directions.
- An account without access, a stopped machine and a machine on the previous
  CLI were each refused with the specified text, and nothing was delivered.
- A viewer that did not negotiate autopilot was sent no autopilot frame and
  kept its ordinary frames.
- Across a server restart the tab kept iterating and its state was sent again
  without anyone asking.
- Under a server rolled back alone the tab held and submitted nothing. It
  kept holding with no server at all, and continued by itself once the new
  server registered.
- An agent's exit was reported by the CLI as a pause, and its reboot brought
  the autopilot back paused.

**A browser run** in headless Chromium at 1440 px and at 390 and 360 px drove
one autopilot end to end through the server and the CLI to OMP: the start
dialog and its refusals, the markers, pause, resume, Stop, Stop now, a
reload, the agent-exited pause and the reboot, the lists and the phone's
notice and sheets.

The runs found five defects, all fixed and run again:

- a hold that ended when a server that could not control autopilot went away;
- state frames sent to a server that had not negotiated them, which logged
  each one;
- the start dialog's prompt field collapsing to about 10 px on a phone or a
  short window;
- start sheet fields small enough for iOS to zoom on focus;
- a pause reason cut short in the phone's raw terminal view. A paused
  autopilot's reason now has a line of its own there.

The last of those fixes first made the notice one line taller while a tab
waited than while it ran, which resized the terminal twice an iteration. The
condition's three lines now fit under the buttons at any root font size. That
correction is covered by a unit test and by the measured heights, not by
another browser run.

Not exercised:

- on a real OMP: a submission OMP drops, Stop and Stop now around a late or
  run-less submission, a reload (OMP has no command for one), the pause after
  four hours, autoresearch and exhausted retries. The extension's tests cover
  these with the event orders recorded in the design;
- a member who can open a session but may not use its machine, and any shared
  collaborator;
- OMP's goal and loop notices, the holds and the not-reported state in a
  browser;
- a real phone, the on-screen keyboard over the start sheet, and iOS;
- OMP versions other than 18.20.0. 18.18.2, the minimum, was checked by
  reading its source only.

On a 360 px phone the selected tab chip can sit off-screen until the chip row
is scrolled. The row behaved that way before; the autopilot glyph adds about
16 px to a marked chip.

### Rollout record

On **2026-11-13** autopilot reached production in the usual order.

- **Server** has run the new binary since 19:38 UTC, and **web** release
  `20261113T193707Z` (version 26.11.30, built at 1c24f56) since 19:39 UTC.
  Backups are under `/opt/apas/backups/deploy-20261113T193707Z`. The nginx
  configuration did not change: autopilot adds no route.
- The server was down for about a second. All nine project instances and the
  daemons registered again by themselves, viewers re-attached, and the server
  logged no error and no frame it could not parse.
- Ten public routes answered `200` with no redirect, 21 referenced static
  assets answered `200`, `/projects/by-remote` and `/projects/workers`
  answered JSON from `apas-server` with and without a credential, and both
  dependency graphs audited clean. No job policy row exists and there is no
  worker, so recurring execution stays disabled.
- **CLI:** `master` was pushed at 1c24f56. No daemon and no agent was
  restarted. Until a machine's daemon is rebooted from the Machines page, its
  OMP tabs show the control disabled with "<host> runs an older APAS that
  cannot run autopilot. Reboot <host> to update it." After that reboot, each
  tab still needs its agent rebooted once before it can run autopilot.

Not checked in production: the autopilot control itself in a signed-in
browser, and a real autopilot. Both wait for the first machine to be rebooted
and the first tab's agent to be relaunched.

### Prompt templates

**Rolled out.** The first three templates went out with web release
`20261114T002349Z`. "APAS worker" became the default with web-only release
`20261114T033111Z` (version 26.11.41, built at 0095aa2 from the committed
tree), live since 03:32 UTC on 2026-11-14; the server was not restarted.
Ten public routes and the 18 referenced static assets answered `200`, and the
web logged no error. Its backups are under
`/opt/apas/backups/deploy-20261114T033111Z`. Its wording was revised after an
independent review of how an agent reads it over many iterations; it has not
yet run on a real OMP tab.

**Checked on 2026-11-13** for the first three templates:

- The dialog's and the templates' unit tests passed, with the web typecheck
  and lint: 208 tests in nine files when the templates were finished, and 211
  when the same nine files were run again later on 2026-11-13.
- Headless Chromium at 1440 px and at 390 px, through a read-only viewer on
  the real project: the dialog opened on the default template; choosing each
  other template replaced the prompt; typed text was kept and came back with
  My own text; the phone sheet kept 16 px fields and did not scroll sideways.

Not exercised:

- **the templates' wording on a real OMP tab.** Whether an agent follows the
  two-turn ending, marks a step blocked, leaves other people's uncommitted
  work alone and stops retrying a failure it tried twice is untested. Only the
  text is asserted;
- a start through the server: the viewer forwards no request, so no autopilot
  was started from a template;
- the dialog for a tab with an earlier autopilot, in a browser (unit tests
  only);
- a real phone, iOS, the on-screen keyboard and a 360 px screen.

One limit is known. Keep the tests green remembers that it tried a failure
twice only through the conversation, and it keeps no notes file. After OMP
compacts the conversation it can try the same failure again.
