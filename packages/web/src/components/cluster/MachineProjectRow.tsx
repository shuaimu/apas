"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { Play, Square } from "lucide-react";
import type { MachineProject, MachineWithProjects } from "@/lib/store";
import {
  overviewButton,
  overviewChip,
  overviewMuted,
} from "@/components/overview/OverviewElements";
import { machineProjectName } from "@/lib/clusterView";
import { sameHost } from "@/lib/projectInstances";
import {
  buildProjectOpenModel,
  machinePartLabel,
  NOTHING_SENT_TEXT,
  type ProjectCloseAction,
  type ProjectOpenAction,
} from "@/lib/projectOpenState";
import type { ConfirmRequest } from "./ConfirmDialog";
import { formatMemory } from "./types";

/** How long an Open or Close reads "Opening…" without the entry changing.
 *  Nothing is sent again: the server reports the result, or the button
 *  comes back. */
const PENDING_TIMEOUT_MS = 30_000;

const NOT_CONNECTED = "Not connected to the server";

// A column of open rows would be a column of red buttons, so Close is the
// plain button with a red icon, and turns red only when pointed at.
const CLOSE_BUTTON = `${overviewButton} hover:border-red-300 hover:text-red-700 dark:hover:border-red-800 dark:hover:text-red-300`;

const AMBER_TEXT = "text-amber-800 dark:text-amber-200";

/** "Open mako", "Keep mako open", "Close mako again": the control's name. */
function actionName(action: ProjectOpenAction | ProjectCloseAction, project: string): string {
  switch (action.kind) {
    case "open": return `Open ${project}`;
    case "open_again": return `Open ${project} again`;
    case "keep_open": return `Keep ${project} open`;
    case "close": return `Close ${project}`;
    case "close_again": return `Close ${project} again`;
  }
}

/**
 * One project in one machine's list: the project's open state, this
 * machine's part in it, and Open and Close for the whole project. Nothing
 * here starts or stops the project on this machine alone; both requests go
 * out with `all_instances: true`, and each is confirmed first with the other
 * machines it reaches named.
 */
