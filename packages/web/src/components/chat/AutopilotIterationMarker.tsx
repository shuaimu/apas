"use client";

import { Repeat } from "lucide-react";
import {
  autopilotIterationMarkerText,
  type AutopilotIterationMarker as IterationMarker,
} from "@/lib/autopilot";

function formatTimestamp(date: Date): string {
  const isToday = date.toDateString() === new Date().toDateString();
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return isToday
    ? time
    : `${date.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

/**
 * Where one autopilot iteration starts in a tab's conversation. Autopilot
 * submitted the prompt, not a person, so this is a line of its own and never
 * a user bubble; and not the agent's words either, so it carries no agent
 * avatar. The first marker of an autopilot, and the first after each resume,
 * carries the prompt, which opens behind the line.
 */
export function AutopilotIterationMarker({
  marker,
  timestamp,
}: {
  marker: IterationMarker;
  timestamp?: Date;
}) {
  const text = autopilotIterationMarkerText(marker);
  return (
    <div
      data-testid="autopilot-iteration-marker"
      className="min-w-0 border-l-2 border-indigo-300 pl-3 text-xs text-gray-600 dark:border-indigo-700 dark:text-gray-300"
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
        <Repeat aria-hidden="true" className="h-3.5 w-3.5 flex-shrink-0 text-indigo-500" />
        <span className="font-medium">{text}</span>
        {timestamp && <span className="text-gray-400">{formatTimestamp(timestamp)}</span>}
      </div>
      {marker.prompt && (
        <details className="mt-1">
          <summary className="cursor-pointer select-none text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200">
            Prompt
          </summary>
          <p className="mt-1 whitespace-pre-wrap break-words rounded bg-gray-100 px-2 py-1.5 text-sm text-gray-800 dark:bg-gray-800 dark:text-gray-200">
            {marker.prompt}
          </p>
        </details>
      )}
    </div>
  );
}
