"use client";

import type { ProjectOpenNoticeLine } from "@/lib/projectOpenState";

/**
 * The machines that are not in line with the project's open state, one line
 * each: a machine that owns tabs of an open project (or of one nobody has
 * recorded that runs elsewhere) and is not running it, with the server's
 * reason, or a machine that still runs a closed project. Nothing is shown
 * when every machine follows.
 *
 * It has no button on purpose. One machine alone is never started or stopped
 * from here; the project's Open and Close in the tab bar are the controls,
 * and they appear ("Open again", "Close again") whenever they can change
 * what this says.
 */
export function ProjectOpenNotice({ lines }: { lines: ProjectOpenNoticeLine[] }) {
  if (lines.length === 0) return null;
  // A machine that is starting, or that starts by itself when it reconnects,
  // needs nobody. The others do.
  const needsSomeone = lines.some((line) =>
    line.kind === "not_started" || line.kind === "offline" || line.kind === "still_running");
  return (
    <div
      role="status"
      aria-label="Machines not following this project"
      className={`flex-shrink-0 border-b px-3 py-2 text-sm ${
        needsSomeone
          ? "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-200"
          : "border-gray-200 bg-gray-50 text-gray-700 dark:border-gray-700 dark:bg-gray-800/50 dark:text-gray-300"
      }`}
    >
      <ul className="space-y-1">
        {lines.map((line) => (
          <li key={line.key} data-notice-kind={line.kind} className="break-words">
            {line.text}
            {line.reason ? ` ${line.reason}` : ""}
          </li>
        ))}
      </ul>
    </div>
  );
}
