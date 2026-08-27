# Shared clusters

Shared clusters let an existing APAS account run its own projects on machines
owned by another account. Each account still owns one cluster and may also
join several clusters shared by trusted collaborators.

## Trust boundary

Only invite people you trust to run code as the operating-system user that
runs your APAS daemon. A member can direct agents and project processes on your
machines. APAS isolates the initial Git clone from the machine owner's Git,
SSH, askpass, and credential-helper configuration, but it does **not** sandbox
the resulting project runtime from the owner's files, processes, environment,
network, or credentials.

Both the invitation creator and recipient must explicitly acknowledge this
warning. Revoking membership stops new provisioning and shared runtime
mutations immediately, including on WebSocket connections that were already
open. It does not delete project data, remove the checkout, or revoke the
former member's normal ownership/content access to projects they created.

## Prerequisites and workflow

The recipient must already have an active APAS account. A shared-cluster
invitation never creates an account; deployment account provisioning remains a
system-administrator operation.

1. The cluster owner opens **Machines**, stays in **My cluster**, enters the
   recipient's APAS email under **Cluster sharing**, confirms the trust warning,
   and creates the invitation.
2. The owner copies the one-time invitation link to the addressed user. Creating
   another invitation for the same pending recipient replaces the pending link;
   old and expired links cannot be accepted.
3. The recipient signs in as the addressed account, opens the link, confirms
   the warning, and joins the cluster.
4. The recipient selects the shared cluster on desktop or mobile and creates a
   project from an exact public GitHub HTTPS URL such as
   `https://github.com/owner/repository`. Private repositories, other Git hosts,
   embedded credentials, SSH/file URLs, alternate ports, query strings, and
   fragments are rejected. Shared checkouts always live below the daemon
   owner's managed `~/apas_projects` directory.
5. The recipient owns the new project. The cluster owner sees it in hosted
   project inventory and may manage its members, owner, lifecycle, runtime,
   and policy.

Shared project creation is shown only for a daemon that advertises the safe
shared-provisioning capability. An older daemon must be upgraded and
reconnected; the server never downgrades a member request to the owner's
credential-aware clone mode.

## Roles and visibility

| Capability | Cluster owner | Active cluster member |
| --- | --- | --- |
| See machines | Full owned-machine view | Safe metadata for shared machines |
| See projects | Every project placed in the cluster | Only projects they own or belong to |
| Create project | Existing trusted clone flow | Public GitHub HTTPS only |
| Project runtime and conversation | Accessible projects | Accessible projects while membership is active |
| Hosted-project members, owner, lifecycle, policy | Manage | No cluster-operator control |
| Invitations and cluster members | Manage | No |
| Reboot daemon or configure providers | Manage | No |
| Cluster audit and aggregate usage | View | No |
| Accessible-project usage | View | View |

Machine-owner provider keys and configuration, provider quota snapshots,
unrelated projects, and owner-only controls are not serialized to member
clients. Explicit refreshes and heartbeat pushes use the same filtered machine
view.

## Usage reporting

The owner view reports observed tokens for lifetime, trailing seven days, and
UTC today, with project and current project-owner breakdowns. A project hosted
in more than one cluster appears in each host owner's report. Do not sum those
cluster views to derive a deployment total.

Cost is shown only when events actually reported a cost. An unavailable cost
is not treated as zero. These counters are operational visibility—not
billing-grade human attribution, a budget, a reservation, a rate limit, or a
hard spending cap. Project-owner grouping reflects current ownership and does
not prove which person caused historical usage.

## Revocation and follow-up

Revocation removes the member's authority to consume that cluster's machines
on their next command and cancels an in-flight shared clone before it can
become a project. If a clone completed locally during that race, the server asks
the daemon to discard only the checkout whose server request marker and project
ID match.

Member-owned projects remain placed in the cluster after revocation so the
owner can decide what happens next. The owner can stop or suspend runtime,
transfer ownership, change project access/policy, or perform a separately
authorized cleanup. APAS does not automatically delete the checkout.

## Rollout and rollback

Roll out in this order:

1. Back up the server database, then deploy the additive schema and server.
2. Verify that migrated project placement is the exact union of the old project
   owner and canonical-session host accounts, and compare effective policy for
   multi-host projects.
3. Upgrade and reconnect daemons. Confirm eligible machines advertise shared
   provisioning before enabling invitations for users.
4. Deploy the web UI and verify owner inventory, redacted member inventory,
   a public-repository clone, revocation, usage totals, and audit events.

The schema is additive, but rolling application binaries back after shared
projects exist is not authorization-safe: older code infers hosting from
project/session identity and does not understand shared provisioning. Before a
binary rollback, stop issuing invitations, revoke or suspend shared compute,
and retain the membership, placement, and provisioning tables for the corrected
redeployment. Do not drop those tables or restore an older database over shared
project records.
