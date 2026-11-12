import type { MachineWithProjects, SessionPaneSummary } from "@/lib/store";
import { paneNotRunning, type SessionInstance } from "@/lib/projectInstances";

interface PaneStatusDotsProps {
  panes?: readonly SessionPaneSummary[];
  isActive: boolean;
  /** The project's attached instances. A pane whose machine none of them is
   *  on is not running, however long the project stays active elsewhere. */
  instances?: SessionInstance[];
  /** Daemon inventory, to recognise a sole instance without multi-machine
   *  support, which runs every untagged pane. */
  machines?: MachineWithProjects[];
}

export function PaneStatusDots({ panes, isActive, instances, machines }: PaneStatusDotsProps) {
  if (!panes?.length) return null;

  return (
    <span
      role="group"
      aria-label="Pane statuses"
      className="flex min-w-0 flex-wrap items-center gap-1"
    >
      {panes.map((pane) => {
        const host = pane.hostname?.trim();
        const status = !isActive
          ? "Not running"
          : paneNotRunning(pane, instances, machines)
            // Another machine keeps the project active; this pane's does not
            // run it, so it is not idle, whatever its last status said.
            ? host ? `Not running on ${host}` : "Not running"
            : pane.awaiting_answer
              ? "Pending answer"
              : pane.is_working
                ? "Working"
                : "Idle";
        const name = pane.label?.trim()
          || `${pane.kind === "terminal" ? "Terminal" : "Pane"} ${pane.pane_id}`;
        const label = `${name}: ${status}`;
        return (
          <span
            key={pane.pane_id}
            role="img"
            aria-label={label}
            title={label}
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${
              status === "Pending answer"
                ? "bg-red-500"
                : status === "Working"
                  ? "bg-blue-500"
                  : "bg-gray-400"
            }`}
          />
        );
      })}
    </span>
  );
}
