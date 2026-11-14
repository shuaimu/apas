## Why

APAS currently exposes two incomplete versions of unattended work: a bot that repeats a prompt and an autonomous job that ends after one approved task. The requested product is a persistent worker with an ongoing job description, a standing checklist, and authority to discover and prioritize concrete work within that responsibility.

## What Changes

- Introduce project-owned autonomous workers with stable identity, an account-approved versioned mandate, a human-controlled standing checklist, a worker-maintained backlog, durable checkpoints, and explicit running, idle, waiting, paused, blocked, and recovery states.
- Approve the continuing responsibility and its execution envelope once. The worker may discover, add, refine, and prioritize work inside that mandate without asking the person to launch each task. It cannot expand its mandate, permissions, budget, repository, or execution target.
- Separate persistent worker lifetime from bounded OMP execution. APAS persists scheduling and continuation; OMP provides the model/tool/delegation loop within each run. No-work inspection is a successful idle outcome, not an excuse to fabricate changes or continuously call the model.
- Reuse the job engine's fenced execution, structured questions, isolated verification, draft-only publication, receipts, and evidence. Add worker-wide allowances across runs, a finite recurring budget, work-item identity, and an explicit no-change inspection result. Completing a task or delivering a PR does not complete the worker.
- **BREAKING:** replace new bot and standalone one-shot-job creation with one Autonomous worker flow on desktop, mobile browser, and native mobile. Keep ordinary interactive panes and provider behavior unchanged. Show worker descriptions, current work, backlog/checklist, status, next check, budgets, and results in project Overview and cross-project Workers/Attention.
- **BREAKING:** remove legacy bot drivers and creation commands after draining old unattended execution. Preserve bot prompts/provider/cadence/history read-only, with no bot-to-worker conversion. Users create and approve fresh workers separately. Never turn a finite prior approval into perpetual spending, silently switch a provider, or reinterpret a live terminal as a qualified OMP worker.

## Capabilities

### New Capabilities

- `autonomous-workers`: standing mandates, autonomous backlog ownership, durable scheduling/checkpoints, lifecycle controls, authority, receipts, finite-job conversion, and read-only legacy bot history.
- `autonomous-worker-execution`: bounded inspection and implementation runs, structured state updates, cumulative accounting, OMP readiness, recovery, and review-aware continuation.
- `autonomous-worker-surfaces`: shared browser/native worker creation, oversight, questions, results, offline continuity, notifications, and history navigation.

### Modified Capabilities

- `project-policy-governance`: independently enable recurring workers, constrain recurring allowances and cadence, and enforce worker authority at every automatic dispatch.
- `web-project-workspace`: expose one worker inventory and typed worker navigation alongside, but independent of, interactive panes.

## Impact

Affects shared Rust/protocol contracts; server job storage, scheduling, policy, routes, lifecycle cleanup and notifications; CLI job hosts, OMP bootstrap and legacy bot paths; browser/native job screens, caches and navigation; and existing operational guidance. Worker APIs remain under `/projects/`; UI routes remain outside nginx's API prefixes.

This is a successor to `add-autonomous-jobs`, not a claim that its seven remaining operational gates are complete. Its implementation/history remain intact; this change supersedes its task-only product/admission boundary for worker-originated execution while preserving its containment and delivery guarantees. Reconcile that explicit dependency before either change's final spec synchronization/archive. The parallel `run-project-tabs-across-machines` proposal does not grant worker failover or change worker identity.

This proposal neither implements nor deploys the feature, enables recurring spend, restarts providers, nor introduces managed teams, automatic merge, arbitrary external issue integrations, or additional worker providers beyond the qualified OMP execution backend.

## Applicability after `add-omp-tab-autopilot`

This change's "one unattended product" position covers contained, approved, recurring work: a worker with a mandate, a budget, independent verification and draft-PR publication. Since `add-omp-tab-autopilot` (deployed and archived on 2026-11-13), a person can also leave one interactive OMP terminal tab repeating a prompt on autopilot, under that change's requirements in `omp-tab-autopilot`.

Autopilot is not a worker and not the bot restored. It shares no job, policy, ledger, verifier, publisher or wire type with this change and satisfies none of its gates. The binding statements here still hold: Start bot stays removed, no legacy path starts a loop, a bot pane cannot be put on autopilot, and no terminal is adopted as a worker.

The requirement formerly titled "One unattended product replaces legacy creation paths" in `autonomous-worker-surfaces` is now titled "The worker flow replaces legacy creation paths". Its text is unchanged. The owner asked on 2026-11-13 for the outdated changes to be updated.

## Applicability after `refine-interactive-workspace`

On 2026-11-13 `refine-interactive-workspace` removed the step in the project's `+` control that chose between an interactive agent and an autonomous worker, and rebuilt the project Overview. This change's worker flow is unchanged. What changed is where it starts and where it is listed:

- `+` creates an interactive agent only, on the desktop and in the mobile browser. An autonomous worker is created from **New autonomous worker** in the Overview's Autonomous workers section, or from `/code/workers`.
- The Overview shows the project's interactive sections first and the worker inventory with earlier results after them. On the desktop the interactive sections are tabs, usage, resource use and allowed launch profiles. In the mobile browser it is the tabs.

The requirements "Worker navigation preserves terminal and project identity" and "Project Overview presents persistent workers independently of panes" in this change's `web-project-workspace` delta, and section 10 of `design.md`, were corrected in place, to follow the changes the owner asked for on 2026-11-13. No gate task changed.