export function MachineProjectRow({
  machineId,
  hostname,
  project,
  highlighted,
  connected,
  openStateSupported,
  projectMachines,
  requestConfirm,
  onOpen,
  onClose,
}: {
  machineId: string;
  hostname: string;
  project: MachineProject;
  highlighted: boolean;
  connected: boolean;
  /** The server records open states (`openStateNegotiated`). */
  openStateSupported: boolean;
  /** Every machine this viewer lists, this one included: other entries of
   *  the same project say what those machines do. */
  projectMachines: MachineWithProjects[];
  requestConfirm: (request: ConfirmRequest) => void;
  /** Each asks for the whole project and returns whether the request was
   *  sent. */
  onOpen: () => boolean;
  onClose: () => boolean;
}) {
  const name = machineProjectName(project);
  // The control's name keeps the wording it always had, path included.
  const controlName = project.name || project.path;
  const memory = formatMemory(project.memoryKb);

  const model = useMemo(() => buildProjectOpenModel({
    supported: openStateSupported,
    projectIds: [project.projectId],
    projectName: name,
    machines: projectMachines,
    target: { machineId, projectId: project.projectId, hostname },
    fromMachineId: machineId,
  }), [openStateSupported, project.projectId, name, projectMachines, machineId, hostname]);

  const here = model.thisMachine;
  const partLabel = here?.label ?? (project.isRunning ? "Running" : "Not running");
  const { openAction, closeAction } = model;
  const others = model.machines.filter((machine) => machine.machineId
    ? machine.machineId !== machineId
    : !sameHost(machine.hostname, hostname));

  // Everything the row shows about the project. A pending request is over
  // when any of it changes.
  const signature = [
    model.state,
    model.recorded,
    partLabel,
    openAction?.kind,
    closeAction?.kind,
    project.isRunning,
    project.lastError,
    others.map((machine) => `${machine.key}=${machine.part}`).join(","),
  ].join("|");
  const latestSignature = useRef(signature);
  useEffect(() => { latestSignature.current = signature; }, [signature]);

  const [pending, setPending] = useState<{ kind: "open" | "close"; signature: string } | null>(null);
  const [notSent, setNotSent] = useState(false);

  useEffect(() => {
    if (pending && pending.signature !== signature) setPending(null);
  }, [pending, signature]);

  useEffect(() => {
    if (!pending) return;
    const timer = window.setTimeout(() => setPending(null), PENDING_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [pending]);

  const send = (kind: "open" | "close") => {
    const sent = kind === "open" ? onOpen() : onClose();
    // "Opening…" only for a request that went out; one the socket refused
    // says so instead of waiting 30 seconds for nothing. Nothing is sent
    // again either way.
    setNotSent(!sent);
    if (sent) setPending({ kind, signature: latestSignature.current });
  };

  const ask = (kind: "open" | "close", action: ProjectOpenAction | ProjectCloseAction) => requestConfirm({
    title: action.confirmation.title,
    body: action.confirmation.body.map((line, index) => <p key={index}>{line}</p>),
    confirmLabel: action.label,
    tone: kind === "open" ? "primary" : "danger",
    onConfirm: () => send(kind),
  });

  const running = here ? here.running : project.isRunning;
  // The chip keeps to the part; a reason gets a line of its own.
  const partChip = here?.part === "not_started" && here.reason
    ? "Not started"
    : running && memory ? `${partLabel} · ${memory}` : partLabel;
  const reasonLine = here?.reason
    ? (here.part === "not_started" ? partLabel : here.reason)
    : undefined;
  // "Closed" beside "Not running" says one thing twice.
  const showPart = !(model.state === "closed" && partLabel === "Not running");
  const partTone = here?.part === "running" || (!here?.part && running)
    ? "text-green-700 dark:text-green-300"
    : here?.part === "still_running" || here?.part === "waiting_offline" || (here?.part === "not_started" && here.reason)
      ? AMBER_TEXT
      : "";
  // What the other machines do. For a closed project only one that is not
  // in line is worth a line; where its tabs are is said when it is opened.
  const othersLine = others
    .filter((machine) => model.state === "open"
      || (machine.part !== "not_started" && machine.part !== "waiting_offline"))
    .map((machine) => `${machine.hostname}: ${machinePartLabel({ part: machine.part, hostname: machine.hostname, partUnconfirmed: machine.partUnconfirmed }, model)}`)
    .join(" · ");
  const disabled = !connected || pending !== null;

  return (
    <div
      id={`project-${encodeURIComponent(machineId)}-${encodeURIComponent(project.projectId)}`}
      tabIndex={-1}
      className={`flex flex-col gap-2 px-4 py-2 outline-none sm:flex-row sm:items-center sm:justify-between ${highlighted ? "bg-emerald-50 ring-1 ring-inset ring-emerald-300 dark:bg-emerald-950/30 dark:ring-emerald-800" : ""}`}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">{name}</div>
        <div className={`truncate ${overviewMuted}`} title={project.path}>{project.path}</div>
        {othersLine && (
          <div data-testid="other-machines" className={`truncate ${overviewMuted}`} title={othersLine}>{othersLine}</div>
        )}
        {reasonLine && <div data-testid="part-reason" className={`break-words text-xs ${AMBER_TEXT}`}>{reasonLine}</div>}
        {project.lastError && project.lastError !== here?.reason && (
          <div className="break-words text-xs text-red-700 dark:text-red-300">{project.lastError}</div>
        )}
        {notSent && <div role="alert" className="text-xs text-red-700 dark:text-red-300">{NOTHING_SENT_TEXT}</div>}
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 self-end sm:self-auto">
        <span
          data-testid="project-open-state"
          className={overviewChip}
          title={openStateSupported && !model.recorded
            ? "Nobody has opened or closed this project yet. It is shown from what runs."
            : undefined}
        >
          {model.stateLabel}
        </span>
        {showPart && (
          <span
            data-testid="machine-part"
            className={`${overviewChip} ${partTone}`}
            title={running && project.pid ? `pid ${project.pid}` : undefined}
          >
            {partChip}
          </span>
        )}
        {openAction && (
          <button
            type="button"
            aria-label={`${actionName(openAction, controlName)} (listed on ${hostname})`}
            title={connected ? openAction.note : NOT_CONNECTED}
            disabled={disabled}
            onClick={() => ask("open", openAction)}
            className={overviewButton}
          >
            <Play className="h-3.5 w-3.5" aria-hidden="true" /> {pending?.kind === "open" ? "Opening…" : openAction.label}
          </button>
        )}
        {closeAction && (
          <button
            type="button"
            aria-label={`${actionName(closeAction, controlName)} (listed on ${hostname})`}
            title={connected ? undefined : NOT_CONNECTED}
            disabled={disabled}
            onClick={() => ask("close", closeAction)}
            className={CLOSE_BUTTON}
          >
            <Square className="h-3.5 w-3.5 text-red-600 dark:text-red-400" aria-hidden="true" /> {pending?.kind === "close" ? "Closing…" : closeAction.label}
          </button>
        )}
      </div>
    </div>
  );
}
