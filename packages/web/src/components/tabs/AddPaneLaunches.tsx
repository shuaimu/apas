"use client";

import { AlertTriangle, LoaderCircle, X } from "lucide-react";
import type { AddPaneRequest } from "@/lib/store";
import { launchStatus } from "@/lib/tabHosts";

const DESKTOP_TONES = {
  progress: "border-blue-200 bg-blue-50 text-blue-800 dark:border-blue-900 dark:bg-blue-950/40 dark:text-blue-200",
  error: "border-red-200 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300",
  warning: "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200",
} as const;

const MOBILE_TONES = {
  progress: "border-[#dedee7] bg-white text-[#45454f] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#c7c7d1]",
  error: "border-red-300 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300",
  warning: "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200",
} as const;

/**
 * One project's new-tab launches, each correlated by its own request: waiting
 * for the server, starting the chosen machine, cloning onto it, opening, or
 * finished without a tab. A launch whose answer was lost is shown as
 * unconfirmed. Nothing here sends anything again: a resent launch could
 * create a second tab, so the only action is to dismiss a final one.
 */
export function AddPaneLaunches({
  launches,
  onDismiss,
  variant = "desktop",
}: {
  launches: AddPaneRequest[];
  onDismiss: (requestId: string) => void;
  variant?: "desktop" | "mobile";
}) {
  if (launches.length === 0) return null;
  const mobile = variant === "mobile";
  const tones = mobile ? MOBILE_TONES : DESKTOP_TONES;
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label="New tab launches"
      className={mobile ? "mx-4 mt-3 flex shrink-0 flex-col gap-1.5" : "flex flex-shrink-0 flex-col gap-px border-b border-gray-200 dark:border-gray-700"}
    >
      {launches.map((request) => {
        const status = launchStatus(request);
        return (
          <div
            key={request.requestId}
            data-launch-status={request.status}
            className={`flex items-start gap-2 border text-xs ${mobile ? "rounded-xl px-3 py-2" : "border-x-0 border-t-0 px-3 py-1.5"} ${tones[status.tone]}`}
          >
            {status.inFlight
              ? <LoaderCircle aria-hidden="true" className="mt-px h-3.5 w-3.5 flex-shrink-0 animate-spin" />
              : <AlertTriangle aria-hidden="true" className="mt-px h-3.5 w-3.5 flex-shrink-0" />}
            <span className="min-w-0 flex-1 break-words">
              <span className="font-semibold">{status.title}</span>
              {" — "}
              {status.detail}
            </span>
            {!status.inFlight && (
              <button
                type="button"
                onClick={() => onDismiss(request.requestId)}
                aria-label={`Dismiss ${status.title}`}
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
