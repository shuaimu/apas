"use client";

import { RotateCcw } from "lucide-react";
import type { CreationOperation, MachineWithProjects } from "@/lib/store";
import { installedProvidersSummary } from "@/lib/machineChoice";
import { daemonVersionLabel, rebootActionLabelFor, rebootLabelFor } from "@/lib/daemonVersion";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import { machineRowsToShow } from "@/lib/clusterView";
import {
  overviewButton,
  overviewCard,
  overviewChip,
  overviewMuted,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
import { MachineProjectRow } from "./MachineProjectRow";
import { formatMemory } from "./types";

const AMBER_CHIP = "shrink-0 rounded bg-amber-100 px-1.5 py-0.5 text-[11px] font-medium text-amber-800 dark:bg-amber-900/40 dark:text-amber-200";

function pendingLabel(pending: CreationOperation): string {
  if (pending.source === "clone") return "Creating…";
  if (pending.status === "pending") return "Registering…";
  if (pending.status === "unconfirmed") return "Unconfirmed";
  return "Failed";
}

/**
 * One machine: whether its daemon is connected and current, what it can run,
 * and its projects with Start and Stop. More than three stopped projects fold
 * into one line, so a machine with many checkouts does not bury the rest.
 */
export function MachineCard({
  entry,
  behind,
  latestVersion,
  showReboot,
  connected,
  pending,
  stoppedOpen,
  onToggleStopped,
  registrationProjectId,
  onReboot,
  onStart,
  onStop,
}: {
  entry: MachineWithProjects;
  behind: boolean;
  latestVersion: string | null;
  showReboot: boolean;
  connected: boolean;
  pending: CreationOperation[];
  stoppedOpen: boolean;
  onToggleStopped: () => void;
  /** The project a link asked to reveal on this machine, if any. */
  registrationProjectId?: string;
  onReboot: () => void;
  onStart: (projectId: string) => boolean;
  onStop: (projectId: string) => boolean;
}) {
  const { machine, projects, daemonConnected, projectInstancesAvailable } = entry;
  const installed = installedProvidersSummary(machine.providers);
  const rows = machineRowsToShow(projects, pending.length, stoppedOpen);
  const running = projects.filter((project) => project.isRunning);
  const runningMemoryKb = running.reduce((sum, project) => sum + (project.memoryKb ?? 0), 0);
  const runningMemory = running.some((project) => project.memoryKb != null) ? formatMemory(runningMemoryKb) : "";
  const hostname = machine.hostname;

  let attention: { text: string; title: string } | null = null;
  if (behind) {
    attention = {
      text: "Update available",
      title: `This machine runs ${daemonVersionLabel(machine.daemonVersion)}. The newest version seen is ${latestVersion ?? "unknown"}.`,
    };
  } else if (daemonConnected === true && projectInstancesAvailable === false) {
    attention = {
      text: "Older APAS",
      title: `${hostname} runs an older APAS that cannot run a project beside other machines. Reboot ${hostname} to update it.`,
    };
  }

  return (
    <div className={`${overviewCard} divide-y divide-gray-100 dark:divide-gray-700/60`}>
      <div className="space-y-0.5 px-4 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="min-w-0 break-all text-sm font-medium text-gray-900 dark:text-gray-100">{hostname}</span>
          {daemonConnected === true && (
            <span className={`${overviewChip} inline-flex items-center gap-1`}>
              <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-green-500" />Connected
            </span>
          )}
          {daemonConnected === false && (
            <span className={`${overviewChip} inline-flex items-center gap-1`}>
              <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-gray-400 dark:bg-gray-500" />Offline
            </span>
          )}
          {attention && <span className={AMBER_CHIP} title={attention.title}>{attention.text}</span>}
          {/* A daemon is per machine, so the restart is addressed by machine
              rather than through any project running on it. */}
          {showReboot && (
            <button
              type="button"
              aria-label={rebootActionLabelFor(behind, hostname)}
              onClick={onReboot}
              className={`${overviewButton} ml-auto`}
            >
              <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" /> {rebootLabelFor(behind)}
            </button>
          )}
        </div>
        <div className={overviewMuted}>
          {`${machine.os}/${machine.arch} · ${daemonVersionLabel(machine.daemonVersion)}`}
          {daemonConnected !== true && machine.lastSeen && (
            <span title={new Date(machine.lastSeen).toLocaleString()}>
              {` · Last seen ${formatRelativeTime(Date.parse(machine.lastSeen))}`}
            </span>
          )}
        </div>
        {installed
          ? <div className={overviewMuted} title={installed.title}>{installed.text}</div>
          : <div className={overviewMuted}>Providers not reported</div>}
        <div className={overviewMuted}>
          {rows.empty
            ? "No projects on this machine."
            : `${running.length} of ${projects.length} projects running${runningMemory ? ` · ${runningMemory}` : ""}`}
        </div>
      </div>

      {pending.map((operation) => (
        <div key={operation.requestId} className="flex items-center justify-between gap-3 px-4 py-2">
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">
              {operation.source === "clone" ? operation.instanceName : operation.path}
            </div>
            <div className={`truncate ${overviewMuted}`}>
              {operation.source === "clone" ? `Cloning ${operation.gitRemote}…` : operation.error || "Registering existing folder…"}
            </div>
          </div>
          <span className={AMBER_CHIP}>{pendingLabel(operation)}</span>
        </div>
      ))}

      {rows.shown.map((project) => (
        <MachineProjectRow
          key={project.projectId}
          machineId={machine.machineId}
          hostname={hostname}
          project={project}
          highlighted={registrationProjectId === project.projectId}
          connected={connected}
          onStart={() => onStart(project.projectId)}
          onStop={() => onStop(project.projectId)}
        />
      ))}

      {rows.foldable && (
        <div className="px-2 py-1">
          <button
            type="button"
            aria-expanded={stoppedOpen}
            onClick={onToggleStopped}
            className={`${overviewQuietButton} w-full`}
          >
            {stoppedOpen ? "Hide stopped projects" : `Show ${rows.hiddenStopped} stopped projects`}
          </button>
        </div>
      )}
    </div>
  );
}
