"use client";

import { useMemo, useState } from "react";
import {
  OverviewEmpty,
  overviewCard,
  overviewChip,
  overviewMuted,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
import type { ProjectOpenModel } from "@/lib/projectOpenState";
import { hostList } from "@/lib/tabHosts";
import { clusterProjectOpenModel, type ClusterOpenStateSource } from "./clusterOpenState";
import type { ClusterProject } from "./types";

/** Above this many projects the list gets a filter. */
const FILTER_THRESHOLD = 8;
/** The list shows this many, then one line for the rest. */
const FOLD_AFTER = 12;

const AMBER_CHIP = "shrink-0 rounded bg-amber-100 px-1.5 py-0.5 text-[11px] font-medium text-amber-800 dark:bg-amber-900/40 dark:text-amber-200";

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

function activityMs(project: ClusterProject): number {
  const at = project.last_activity ? Date.parse(project.last_activity) : Number.NaN;
  return Number.isNaN(at) ? Number.NEGATIVE_INFINITY : at;
}

/** Connected first, then the most recently active, then by name. */
export function sortClusterProjects(projects: ClusterProject[]): ClusterProject[] {
  return [...projects].sort((a, b) =>
    Number(b.connected) - Number(a.connected)
    || activityMs(b) - activityMs(a)
    || (a.project_name || a.id).localeCompare(b.project_name || b.id));
}

/**
 * The row's second line, as one string so it can be read as one.
 *
 * Without `open`, the machine is the one host the server recorded for the
 * project's session, and "1 active session" says that a CLI of the project
 * is attached now; a project has one session however many machines run it.
 * Beside a known Open or Closed both mislead: a project open on two machines
 * read "zoo-005 · 1 active session". With `open` (the project's open-state
 * model) the line names instead the machines that run the project, or, when
 * none does, the machines that have its tabs.
 */
export function projectFacts(
  project: ClusterProject,
  userId: string | null | undefined,
  open?: Pick<ProjectOpenModel, "runningHosts" | "machines"> | null,
): string {
  const where = !open
    ? project.hostname || ""
    : open.runningHosts.length > 0
      ? `Running on ${hostList(open.runningHosts)}`
      : hostList(open.machines.map((machine) => machine.hostname)) || project.hostname || "";
  return [
    project.owner_user_id === userId ? "" : `Owner ${project.owner_email}`,
    where,
    project.member_count > 0 ? plural(project.member_count, "member", "members") : "",
    !open && project.active_session_count > 0 ? plural(project.active_session_count, "active session", "active sessions") : "",
  ].filter(Boolean).join(" · ");
}

/** The id of a project's row, so focus can come back to it. */
export function projectRowId(projectId: string): string {
  return `cluster-project-${encodeURIComponent(projectId)}`;
}

/** A row's accessible name. Projects can share a name (three checkouts of
 *  one repository), so a shared name also says the machine. */
export function projectRowLabel(project: ClusterProject, projects: ClusterProject[]): string {
  const name = project.project_name || project.id;
  const shared = projects.some((other) => other.id !== project.id && (other.project_name || other.id) === name);
  return shared && project.hostname ? `Manage ${name} on ${project.hostname}` : `Manage ${name}`;
}

/**
 * Whether a CLI of the project is attached to the server now: the cluster
 * list's `connected`, which is about the project's session and not about any
 * machine. It is shown only while nothing is known about the project's open
 * state. Beside "Closed" it read "Offline" for a project whose machines were
 * all connected, a second state that contradicted the first; the open state
 * and the machines named in the row's second line say what it said.
 */
export function ConnectionChip({ connected }: { connected: boolean }) {
  return (
    <span className={`${overviewChip} inline-flex items-center gap-1`}>
      <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${connected ? "bg-green-500" : "bg-gray-400 dark:bg-gray-500"}`} />
      {connected ? "Connected" : "Offline"}
    </span>
  );
}

/** "Open" or "Closed": the project's state as every list shows it. */
export function OpenStateChip({ label }: { label: "Open" | "Closed" }) {
  return (
    <span data-testid="project-open-state" className={overviewChip}>
      {label}
    </span>
  );
}

export function SuspendedChip() {
  return <span className={AMBER_CHIP}>Suspended</span>;
}

export function ProjectList({
  projects,
  userId,
  openState,
  selectedId,
  onSelect,
}: {
  projects: ClusterProject[];
  userId: string | null | undefined;
  /** Where each project's Open / Closed comes from; none is shown without
   *  it, or for a project nothing is known about. */
  openState?: ClusterOpenStateSource;
  selectedId: string | null;
  onSelect: (projectId: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const [showAll, setShowAll] = useState(false);
  const sorted = useMemo(() => sortClusterProjects(projects), [projects]);
  const needle = filter.trim().toLowerCase();
  const matching = needle
    ? sorted.filter((project) => [project.project_name, project.id, project.owner_email, project.hostname]
      .some((field) => field?.toLowerCase().includes(needle)))
    : sorted;
  const folded = !showAll && matching.length > FOLD_AFTER;
  const shown = folded
    ? matching.filter((project, index) => index < FOLD_AFTER || project.id === selectedId)
    : matching;
  const hidden = matching.length - shown.length;

  if (projects.length === 0) {
    return <OverviewEmpty>No projects are hosted in your cluster yet.</OverviewEmpty>;
  }

  return (
    <div className="min-w-0 space-y-2">
      {projects.length > FILTER_THRESHOLD && (
        <input
          type="search"
          aria-label="Filter projects"
          placeholder="Filter by name, owner or machine"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          className="h-8 w-full rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800"
        />
      )}
      <div className={`${overviewCard} divide-y divide-gray-100 overflow-hidden dark:divide-gray-700/60`}>
        {shown.length === 0 && <p className={`px-4 py-3 ${overviewMuted}`}>No project matches the filter.</p>}
        {shown.map((project) => {
          const selected = project.id === selectedId;
          const open = openState
            ? clusterProjectOpenModel({ id: project.id, name: project.project_name }, openState)
            : null;
          const facts = projectFacts(project, userId, open);
          return (
            <button
              key={project.id}
              id={projectRowId(project.id)}
              type="button"
              aria-label={projectRowLabel(project, projects)}
              aria-current={selected ? "true" : undefined}
              onClick={() => onSelect(project.id)}
              className={`block w-full px-4 py-2 text-left transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-blue-500 ${selected ? "bg-blue-50 dark:bg-blue-950/30" : "hover:bg-gray-50 dark:hover:bg-gray-700/40"}`}
            >
              <span className="flex min-w-0 items-center gap-2">
                <span className="min-w-0 truncate text-sm font-medium text-gray-900 dark:text-gray-100">
                  {project.project_name || "Unnamed project"}
                </span>
                {!open && <ConnectionChip connected={project.connected} />}
                {open && <OpenStateChip label={open.stateLabel} />}
                {project.lifecycle_status === "suspended" && <SuspendedChip />}
              </span>
              {facts && <span className={`block truncate ${overviewMuted}`}>{facts}</span>}
            </button>
          );
        })}
        {(folded || (showAll && matching.length > FOLD_AFTER)) && (
          <div className="px-2 py-1">
            <button
              type="button"
              onClick={() => setShowAll((value) => !value)}
              className={`${overviewQuietButton} w-full`}
            >
              {folded ? `Show ${hidden} more projects` : "Show fewer"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
