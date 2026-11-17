"use client";

import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { PolicyEditor, type LaunchProfile } from "@/components/PolicyEditor";
import { AutonomousJobPolicyEditor } from "@/components/AutonomousJobPolicyEditor";
import {
  OverviewEmpty,
  OverviewNotice,
  OverviewSection,
  overviewButton,
  overviewCard,
  overviewDangerButton,
  overviewMuted,
  overviewQuietButton,
  overviewQuietDangerButton,
} from "@/components/overview/OverviewElements";
import { UsagePeriodControl, type UsagePeriod } from "@/components/overview/UsagePeriodControl";
import { Disclosure } from "./Disclosure";
import { UsageStatCards } from "./UsageStatCards";
import { JOB_REASON_LABELS } from "./jobReasonLabels";
import { DELETE_PROJECT_LABEL, DeleteProjectDialog } from "@/components/project/DeleteProjectDialog";
import { ConnectionChip, OpenStateChip, SuspendedChip } from "./ProjectList";
import { clusterProjectOpenModel, type ClusterOpenStateSource } from "./clusterOpenState";
import { STOP_RUNTIME_CLOSES_PROJECT, SUSPEND_CLOSES_PROJECT } from "@/lib/projectOpenState";
import type { ConfirmRequest } from "./ConfirmDialog";
import type { ClusterPolicy, ClusterProject, ClusterProjectDetail, ClusterRequest, UsageWindows } from "./types";

const INPUT = "h-8 min-w-0 flex-1 rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800";

/** A write from the panel: its success text, then the panel's project is
 *  loaded again with the cluster. */
export type ProjectWrite = (path: string, init: RequestInit, success: string) => Promise<boolean>;

/**
 * The selected project: its open state and usage, for the cluster's owner
 * its runtime, owner, members and policies, and for the project's owner
 * "Delete project…". Everything here is about this one project.
 */
export function ProjectPanel({
  detail,
  summary,
  usage,
  loading,
  error,
  notice,
  sharedView,
  userId,
  openState,
  onDeleted,
  onDeletionUnconfirmed,
  usagePeriod,
  onUsagePeriod,
  profiles,
  clusterPolicy,
  write,
  requestConfirm,
  request,
  onBack,
}: {
  detail: ClusterProjectDetail | null;
  /** The list's row for the project, for whether it is connected. Shown
   *  only when the project's open state is not known. */
  summary?: ClusterProject;
  usage: UsageWindows | null;
  loading: boolean;
  error: string | null;
  notice: string | null;
  sharedView: boolean;
  /** The viewer's account. Only the project's owner is offered Delete. */
  userId?: string | null;
  /** Where the project's Open / Closed comes from. */
  openState?: ClusterOpenStateSource;
  /** The owner deleted the project and the server reported it done. It can
   *  arrive after the Delete dialog, or this panel, is gone. */
  onDeleted?: (projectId: string) => void;
  /** The Delete dialog closed without learning how an accepted deletion
   *  went: the project list may be out of date. */
  onDeletionUnconfirmed?: (projectId: string) => void;
  usagePeriod: UsagePeriod;
  onUsagePeriod: (value: UsagePeriod) => void;
  profiles: LaunchProfile[];
  clusterPolicy: ClusterPolicy | null;
  write: ProjectWrite;
  requestConfirm: (request: ConfirmRequest) => void;
  request: ClusterRequest;
  /** Shown only below the wide layout, where the panel replaces the list. */
  onBack?: () => void;
}) {
  // Nothing chosen yet: the prompt alone, not a card around an empty box.
  if (!detail && !loading && !error && !notice) {
    return <OverviewEmpty>Select a project to manage it.</OverviewEmpty>;
  }

  return (
    <div className={`${overviewCard} min-w-0 space-y-5 p-4`}>
      {onBack && (
        <button type="button" onClick={onBack} className={`${overviewQuietButton} -ml-2 lg:hidden`}>
          <ArrowLeft className="h-3.5 w-3.5" aria-hidden="true" /> All projects
        </button>
      )}
      {error && <OverviewNotice alert>{error}</OverviewNotice>}
      {notice && <OverviewNotice status>{notice}</OverviewNotice>}
      {loading && !detail ? (
        <OverviewNotice status>Loading project…</OverviewNotice>
      ) : !detail ? (
        <OverviewEmpty>Select a project to manage it.</OverviewEmpty>
      ) : (
        <ProjectDetail
          key={detail.project.id}
          detail={detail}
          summary={summary}
          usage={usage}
          sharedView={sharedView}
          userId={userId}
          openState={openState}
          onDeleted={onDeleted}
          onDeletionUnconfirmed={onDeletionUnconfirmed}
          usagePeriod={usagePeriod}
          onUsagePeriod={onUsagePeriod}
          profiles={profiles}
          clusterPolicy={clusterPolicy}
          write={write}
          requestConfirm={requestConfirm}
          request={request}
        />
      )}
    </div>
  );
}

