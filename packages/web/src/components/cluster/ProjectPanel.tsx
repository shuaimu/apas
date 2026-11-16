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
import { ConnectionChip, SuspendedChip } from "./ProjectList";
import type { ConfirmRequest } from "./ConfirmDialog";
import type { ClusterPolicy, ClusterProject, ClusterProjectDetail, ClusterRequest, UsageWindows } from "./types";

const INPUT = "h-8 min-w-0 flex-1 rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800";

/** A write from the panel: its success text, then the panel's project is
 *  loaded again with the cluster. */
export type ProjectWrite = (path: string, init: RequestInit, success: string) => Promise<boolean>;

/**
 * The selected project: its usage, and for the cluster's owner its runtime,
 * owner, members and policies. Everything here is about this one project.
 */
export function ProjectPanel({
  detail,
  summary,
  usage,
  loading,
  error,
  notice,
  sharedView,
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
  /** The list's row for the project, for whether it is connected. */
  summary?: ClusterProject;
  usage: UsageWindows | null;
  loading: boolean;
  error: string | null;
  notice: string | null;
  sharedView: boolean;
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
        {summary && <ConnectionChip connected={summary.connected} />}
        {suspended && <SuspendedChip />}
      </div>
    </div>
  );

  if (sharedView) {
    return (
      <>
        {header}
        <p className={overviewMuted}>Owned by {project.owner_email}. Cluster membership does not expose any other hosted projects.</p>
        {usageSection}
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
              body: <p>Project data and future access are preserved.</p>,
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
                body: <p>This stops its runtime. Project data is preserved.</p>,
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
    </>
  );
}
