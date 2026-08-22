"use client";

import { useStore } from "@/lib/store";

const TERMINAL_KEYS = [
  { label: "Esc", ariaLabel: "Send Escape", data: "\x1b" },
  { label: "↑", ariaLabel: "Send Arrow Up", data: "\x1b[A" },
  { label: "↓", ariaLabel: "Send Arrow Down", data: "\x1b[B" },
  { label: "Enter", ariaLabel: "Send Enter", data: "\r" },
  { label: "Tab", ariaLabel: "Send Tab", data: "\t" },
  { label: "Ctrl-C", ariaLabel: "Send Ctrl-C", data: "\x03" },
] as const;

export function MobileTerminalKeyBar({
  paneId,
  connected,
}: {
  paneId: number;
  connected: boolean;
}) {
  const sendTerminalInput = useStore((state) => state.sendTerminalInput);

  return (
    <div
      role="toolbar"
      aria-label="Terminal keys"
      className="flex shrink-0 gap-1.5 overflow-x-auto border-t border-neutral-800 bg-neutral-950 px-2 py-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]"
    >
      {TERMINAL_KEYS.map((key) => (
        <button
          key={key.ariaLabel}
          type="button"
          aria-label={key.ariaLabel}
          disabled={!connected}
          // Keep xterm focused so tapping an accessory key does not dismiss a
          // phone's software keyboard while the user filters a provider menu.
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => sendTerminalInput(paneId, key.data)}
          className="min-h-10 min-w-12 shrink-0 touch-manipulation rounded-lg border border-neutral-700 bg-neutral-900 px-3 font-mono text-sm font-semibold text-neutral-100 active:bg-neutral-700 disabled:opacity-40"
        >
          {key.label}
        </button>
      ))}
    </div>
  );
}
