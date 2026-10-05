import type { SessionPaneSummary } from "@/lib/store";

interface PaneStatusDotsProps {
  panes?: readonly SessionPaneSummary[];
  isActive: boolean;
}

export function PaneStatusDots({ panes, isActive }: PaneStatusDotsProps) {
  if (!panes?.length) return null;

  return (
    <span
      role="group"
      aria-label="Pane statuses"
      className="flex min-w-0 flex-wrap items-center gap-1"
    >
      {panes.map((pane) => {
        const status = !isActive
          ? "Not running"
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
