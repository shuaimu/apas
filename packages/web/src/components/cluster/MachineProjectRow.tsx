"use client";

import { useEffect, useRef, useState } from "react";
import { Play, Square } from "lucide-react";
import type { MachineProject } from "@/lib/store";
import {
  overviewButton,
  overviewChip,
  overviewMuted,
} from "@/components/overview/OverviewElements";
import { machineProjectName } from "@/lib/clusterView";
import { formatMemory } from "./types";

/** How long a Start or Stop reads "Starting…" without an answer. Nothing is
 *  sent again: the daemon reports the result, or the button comes back. */
const PENDING_TIMEOUT_MS = 30_000;

const NOT_CONNECTED = "Not connected to the server";

// A column of running rows would be a column of red buttons, so Stop is the
// plain button with a red icon, and turns red only when pointed at.
const STOP_BUTTON = `${overviewButton} hover:border-red-300 hover:text-red-700 dark:hover:border-red-800 dark:hover:text-red-300`;

/**
 * One project on one machine. Start and Stop act on this machine only: they
 * never send `all_instances`, so another machine's instance is untouched.
 */
export function MachineProjectRow({
  machineId,
  hostname,
  project,
  highlighted,
  connected,
  onStart,
  onStop,
}: {
  machineId: string;
  hostname: string;
  project: MachineProject;
  highlighted: boolean;
  connected: boolean;
  /** Each returns whether the request was sent. */
  onStart: () => boolean;
  onStop: () => boolean;
}) {
  const [pending, setPending] = useState<"start" | "stop" | null>(null);
  const before = useRef<{ isRunning: boolean; lastError?: string } | null>(null);

  useEffect(() => {
    if (!pending || !before.current) return;
    if (project.isRunning !== before.current.isRunning || project.lastError !== before.current.lastError) {
      setPending(null);
    }
  }, [pending, project.isRunning, project.lastError]);

  useEffect(() => {
    if (!pending) return;
    const timer = window.setTimeout(() => setPending(null), PENDING_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [pending]);

  const name = machineProjectName(project);
  // The control's name keeps the wording it always had, path included.
  const controlName = project.name || project.path;
  const memory = formatMemory(project.memoryKb);
  const [notSent, setNotSent] = useState(false);
  const act = (kind: "start" | "stop") => {
    before.current = { isRunning: project.isRunning, lastError: project.lastError };
    const sent = kind === "start" ? onStart() : onStop();
    // "Starting…" only for a request that went out; one the socket refused
    // says so instead of waiting 30 seconds for nothing.
    setNotSent(!sent);
    if (sent) setPending(kind);
  };

  return (
    <div
      id={`project-${encodeURIComponent(machineId)}-${encodeURIComponent(project.projectId)}`}
      tabIndex={-1}
      className={`flex flex-col gap-2 px-4 py-2 outline-none sm:flex-row sm:items-center sm:justify-between ${highlighted ? "bg-emerald-50 ring-1 ring-inset ring-emerald-300 dark:bg-emerald-950/30 dark:ring-emerald-800" : ""}`}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">{name}</div>
        <div className={`truncate ${overviewMuted}`} title={project.path}>{project.path}</div>
        {project.lastError && <div className="break-words text-xs text-red-700 dark:text-red-300">{project.lastError}</div>}
        {notSent && <div role="alert" className="text-xs text-red-700 dark:text-red-300">Not connected to the server. Nothing was sent.</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2 self-end sm:self-auto">
        {project.isRunning ? (
          <span
            className={`${overviewChip} text-green-700 dark:text-green-300`}
            title={project.pid ? `pid ${project.pid}` : undefined}
          >
            {memory ? `Running · ${memory}` : "Running"}
          </span>
        ) : (
          <span className={overviewChip}>Stopped</span>
        )}
        {project.isRunning ? (
          <button
            type="button"
            aria-label={`Stop ${controlName} on ${hostname}`}
            title={connected ? undefined : NOT_CONNECTED}
            disabled={!connected || pending !== null}
            onClick={() => act("stop")}
            className={STOP_BUTTON}
          >
            <Square className="h-3.5 w-3.5 text-red-600 dark:text-red-400" aria-hidden="true" /> {pending === "stop" ? "Stopping…" : "Stop"}
          </button>
        ) : (
          <button
            type="button"
            aria-label={`Start ${controlName} on ${hostname}`}
            title={connected ? undefined : NOT_CONNECTED}
            disabled={!connected || pending !== null}
            onClick={() => act("start")}
            className={overviewButton}
          >
            <Play className="h-3.5 w-3.5" aria-hidden="true" /> {pending === "start" ? "Starting…" : "Start"}
          </button>
        )}
      </div>
    </div>
  );
}
