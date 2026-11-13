"use client";

import { AlertTriangle, LoaderCircle, Repeat, X } from "lucide-react";
import { useStore } from "@/lib/store";
import {
  autopilotConditionIsOn,
  autopilotConditionText,
  autopilotProgressText,
  autopilotRequestNotice,
  ompModeNotices,
  paneAutopilotKey,
  type AutopilotRequest,
} from "@/lib/autopilot";

/**
 * The second status line of a tab: what its autopilot is doing, beside (never
 * instead of) Working, Pending answer or Idle; and OMP's own goal and loop
 * modes while OMP reports them, with how to stop them in the terminal. APAS
 * shows those modes and does not switch them.
 */
export function AutopilotStatusLine({ sessionId, paneId }: { sessionId: string; paneId: number }) {
  const entry = useStore((state) => state.paneAutopilot[paneAutopilotKey(sessionId, paneId)]);
  // A current autopilot is shown only for a tab that reports one now.
  const autopilot = entry?.support === "supported" ? entry.state?.autopilot : undefined;
  const modes = ompModeNotices(entry?.state?.omp_modes);
  if (!autopilot && modes.length === 0) return null;
  const on = autopilot ? autopilotConditionIsOn(autopilot.condition.kind) : false;
  return (
    <div className="flex-shrink-0 space-y-1.5 border-t border-gray-200 bg-gray-50 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-800/40">
      {autopilot && (
        <div
          role="status"
          data-testid="autopilot-status-line"
          className={`flex flex-wrap items-center gap-x-2 gap-y-0.5 ${on ? "text-indigo-700 dark:text-indigo-300" : "text-amber-800 dark:text-amber-200"}`}
        >
          <Repeat aria-hidden="true" className="h-3.5 w-3.5 flex-shrink-0" />
          <span>Autopilot · {autopilotConditionText(autopilot.condition, autopilot.next_iteration_at)}</span>
          <span className="text-xs text-gray-500 dark:text-gray-400">
            {autopilotProgressText(autopilot.iterations, autopilot.spec.iteration_limit)}
          </span>
        </div>
      )}
      {modes.map((notice) => (
        <div
          key={notice.mode}
          role="note"
          data-testid={`omp-mode-${notice.mode}`}
          className="text-xs text-gray-700 dark:text-gray-200"
        >
          <p className="font-medium">{notice.title}</p>
          {notice.facts.map((fact) => (
            <p key={fact} className="break-words text-gray-600 dark:text-gray-300">{fact}</p>
          ))}
          <p className="text-gray-600 dark:text-gray-300">{notice.howToStop}</p>
          {notice.warning && <p className="text-amber-700 dark:text-amber-300">{notice.warning}</p>}
        </div>
      ))}
    </div>
  );
}

const TONES = {
  progress: "border-blue-200 bg-blue-50 text-blue-800 dark:border-blue-900 dark:bg-blue-950/40 dark:text-blue-200",
  error: "border-red-200 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300",
  warning: "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200",
} as const;

// The mobile-browser screen's palette, as its new-tab launches use it.
const MOBILE_TONES = {
  progress: "border-[#dedee7] bg-white text-[#45454f] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#c7c7d1]",
  error: "border-red-300 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300",
  warning: "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200",
} as const;

/**
 * One project's autopilot requests: waiting for their receipt, accepted until
 * the tab reports, or refused or not confirmed. A request that was not
 * confirmed stays here until it is dismissed. Nothing here sends anything
 * again: a pause or stop stays in force on the tab's machine, and a second
 * start could start a second autopilot once the first has ended.
 */
export function AutopilotRequestNotices({
  requests,
  tabLabel,
  onDismiss,
  variant = "desktop",
}: {
  requests: AutopilotRequest[];
  tabLabel: (paneId: number) => string | undefined;
  onDismiss: (requestId: string) => void;
  variant?: "desktop" | "mobile";
}) {
  if (requests.length === 0) return null;
  const mobile = variant === "mobile";
  const tones = mobile ? MOBILE_TONES : TONES;
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label="Autopilot requests"
      className={mobile ? "mx-4 mt-3 flex shrink-0 flex-col gap-1.5" : "flex flex-shrink-0 flex-col gap-px border-b border-gray-200 dark:border-gray-700"}
    >
      {requests.map((request) => {
        const notice = autopilotRequestNotice(request, tabLabel(request.paneId));
        return (
          <div
            key={request.requestId}
            data-autopilot-request={request.status}
            className={`flex items-start gap-2 border text-xs ${mobile ? "rounded-xl px-3 py-2" : "border-x-0 border-t-0 px-3 py-1.5"} ${tones[notice.tone]}`}
          >
            {notice.inFlight
              ? <LoaderCircle aria-hidden="true" className="mt-px h-3.5 w-3.5 flex-shrink-0 animate-spin" />
              : <AlertTriangle aria-hidden="true" className="mt-px h-3.5 w-3.5 flex-shrink-0" />}
            <span className="min-w-0 flex-1 break-words">
              <span className="font-semibold">{notice.title}</span>
              {" — "}
              {notice.detail}
            </span>
            {!notice.inFlight && (
              <button
                type="button"
                onClick={() => onDismiss(request.requestId)}
                aria-label={`Dismiss ${notice.title}`}
                className={`flex flex-shrink-0 items-center justify-center rounded opacity-70 hover:opacity-100 ${mobile ? "-my-1 -mr-1 h-8 w-8" : "h-5 w-5"}`}
              >
                <X aria-hidden="true" className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
