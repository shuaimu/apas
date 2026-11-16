"use client";

import { useMemo, useState } from "react";
import { formatCompactCount, formatUsd } from "@/lib/formatCount";
import {
  OverviewEmpty,
  OverviewNotice,
  OverviewSection,
  overviewCard,
  overviewMuted,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
import { UsagePeriodControl, type UsagePeriod } from "@/components/overview/UsagePeriodControl";
import { UsageStatCards } from "./UsageStatCards";
import { ClusterResourceUse } from "./ClusterResourceUse";
import type { ClusterProject, ClusterUsageProject, ClusterUsageReport, LoadState } from "./types";

/** Projects with usage show this many, then one line for the rest. */
const FOLD_AFTER = 15;
const NUMBER_CELL = "px-3 py-2 text-right tabular-nums";

function tokens(project: ClusterUsageProject, period: UsagePeriod): number {
  const counters = project.usage[period];
  return counters.input_tokens + counters.output_tokens;
}

/** The Cost column: a reported zero is "$0.00", so the column's one blank
 *  means the cost was not reported. */
export function costText(cost: number, reported: boolean): string {
  if (!reported) return "Unavailable";
  return cost > 0 ? formatUsd(cost) : "$0.00";
}

function projectName(project: ClusterUsageProject): string {
  return project.project_name || project.project_id;
}

/** The table's rows: the heaviest users of the period first, and apart from
 *  them the projects with no use at all. */
export function usageRows(projects: ClusterUsageProject[], period: UsagePeriod): {
  used: ClusterUsageProject[];
  idle: ClusterUsageProject[];
} {
  const sorted = [...projects].sort((a, b) =>
    tokens(b, period) - tokens(a, period) || projectName(a).localeCompare(projectName(b)));
  const isIdle = (project: ClusterUsageProject) => project.usage[period].prompts === 0 && tokens(project, period) === 0;
  const used = sorted.filter((project) => !isIdle(project));
  const idle = sorted.filter(isIdle);
  // With nothing used, the idle projects are the table.
  return used.length === 0 ? { used: idle, idle: [] } : { used, idle };
}

/** The muted line under a project's name: its machine, and its owner when
 *  that is someone else. */
export function usageProjectContext(
  project: ClusterUsageProject,
  clusterProjects: ClusterProject[],
  userEmail: string | null | undefined,
): string {
  const hostname = clusterProjects.find((entry) => entry.id === project.project_id)?.hostname || "";
  const owner = project.owner_email && project.owner_email !== userEmail ? project.owner_email : "";
  return [hostname, owner].filter(Boolean).join(" · ");
}

export function UsageView({
  usage,
  load,
  period,
  onPeriod,
  clusterProjects,
  userEmail,
  onOpenProject,
}: {
  usage: ClusterUsageReport | null;
  load: LoadState;
  period: UsagePeriod;
  onPeriod: (value: UsagePeriod) => void;
  clusterProjects: ClusterProject[];
  userEmail: string | null | undefined;
  onOpenProject: (projectId: string) => void;
}) {
  const [showAll, setShowAll] = useState(false);
  const [showIdle, setShowIdle] = useState(false);
  const { used, idle } = useMemo(() => usageRows(usage?.projects ?? [], period), [usage, period]);
  const known = useMemo(() => new Set(clusterProjects.map((project) => project.id)), [clusterProjects]);
  // Names that more than one project shares: their buttons also name the
  // machine, so a screen reader can tell them apart.
  const repeated = useMemo(() => {
    const seen = new Map<string, number>();
    for (const project of usage?.projects ?? []) seen.set(projectName(project), (seen.get(projectName(project)) ?? 0) + 1);
    return new Set([...seen].filter(([, count]) => count > 1).map(([name]) => name));
  }, [usage]);
  const folded = !showAll && used.length > FOLD_AFTER;
  const shown = folded ? used.slice(0, FOLD_AFTER) : used;

  const row = (project: ClusterUsageProject) => {
    const counters = project.usage[period];
    const total = counters.input_tokens + counters.output_tokens;
    const name = projectName(project);
    const context = usageProjectContext(project, clusterProjects, userEmail);
    const hostname = clusterProjects.find((entry) => entry.id === project.project_id)?.hostname;
    const buttonName = repeated.has(name) && hostname ? `${name} on ${hostname}` : name;
    return (
      <tr key={project.project_id}>
        <th scope="row" className="max-w-72 px-3 py-2 text-left font-normal">
          {known.has(project.project_id) ? (
            <button
              type="button"
              aria-label={`Open ${buttonName} in Projects`}
              onClick={() => onOpenProject(project.project_id)}
              className="block max-w-full truncate rounded text-left font-medium text-gray-900 hover:text-blue-700 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:text-gray-100 dark:hover:text-blue-300"
            >
              {name}
            </button>
          ) : (
            <span className="block truncate font-medium">{name}</span>
          )}
          {context && <span className={`block truncate ${overviewMuted}`}>{context}</span>}
        </th>
        <td className={`${NUMBER_CELL} hidden sm:table-cell`}>{counters.prompts.toLocaleString()}</td>
        <td className={NUMBER_CELL} title={total.toLocaleString()}>{formatCompactCount(total)}</td>
        <td className={NUMBER_CELL}>{costText(counters.cost_usd, counters.cost_usd_reported)}</td>
      </tr>
    );
  };

  return (
    <>
      <OverviewSection
        title="Usage"
        aside={<UsagePeriodControl value={period} onChange={onPeriod} />}
        description="Totals to read. To limit use, change a policy, suspend a project or revoke a member."
      >
        {load === "loading" ? (
          <OverviewNotice status>Loading usage…</OverviewNotice>
        ) : load === "failed" ? (
          <OverviewEmpty>Could not load usage.</OverviewEmpty>
        ) : !usage ? (
          <OverviewEmpty>No usage recorded in this cluster yet.</OverviewEmpty>
        ) : (
          <div className="space-y-3">
            <UsageStatCards counters={usage[period]} />
            {usage.projects.length > 0 && (
              <div className={`${overviewCard} overflow-x-auto`}>
                <table className="w-full whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                  <thead>
                    <tr className="border-b border-gray-200 text-left text-xs text-gray-500 dark:border-gray-700 dark:text-gray-400">
                      <th scope="col" className="px-3 py-2 font-medium">Project</th>
                      <th scope="col" className="hidden px-3 py-2 text-right font-medium sm:table-cell">Prompts</th>
                      <th scope="col" className="px-3 py-2 text-right font-medium">Tokens</th>
                      <th scope="col" className="px-3 py-2 text-right font-medium">Cost</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100 dark:divide-gray-700/60">
                    {shown.map(row)}
                    {used.length > FOLD_AFTER && (
                      <tr>
                        <td colSpan={4} className="px-2 py-1">
                          <button type="button" onClick={() => setShowAll((value) => !value)} className={`${overviewQuietButton} w-full`}>
                            {folded ? `Show ${used.length - FOLD_AFTER} more projects` : "Show fewer"}
                          </button>
                        </td>
                      </tr>
                    )}
                    {idle.length > 0 && (
                      <tr>
                        <td colSpan={4} className="px-2 py-1">
                          <button
                            type="button"
                            aria-expanded={showIdle}
                            onClick={() => setShowIdle((value) => !value)}
                            className={`${overviewQuietButton} w-full`}
                          >
                            {showIdle
                              ? `Hide ${idle.length} ${idle.length === 1 ? "project" : "projects"} with no usage`
                              : `Show ${idle.length} ${idle.length === 1 ? "project" : "projects"} with no usage`}
                          </button>
                        </td>
                      </tr>
                    )}
                    {showIdle && idle.map(row)}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        )}
      </OverviewSection>
      <ClusterResourceUse />
    </>
  );
}
