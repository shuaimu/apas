"use client";

import { useEffect, useRef } from "react";
import { OverviewEmpty, OverviewNotice, OverviewSection } from "@/components/overview/OverviewElements";
import type { LaunchProfile } from "@/components/PolicyEditor";
import type { UsagePeriod } from "@/components/overview/UsagePeriodControl";
import { ProjectList, projectRowId } from "./ProjectList";
import { ProjectPanel, type ProjectWrite } from "./ProjectPanel";
import type { ConfirmRequest } from "./ConfirmDialog";
import type {
  ClusterPolicy,
  ClusterProject,
  ClusterProjectDetail,
  ClusterRequest,
  LoadState,
  UsageWindows,
} from "./types";

/**
 * Every project hosted in the cluster beside the selected one's panel. On a
 * narrow screen the panel takes the list's place until "All projects".
 */
export function ProjectsView({
  sharedView,
  projects,
  load,
  userId,
  selectedId,
  onSelect,
  onClear,
  detail,
  detailUsage,
  detailLoading,
  projectError,
  projectNotice,
  usagePeriod,
  onUsagePeriod,
  profiles,
  clusterPolicy,
  write,
  requestConfirm,
  request,
}: {
  sharedView: boolean;
  projects: ClusterProject[];
  load: LoadState;
  userId: string | null | undefined;
  selectedId: string | null;
  onSelect: (projectId: string) => void;
  onClear: () => void;
  detail: ClusterProjectDetail | null;
  detailUsage: UsageWindows | null;
  detailLoading: boolean;
  projectError: string | null;
  projectNotice: string | null;
  usagePeriod: UsagePeriod;
  onUsagePeriod: (value: UsagePeriod) => void;
  profiles: LaunchProfile[];
  clusterPolicy: ClusterPolicy | null;
  write: ProjectWrite;
  requestConfirm: (request: ConfirmRequest) => void;
  request: ClusterRequest;
}) {
  const panel = useRef<HTMLDivElement | null>(null);
  const hasSelection = selectedId !== null;

  // Below the wide layout the panel replaces the list, so the row that had
  // focus is hidden: bring the panel into view and give it focus. Going back
  // to the list gives focus to the row that was chosen. Beside the list the
  // panel is already in view and sticks there, and focus stays on the row.
  const previous = useRef<string | null>(selectedId);
  useEffect(() => {
    const was = previous.current;
    previous.current = selectedId;
    if (window.matchMedia?.("(min-width: 1024px)").matches) return;
    if (selectedId) {
      panel.current?.scrollIntoView?.({ block: "start" });
      panel.current?.focus({ preventScroll: true });
    } else if (was) {
      document.getElementById(projectRowId(was))?.focus();
    }
  }, [selectedId]);

  return (
    <OverviewSection
      title="Projects in this cluster"
      description={sharedView
        ? "Projects on this cluster that your account can see."
        : "Every project that runs under your account, including projects another account owns. You administer these without being a member of them."}
    >
      {load === "loading" ? (
        <OverviewNotice status>Loading projects…</OverviewNotice>
      ) : load === "failed" ? (
        <OverviewEmpty>Could not load projects.</OverviewEmpty>
      ) : (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_26rem] lg:items-start">
          <div className={`min-w-0 ${hasSelection ? "max-lg:hidden" : ""}`}>
            <ProjectList projects={projects} userId={userId} selectedId={selectedId} onSelect={onSelect} />
          </div>
          <div
            ref={panel}
            tabIndex={-1}
            role="region"
            aria-label="Selected project"
            className={`min-w-0 scroll-mt-4 outline-none lg:sticky lg:top-4 lg:max-h-[calc(100vh-2rem)] lg:overflow-y-auto ${hasSelection ? "" : "max-lg:hidden"}`}
          >
            <ProjectPanel
              detail={detail}
              summary={projects.find((project) => project.id === detail?.project.id)}
              usage={detailUsage}
              loading={detailLoading}
              error={projectError}
              notice={projectNotice}
              sharedView={sharedView}
              usagePeriod={usagePeriod}
              onUsagePeriod={onUsagePeriod}
              profiles={profiles}
              clusterPolicy={clusterPolicy}
              write={write}
              requestConfirm={requestConfirm}
              request={request}
              onBack={hasSelection ? onClear : undefined}
            />
          </div>
        </div>
      )}
    </OverviewSection>
  );
}
