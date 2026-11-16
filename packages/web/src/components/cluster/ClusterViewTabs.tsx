"use client";

import { useRef, type KeyboardEvent } from "react";
import { overviewChip } from "@/components/overview/OverviewElements";
import type { ClusterView } from "./types";

export const CLUSTER_VIEW_LABELS: Record<ClusterView, string> = {
  machines: "Machines",
  projects: "Projects",
  sharing: "Sharing",
  usage: "Usage",
  policies: "Policies",
  activity: "Activity",
};

/**
 * The page's views as tabs. A count rides beside a name for sighted users
 * only, so a tab's accessible name stays its plain name.
 */
export function ClusterViewTabs({
  views,
  selected,
  counts,
  onSelect,
}: {
  views: readonly ClusterView[];
  selected: ClusterView;
  counts: Partial<Record<ClusterView, number | undefined>>;
  onSelect: (view: ClusterView) => void;
}) {
  const buttons = useRef<Partial<Record<ClusterView, HTMLButtonElement | null>>>({});

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = views.indexOf(selected);
    let next: number | null = null;
    if (event.key === "ArrowRight") next = (index + 1) % views.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + views.length) % views.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = views.length - 1;
    if (next === null) return;
    event.preventDefault();
    const view = views[next];
    onSelect(view);
    buttons.current[view]?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label="Cluster views"
      onKeyDown={onKeyDown}
      className={`grid ${views.length > 2 ? "grid-cols-3" : "grid-cols-2"} gap-0.5 rounded border border-gray-300 bg-white p-0.5 sm:inline-flex dark:border-gray-600 dark:bg-gray-800`}
    >
      {views.map((view) => {
        const active = view === selected;
        const count = counts[view];
        return (
          <button
            key={view}
            ref={(element) => { buttons.current[view] = element; }}
            type="button"
            role="tab"
            id={`cluster-tab-${view}`}
            aria-controls={`cluster-view-${view}`}
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            onClick={() => onSelect(view)}
            className={`inline-flex h-8 items-center justify-center gap-1.5 rounded-sm px-3 text-xs font-medium transition-colors pointer-coarse:h-10 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 ${
              active
                ? "bg-blue-100 text-blue-700 dark:bg-blue-900/50 dark:text-blue-200"
                : "text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-gray-100"
            }`}
          >
            {CLUSTER_VIEW_LABELS[view]}
            {count != null && count > 0 && (
              <span aria-hidden="true" className={overviewChip}>{count}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
