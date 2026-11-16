"use client";

import { useState } from "react";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import { actionLabel, resolveActor, resolveTarget } from "@/lib/clusterActivity";
import {
  OverviewEmpty,
  OverviewNotice,
  OverviewSection,
  overviewCard,
  overviewMuted,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
import type { AuditEvent, ClusterMembership, ClusterProject, LoadState } from "./types";

const FOLD_AFTER = 10;

/** The latest things done in the cluster, in words, with who did them. */
export function ActivityView({
  events,
  load,
  projects,
  members,
  userId,
  onOpenProject,
}: {
  events: AuditEvent[];
  load: LoadState;
  projects: ClusterProject[];
  members: ClusterMembership[];
  userId: string | null | undefined;
  onOpenProject: (projectId: string) => void;
}) {
  const [showAll, setShowAll] = useState(false);
  const shown = showAll ? events : events.slice(0, FOLD_AFTER);

  return (
    <OverviewSection
      title="Cluster activity"
      aside={load === "loaded" && events.length > 0
        ? <span className={overviewMuted}>{events.length === 1 ? "The latest event" : `The latest ${events.length} events`}</span>
        : undefined}
    >
      {load === "loading" ? (
        <OverviewNotice status>Loading activity…</OverviewNotice>
      ) : load === "failed" ? (
        <OverviewEmpty>Could not load activity.</OverviewEmpty>
      ) : events.length === 0 ? (
        <OverviewEmpty>Nothing has been recorded for this cluster yet.</OverviewEmpty>
      ) : (
        <div className={`${overviewCard} divide-y divide-gray-100 dark:divide-gray-700/60`}>
          {shown.map((event) => {
            const action = actionLabel(event.action);
            const target = resolveTarget(event, projects, members);
            const actor = resolveActor(event, userId, projects, members);
            const at = event.created_at ? Date.parse(event.created_at) : Number.NaN;
            return (
              <div key={event.id} className="min-w-0 px-4 py-2">
                <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                  {/* The server's code is kept for whoever needs it, in the
                      label's title rather than on every row. */}
                  <span className="text-sm text-gray-900 dark:text-gray-100" title={action.known ? event.action : undefined}>{action.label}</span>
                  {target.kind === "project" ? (
                    <span className="min-w-0 text-sm">
                      <button
                        type="button"
                        aria-label={`Open ${target.label}${target.hostname ? ` on ${target.hostname}` : ""} in Projects`}
                        onClick={() => onOpenProject(target.projectId)}
                        className="break-all rounded text-left font-medium text-blue-700 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:text-blue-300"
                      >
                        {target.label}
                      </button>
                      {target.hostname && <span className={`ml-1 ${overviewMuted}`}>{target.hostname}</span>}
                    </span>
                  ) : (
                    <span
                      className="min-w-0 break-all text-sm text-gray-700 dark:text-gray-200"
                      title={target.kind === "other" ? target.title : undefined}
                    >
                      {target.label}
                    </span>
                  )}
                </div>
                <div className={`break-words ${overviewMuted}`}>
                  {!Number.isNaN(at) && (
                    <span title={new Date(at).toLocaleString()}>{formatRelativeTime(at)} · </span>
                  )}
                  <span>{actor}</span>
                </div>
              </div>
            );
          })}
          {events.length > FOLD_AFTER && (
            <div className="px-2 py-1">
              <button type="button" onClick={() => setShowAll((value) => !value)} className={`${overviewQuietButton} w-full`}>
                {showAll ? "Show fewer" : `Show ${events.length - FOLD_AFTER} more`}
              </button>
            </div>
          )}
        </div>
      )}
    </OverviewSection>
  );
}