function ProjectDetail({
  detail,
  summary,
  usage,
  sharedView,
  userId,
  openState,
  onDeleted,
  onDeletionUnconfirmed,
  usagePeriod,
  onUsagePeriod,
  profiles,
  clusterPolicy,
  write,
  requestConfirm,
  request,
}: {
  detail: ClusterProjectDetail;
  summary?: ClusterProject;
  usage: UsageWindows | null;
  sharedView: boolean;
  userId?: string | null;
  openState?: ClusterOpenStateSource;
  onDeleted?: (projectId: string) => void;
  onDeletionUnconfirmed?: (projectId: string) => void;
  usagePeriod: UsagePeriod;
  onUsagePeriod: (value: UsagePeriod) => void;
  profiles: LaunchProfile[];
  clusterPolicy: ClusterPolicy | null;
  write: ProjectWrite;
  requestConfirm: (request: ConfirmRequest) => void;
  request: ClusterRequest;
}) {
  const project = detail.project;
  const name = project.project_name || project.id;
  const base = `/cluster/projects/${project.id}`;
  const [newOwner, setNewOwner] = useState("");
  const [newMember, setNewMember] = useState("");
  const suspended = project.lifecycle_status === "suspended";
  const ownerTrimmed = newOwner.trim();
  const memberTrimmed = newMember.trim();
  // Deleting is the owner's alone; hosting a project does not grant it.
  const isOwner = Boolean(userId) && project.owner_user_id === userId;
  const [deleteOpen, setDeleteOpen] = useState(false);
  const open = openState
    ? clusterProjectOpenModel({ id: project.id, name: project.project_name }, openState, isOwner)
    : null;
  // A server that records open states records a stopped or suspended
  // project as closed. An older one records nothing, and nothing is said.
  const closesProject = openState?.supported === true;

  const usageSection = (
    <OverviewSection title="Usage" level={3} aside={<UsagePeriodControl value={usagePeriod} onChange={onUsagePeriod} />}>
      {usage?.[usagePeriod] ? <UsageStatCards counters={usage[usagePeriod]} narrow /> : <p className={overviewMuted}>No usage reported.</p>}
    </OverviewSection>
  );

  const header = (
    <div className="space-y-1">
      <h3 className="break-words text-sm font-semibold text-gray-900 dark:text-gray-100">{name}</h3>
      <div className="select-all break-all font-mono text-[11px] text-gray-500 dark:text-gray-400">{project.id}</div>
      <div className="flex flex-wrap items-center gap-2">
        {/* Only while nothing is known about the open state: see
            `ConnectionChip`. */}
        {summary && !open && <ConnectionChip connected={summary.connected} />}
        {open && <OpenStateChip label={open.stateLabel} />}
        {suspended && <SuspendedChip />}
      </div>
    </div>
  );

  // A control of its own, never part of closing or stopping the project.
  const deleteSection = isOwner ? (
    <OverviewSection title="Delete" level={3}>
      <p className={`mb-2 ${overviewMuted}`}>
        Erases the project from the APAS server. Folders and files on the machines stay.
      </p>
      <button type="button" className={overviewDangerButton} onClick={() => setDeleteOpen(true)}>
        {DELETE_PROJECT_LABEL}
      </button>
      {deleteOpen && (
        <DeleteProjectDialog
          project={{ id: project.id, name: project.project_name ?? "" }}
          stoppedFirstHosts={open?.stoppedFirstHosts}
          offlineTabOwnerHosts={open?.offlineTabOwnerHosts}
          onClose={() => setDeleteOpen(false)}
          onDeleted={onDeleted}
          onUnconfirmedClose={onDeletionUnconfirmed}
        />
      )}
    </OverviewSection>
  ) : null;

  if (sharedView) {
    return (
      <>
        {header}
        <p className={overviewMuted}>Owned by {project.owner_email}. Cluster membership does not expose any other hosted projects.</p>
        {usageSection}
        {deleteSection}
      </>
    );
  }

  return (
    <>
      <div className="space-y-1">
        {header}
        <p className={`break-all ${overviewMuted}`}>Owned by {project.owner_email}.</p>
      </div>
      {usageSection}

      <OverviewSection title="Runtime" level={3}>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className={overviewDangerButton}
            onClick={() => requestConfirm({
              title: "Stop this project's runtime?",
              body: (
                <>
                  {closesProject && <p>{STOP_RUNTIME_CLOSES_PROJECT}</p>}
                  <p>Project data and future access are preserved.</p>
                </>
              ),
              confirmLabel: "Stop runtime",
              tone: "danger",
              onConfirm: () => void write(`${base}/stop-runtime`, { method: "POST" }, "Runtime stop requested."),
            })}
          >
            Stop runtime
          </button>
          {suspended ? (
            <button
              type="button"
              className={overviewButton}
              onClick={() => void write(`${base}/lifecycle`, { method: "PATCH", body: JSON.stringify({ status: "active" }) }, "Project reactivated.")}
            >
              Reactivate project
            </button>
          ) : (
            <button
              type="button"
              className={overviewDangerButton}
              onClick={() => requestConfirm({
                title: `Suspend ${name}?`,
                body: (
                  <>
                    <p>This stops its runtime. Project data is preserved.</p>
                    {closesProject && <p>{SUSPEND_CLOSES_PROJECT}</p>}
                  </>
                ),
                confirmLabel: "Suspend project",
                tone: "danger",
                onConfirm: () => void write(`${base}/lifecycle`, { method: "PATCH", body: JSON.stringify({ status: "suspended" }) }, "Project suspended."),
              })}
            >
              Suspend project
            </button>
          )}
        </div>
      </OverviewSection>

      <OverviewSection title="Owner" level={3}>
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (!ownerTrimmed || ownerTrimmed === project.owner_user_id) return;
            requestConfirm({
              title: `Transfer ${name}?`,
              body: <p>The account with user ID <span className="break-all font-mono">{ownerTrimmed}</span> becomes the owner. You keep administering it while it runs in your cluster.</p>,
              confirmLabel: "Transfer",
              tone: "danger",
              onConfirm: () => void write(`${base}/owner`, { method: "PATCH", body: JSON.stringify({ user_id: ownerTrimmed }) }, "Owner changed.")
                .then((ok) => { if (ok) setNewOwner(""); }),
            });
          }}
        >
          <input aria-label="New owner user ID" placeholder="User ID" value={newOwner} onChange={(event) => setNewOwner(event.target.value)} className={INPUT} />
          <button type="submit" disabled={!ownerTrimmed || ownerTrimmed === project.owner_user_id} className={overviewButton}>Transfer</button>
        </form>
      </OverviewSection>

      <OverviewSection title="Members" level={3}>
        {detail.members.length === 0 ? (
          <p className={overviewMuted}>No members.</p>
        ) : (
          <ul className="divide-y divide-gray-100 dark:divide-gray-700/60">
            {detail.members.map((member) => (
              <li key={member.user_id} className="flex items-center justify-between gap-3 py-1.5">
                <span className="min-w-0 break-all text-sm text-gray-800 dark:text-gray-200">{member.email}</span>
                <button
                  type="button"
                  aria-label={`Remove ${member.email}`}
                  className={overviewQuietDangerButton}
                  onClick={() => requestConfirm({
                    title: `Remove ${member.email} from ${name}?`,
                    body: <p>They lose access to this project.</p>,
                    confirmLabel: "Remove",
                    tone: "danger",
                    onConfirm: () => void write(`${base}/members/${member.user_id}`, { method: "DELETE" }, "Member removed."),
                  })}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
        <form
          className="mt-2 flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (!memberTrimmed) return;
            void write(`${base}/members`, { method: "POST", body: JSON.stringify({ user_id: memberTrimmed }) }, "Member added.")
              .then((ok) => { if (ok) setNewMember(""); });
          }}
        >
          <input aria-label="Member user ID" placeholder="User ID" value={newMember} onChange={(event) => setNewMember(event.target.value)} className={INPUT} />
          <button type="submit" disabled={!memberTrimmed} className={overviewButton}>Add</button>
        </form>
        <p className={`mt-1 ${overviewMuted}`}>Use the account&apos;s user ID.</p>
      </OverviewSection>

      <PolicyEditor
        appearance="panel"
        title="Project policy"
        policy={detail.policy}
        profiles={profiles}
        bound={clusterPolicy?.deployment}
        onSave={(next) => void write(`${base}/policy`, { method: "PATCH", body: JSON.stringify(next) }, "Project policy saved.")}
        onInherit={() => requestConfirm({
          title: `Use the cluster policy for ${name}?`,
          body: <p>This project&apos;s own list is removed. It follows the cluster policy.</p>,
          confirmLabel: "Inherit from above",
          tone: "primary",
          onConfirm: () => void write(`${base}/policy`, { method: "PATCH", body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }) }, "Project policy saved."),
        })}
      />

      <div className="-mx-4 border-t border-gray-100 dark:border-gray-700/60">
        <Disclosure
          id="project-job-policy"
          label="Project autonomous worker policy"
          description="Limits for autonomous workers on this project."
        >
          {() => (
            <AutonomousJobPolicyEditor
              key={project.id}
              embedded
              reasonLabels={JOB_REASON_LABELS}
              title="Project autonomous worker policy"
              endpoint={`/cluster/projects/${encodeURIComponent(project.id)}/job-policy`}
              request={request}
            />
          )}
        </Disclosure>
      </div>

      {deleteSection}
    </>
  );
}
