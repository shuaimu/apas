"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Repeat } from "lucide-react";
import type { AutopilotSpec } from "@apas/protocol";
import { useStore } from "@/lib/store";
import {
  autopilotActionDetail,
  autopilotActionLabel,
  autopilotActionRequest,
  autopilotControlModel,
  autopilotDetailRows,
  autopilotProgressText,
  autopilotStartInFlight,
  AUTOPILOT_START_IN_FLIGHT_TEXT,
  formatAutopilotTime,
  paneAutopilotKey,
  type AutopilotRequestKind,
} from "@/lib/autopilot";
import { AutopilotStartDialog } from "./AutopilotStartDialog";

/**
 * The pane toolbar's autopilot control for one OMP terminal tab. It shows the
 * condition the tab reports and never one of its own: a request's receipt
 * only says the tab took it. With no autopilot (or an ended one) it opens the
 * start dialog; with one it opens a menu of the actions that apply and the
 * details. Where autopilot cannot be used the control stays, disabled, with
 * the reason. It touches neither the terminal nor the selected view.
 */
export function AutopilotControl({
  sessionId,
  paneId,
  tabLabel,
  hostname,
  tabRefusal,
  sharedCheckout,
}: {
  sessionId: string;
  paneId: number;
  tabLabel: string;
  /** The tab's machine. */
  hostname?: string;
  /** Why the tab cannot be used now: its machine is not running the project,
   *  or this viewer has no runtime access there. */
  tabRefusal?: string;
  sharedCheckout: boolean;
}) {
  const entry = useStore((state) => state.paneAutopilot[paneAutopilotKey(sessionId, paneId)]);
  const connected = useStore((state) => state.connected);
  const requests = useStore((state) => state.autopilotRequests);
  const controlPaneAutopilot = useStore((state) => state.controlPaneAutopilot);
  const [menuOpen, setMenuOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const model = useMemo(
    () => autopilotControlModel({ entry, hostname, connected, tabRefusal }),
    [connected, entry, hostname, tabRefusal],
  );
  // A start that is on its way: offering another would only be refused.
  const startInFlight = useMemo(
    () => autopilotStartInFlight(requests, sessionId, paneId),
    [paneId, requests, sessionId],
  );
  // With a current autopilot, or with a state that is not reported, the
  // control opens the menu: there is something to read or to stop.
  const hasMenu = Boolean(model.autopilot) || model.notReported;

  // Another tab was selected: its control starts closed.
  useEffect(() => {
    setMenuOpen(false);
    setDetailsOpen(false);
    setDialogOpen(false);
    setSendError(null);
  }, [paneId, sessionId]);

  // The autopilot ended while its menu was open: the menu goes with it, and
  // does not come back open by itself with the next autopilot.
  useEffect(() => {
    if (!hasMenu) {
      setMenuOpen(false);
      setDetailsOpen(false);
    }
  }, [hasMenu]);

  useEffect(() => {
    if (!menuOpen) return;
    const handleClick = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setMenuOpen(false);
    };
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKey);
    };
  }, [menuOpen]);

  const send = (kind: Exclude<AutopilotRequestKind, "start">) => {
    const action = autopilotActionRequest(kind, model.autopilot?.autopilot_id);
    if (!action) return;
    const result = controlPaneAutopilot(sessionId, paneId, action);
    if (result.success) {
      setSendError(null);
      setMenuOpen(false);
    } else {
      setSendError(result.error ?? "The request could not be sent.");
    }
  };

  const start = (spec: AutopilotSpec) => {
    const result = controlPaneAutopilot(sessionId, paneId, { kind: "start", spec: { ...spec } });
    if (result.success) setDialogOpen(false);
    return result;
  };

  const startDisabledReason = hasMenu
    ? undefined
    : model.disabledReason ?? (startInFlight ? AUTOPILOT_START_IN_FLIGHT_TEXT : undefined);
  const endedAt = model.endedAt ? formatAutopilotTime(model.endedAt) : null;
  // Only what the tab reports. A start that is on its way changes nothing
  // here: the request list says what became of it, and the condition changes
  // when the tab reports one.
  const status = model.statusText
    ? `${model.statusText}${endedAt ? ` · ${endedAt}` : ""}`
    : undefined;
  const tone = model.phase === "on"
    ? "border-indigo-400 text-indigo-700 dark:border-indigo-600 dark:text-indigo-300"
    : model.phase === "paused"
      ? "border-amber-400 text-amber-800 dark:border-amber-600 dark:text-amber-300"
      : "border-gray-300 text-gray-700 dark:border-gray-600 dark:text-gray-200";

  return (
    <div ref={rootRef} data-testid="autopilot-control" className="relative flex min-w-0 items-center gap-2">
      <button
        type="button"
        disabled={Boolean(startDisabledReason)}
        title={startDisabledReason ?? (hasMenu ? "Autopilot: what this tab reports, and its actions" : "Start autopilot on this tab")}
        aria-haspopup={hasMenu ? "true" : "dialog"}
        aria-expanded={hasMenu ? menuOpen : undefined}
        onClick={() => {
          setSendError(null);
          if (hasMenu) setMenuOpen((open) => !open);
          else setDialogOpen(true);
        }}
        className={`flex flex-shrink-0 items-center gap-1 rounded border bg-white px-2 py-0.5 text-[11px] font-medium transition-colors hover:bg-gray-100 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-gray-800 dark:hover:bg-gray-700 ${tone}`}
      >
        <Repeat aria-hidden="true" className="h-3 w-3" />
        Autopilot
        {hasMenu && <ChevronDown aria-hidden="true" className="h-3 w-3" />}
      </button>
      {/* Not a live region: the tab's second status line already announces
          the condition, and it changes with every iteration. */}
      {status && (
        <span
          data-testid="autopilot-condition"
          title={status}
          className="min-w-0 max-w-[18rem] truncate text-[11px] text-gray-600 dark:text-gray-300"
        >
          {status}
        </span>
      )}
      {/* Disabled, not hidden: the reason is what tells the person what the
          tab needs, or that a start is already on its way. */}
      {startDisabledReason && (
        <span
          role="note"
          title={startDisabledReason}
          className="min-w-0 max-w-[22rem] truncate text-[11px] text-amber-700 dark:text-amber-300"
        >
          {startDisabledReason}
        </span>
      )}

      {menuOpen && hasMenu && (
        <div
          role="group"
          aria-label="Autopilot"
          className="absolute left-0 top-full z-50 mt-1 max-h-[70vh] w-80 max-w-[calc(100vw-24px)] overflow-y-auto rounded-lg border border-gray-200 bg-white p-1.5 shadow-xl dark:border-gray-700 dark:bg-gray-800"
        >
          <p className="px-3 py-1.5 text-xs font-medium text-gray-800 dark:text-gray-100">
            {model.statusText}
            {model.autopilot && (
              <span className="font-normal text-gray-500 dark:text-gray-400">
                {" · "}
                {autopilotProgressText(model.autopilot.iterations, model.autopilot.spec.iteration_limit)}
              </span>
            )}
          </p>
          {model.notice && (
            <p role="note" className="px-3 pb-1.5 text-xs text-gray-600 dark:text-gray-300">{model.notice}</p>
          )}
          {model.disabledReason && model.actions.length > 0 && (
            <p role="note" className="px-3 pb-1.5 text-xs text-amber-700 dark:text-amber-300">
              {model.disabledReason}
            </p>
          )}
          {sendError && (
            <p role="alert" className="px-3 pb-1.5 text-xs text-red-600 dark:text-red-400">{sendError}</p>
          )}
          {model.actions.map((kind) => (
            <button
              key={kind}
              type="button"
              disabled={Boolean(model.disabledReason)}
              title={model.disabledReason}
              onClick={() => send(kind)}
              className={`w-full rounded px-3 py-2 text-left text-sm hover:bg-gray-100 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-transparent dark:hover:bg-gray-700 ${
                kind === "stop_now" ? "text-red-700 dark:text-red-300" : "text-gray-800 dark:text-gray-100"
              }`}
            >
              <span className="font-medium">{autopilotActionLabel(kind)}</span>
              <span className="mt-0.5 block text-xs font-normal text-gray-500 dark:text-gray-400">
                {autopilotActionDetail(kind)}
              </span>
            </button>
          ))}
          {model.autopilot && (
            <>
              <button
                type="button"
                aria-expanded={detailsOpen}
                onClick={() => setDetailsOpen((open) => !open)}
                className="w-full rounded px-3 py-2 text-left text-sm font-medium text-gray-800 hover:bg-gray-100 dark:text-gray-100 dark:hover:bg-gray-700"
              >
                Details
              </button>
              {detailsOpen && (
                <div
                  data-testid="autopilot-details"
                  className="border-t border-gray-200 px-3 py-2 text-xs text-gray-700 dark:border-gray-700 dark:text-gray-200"
                >
                  <p className="mb-1 text-gray-500 dark:text-gray-400">Prompt</p>
                  <p className="mb-2 max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded bg-gray-100 px-2 py-1.5 dark:bg-gray-900">
                    {model.autopilot.spec.prompt}
                  </p>
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
                    {autopilotDetailRows(model.autopilot).map((row) => (
                      <div key={row.label} className="contents">
                        <dt className="text-gray-500 dark:text-gray-400">{row.label}</dt>
                        <dd>{row.value}</dd>
                      </div>
                    ))}
                  </dl>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {dialogOpen && (
        <AutopilotStartDialog
          sessionId={sessionId}
          tabLabel={tabLabel}
          hostname={hostname}
          sharedCheckout={sharedCheckout}
          last={model.lastEnded?.spec}
          onCancel={() => setDialogOpen(false)}
          onStart={start}
        />
      )}
    </div>
  );
}
