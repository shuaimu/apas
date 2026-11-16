"use client";

import { useMemo, useState } from "react";
import {
  OverviewEmpty,
  overviewCard,
  overviewChip,
  overviewMuted,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
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

/** The row's second line, as one string so it can be read as one. */
export function projectFacts(project: ClusterProject, userId: string | null | undefined): string {
  return [
    project.owner_user_id === userId ? "" : `Owner ${project.owner_email}`,
    project.hostname || "",
    project.member_count > 0 ? plural(project.member_count, "member", "members") : "",
    project.active_session_count > 0 ? plural(project.active_session_count, "active session", "active sessions") : "",
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

export function ConnectionChip({ connected }: { connected: boolean }) {
  return (
    <span className={`${overviewChip} inline-flex items-center gap-1`}>
      <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${connected ? "bg-green-500" : "bg-gray-400 dark:bg-gray-500"}`} />
      {connected ? "Connected" : "Offline"}
    </span>
  );
}

export function SuspendedChip() {
  return <span className={AMBER_CHIP}>Suspended</span>;
}

export function ProjectList({
  projects,
  userId,
  selectedId,
  onSelect,
}: {
  projects: ClusterProject[];
  userId: string | null | undefined;
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
          const facts = projectFacts(project, userId);
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
                <ConnectionChip connected={project.connected} />
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
