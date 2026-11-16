"use client";

import { useEffect, useRef, useState } from "react";
import { useStore } from "@/lib/store";
import { overviewButton } from "@/components/overview/OverviewElements";
import {
  requestProjectDeletion,
  waitForProjectDeletion,
  type DeletionWaitOptions,
} from "@/lib/projectDeletion";
import { hostList } from "@/lib/tabHosts";

/** The control that opens this dialog, wherever it is offered: beside Close
 *  in the workspace, in the cluster page's project panel and in project
 *  access. Always to the project's owner only. */
export const DELETE_PROJECT_LABEL = "Delete project…";

export interface DeleteProjectDialogProps {
  /** The project's id, which the server takes as its confirmation value,
   *  and the name the user types. */
  project: { id: string; name: string };
  /** Connected machines that run the project now: their agents are stopped
   *  first (`ProjectOpenModel.stoppedFirstHosts`). */
  stoppedFirstHosts?: string[];
  /** Offline machines that own tabs of the project: the deletion cannot
   *  stop them (`ProjectOpenModel.offlineTabOwnerHosts`). */
  offlineTabOwnerHosts?: string[];
  /** Cancel, Escape, a press on the backdrop, and the end of a deletion. */
  onClose: () => void;
  /** The server reported the deletion done and the project was dropped from
   *  the lists. Called before `onClose`, and only while the dialog is open. */
  onDeleted?: (projectId: string) => void;
  /** How often to ask whether the deletion is done; the shared default
   *  otherwise. */
  deletionWait?: DeletionWaitOptions;
}

/** `idle`: nothing sent. `sending`: the one request is out. `deleting`: the
 *  server accepted it. `unconfirmed`: accepted, but this dialog stopped
 *  learning how it went; nothing more is sent. */
type Phase = "idle" | "sending" | "deleting" | "unconfirmed";

// The Overview's button shape, filled: the one button this dialog is for.
const dangerSolidButton = "inline-flex h-8 pointer-coarse:h-10 pointer-coarse:px-3 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded border border-red-600 bg-red-600 px-2.5 text-xs font-medium text-white transition-colors hover:bg-red-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-red-500 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-red-600";

function offlineOwnersText(hosts: string[]): string {
  return hosts.length === 1
    ? `${hosts[0]} is offline and owns tabs of this project, so it cannot be stopped now. If ${hosts[0]} reconnects while it still runs the project, the project comes back from it.`
    : `${hostList(hosts)} are offline and own tabs of this project, so they cannot be stopped now. If one of them reconnects while it still runs the project, the project comes back from it.`;
}

/**
 * The one dialog that deletes a project (design Decision 12 of
 * `make-open-a-project-state`). It says what is erased and what stays, which
 * machines' agents are stopped first, and asks for the project's name. It
 * sends one request, shows "Deleting…" until the server reports the deletion
 * done, and never sends a second one by itself.
 *
 * Render it only for the project's owner, and only while it is open: it
 * takes focus when mounted and gives it back when unmounted. Closing a
 * project never opens it.
 */
export function DeleteProjectDialog({
  project,
  stoppedFirstHosts = [],
  offlineTabOwnerHosts = [],
  onClose,
  onDeleted,
  deletionWait,
}: DeleteProjectDialogProps) {
  const token = useStore((state) => state.token);
  const [typed, setTyped] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  const input = useRef<HTMLInputElement | null>(null);
  const mounted = useRef(true);
  const opener = useRef<HTMLElement | null>(null);
  // Set with the click, before any state lands, so two quick presses cannot
  // send two requests.
  const requestOut = useRef(false);

  useEffect(() => {
    mounted.current = true;
    // Kept across a development double-mount, which would otherwise take the
    // dialog's own input for the opener.
    opener.current ??= document.activeElement instanceof HTMLElement ? document.activeElement : null;
    input.current?.focus();
    return () => {
      mounted.current = false;
      opener.current?.focus?.();
    };
  }, []);

  // A project list may show a project without a name; its id is then what
  // identifies it, and what is typed.
  const expected = project.name.trim() || project.id;
  const matches = typed.trim() === expected;
  const busy = phase !== "idle";

  const confirmDeletion = async () => {
    if (!matches || requestOut.current) return;
    if (!token) {
      setError("You are not signed in. Nothing was sent.");
      return;
    }
    requestOut.current = true;
    setError(null);
    setPhase("sending");
    try {
      await requestProjectDeletion(project.id, token);
    } catch (cause) {
      // Refused, or it never got an answer. Either way this dialog sends
      // nothing more unless the user asks again.
      requestOut.current = false;
      if (!mounted.current) return;
      setPhase("idle");
      setError(cause instanceof Error && cause.message ? cause.message : "Failed to delete project");
      return;
    }
    if (mounted.current) setPhase("deleting");
    try {
      await waitForProjectDeletion(project.id, token, deletionWait);
    } catch (cause) {
      // The server accepted the deletion and goes on with it. The request is
      // not sent again.
      if (!mounted.current) return;
      setPhase("unconfirmed");
      setError(cause instanceof Error && cause.message ? cause.message : "Failed to check deletion status");
      return;
    }
    // The project is gone on the server whether or not this dialog is still
    // open: drop it from the lists.
    const store = useStore.getState();
    store.forgetProject(project.id);
    store.listSessions();
    if (!mounted.current) return;
    onDeleted?.(project.id);
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label="Delete project"
        tabIndex={-1}
        className="max-h-full w-full max-w-lg overflow-y-auto rounded-lg border border-gray-200 bg-white p-4 shadow-xl outline-none dark:border-gray-700 dark:bg-gray-800"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
            return;
          }
          if (event.key !== "Tab") return;
          const stops = Array.from(panel.current?.querySelectorAll<HTMLElement>("input:not([disabled]), button:not([disabled])") ?? []);
          if (stops.length === 0) return;
          const index = stops.indexOf(document.activeElement as HTMLElement);
          const next = event.shiftKey
            ? (index <= 0 ? stops.length - 1 : index - 1)
            : (index === -1 || index === stops.length - 1 ? 0 : index + 1);
          event.preventDefault();
          stops[next].focus();
        }}
      >
        <h2 className="break-words text-sm font-semibold text-gray-900 dark:text-gray-100">
          Delete {expected}?
        </h2>
        <div className="mt-2 space-y-2 text-sm text-gray-600 dark:text-gray-300">
          <p>
            This erases the project from the APAS server: its tabs, the conversations shown in APAS, its history,
            usage, sharing, invitations and policy. This cannot be undone.
          </p>
          <p>
            The folder and its files stay on every machine. Each of those machines keeps the folder registered, so
            it stays in the lists as a registered folder, and opening it there creates a new, empty project.
          </p>
          <p>
            {stoppedFirstHosts.length > 0
              ? `Agents running in the project are stopped first, on ${hostList(stoppedFirstHosts)}.`
              : "Agents running in the project are stopped first."}
          </p>
          {offlineTabOwnerHosts.length > 0 && <p>{offlineOwnersText(offlineTabOwnerHosts)}</p>}
        </div>
        <label className="mt-3 block text-sm text-gray-700 dark:text-gray-200">
          <span>
            Type <strong className="break-all font-semibold">{expected}</strong> to confirm
          </span>
          <input
            ref={input}
            type="text"
            aria-label="Project name"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            value={typed}
            disabled={busy}
            onChange={(event) => setTyped(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && matches && !busy) void confirmDeletion();
            }}
            className="mt-1 w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-base text-gray-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-red-500 disabled:opacity-60 dark:border-gray-600 dark:bg-gray-900 dark:text-gray-100 sm:text-sm"
          />
        </label>
        {busy && (
          <p role="status" className="mt-2 text-sm text-gray-600 dark:text-gray-300">
            {phase === "sending"
              ? "Deleting…"
              : "Deleting… The project leaves the lists when the server reports the deletion done."}
          </p>
        )}
        {error && (
          <p role="alert" className="mt-2 text-sm text-red-700 dark:text-red-300">{error}</p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" className={overviewButton} onClick={onClose}>
            {busy ? "Close" : "Cancel"}
          </button>
          <button
            type="button"
            className={dangerSolidButton}
            disabled={!matches || busy}
            onClick={() => void confirmDeletion()}
          >
            {busy ? "Deleting…" : "Delete project"}
          </button>
        </div>
      </div>
    </div>
  );
}
