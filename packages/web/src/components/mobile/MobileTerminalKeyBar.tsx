"use client";

import { useEffect, useState } from "react";
import { useStore } from "@/lib/store";
import { controlTerminal } from "@/lib/terminalControls";

const TERMINAL_KEYS = [
  { label: "Esc", ariaLabel: "Send Escape", data: "\x1b" },
  { label: "Tab", ariaLabel: "Send Tab", data: "\t" },
  { label: "↑", ariaLabel: "Send Arrow Up", data: "\x1b[A" },
  { label: "↓", ariaLabel: "Send Arrow Down", data: "\x1b[B" },
  { label: "←", ariaLabel: "Send Arrow Left", data: "\x1b[D" },
  { label: "→", ariaLabel: "Send Arrow Right", data: "\x1b[C" },
  { label: "Enter", ariaLabel: "Send Enter", data: "\r" },
  { label: "Ctrl-C", ariaLabel: "Send Ctrl-C", data: "\x03" },
] as const;

const keyClass =
  "min-h-10 min-w-12 shrink-0 touch-manipulation rounded-lg border border-neutral-700 bg-neutral-900 px-3 font-mono text-sm font-semibold text-neutral-100 active:bg-neutral-700 disabled:opacity-40";

export function MobileTerminalKeyBar({
  paneId,
  connected,
}: {
  paneId: number;
  connected: boolean;
}) {
  const sessionId = useStore((state) => state.sessionId);
  const [textOpen, setTextOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [ctrl, setCtrl] = useState(false);
  const draftKey = `apas-terminal-draft:${sessionId}:${paneId}`;
  useEffect(() => {
    try {
      setDraft(sessionStorage.getItem(draftKey) ?? "");
    } catch {
      setDraft("");
    }
    setTextOpen(false);
    setCtrl(false);
  }, [draftKey]);
  const updateDraft = (text: string) => {
    setDraft(text);
    try {
      sessionStorage.setItem(draftKey, text);
    } catch {}
  };
  const key = (data: string) => {
    controlTerminal(paneId, { kind: "key", data });
  };

  return (
    <div className="shrink-0 border-t border-neutral-800 bg-neutral-950 pb-[max(0.25rem,env(safe-area-inset-bottom))]">
      {textOpen && (
        <div className="space-y-2 px-2 pt-2">
          <label
            htmlFor={`terminal-draft-${paneId}`}
            className="text-xs text-neutral-300"
          >
            Write text, then paste it into the terminal. Press Enter when ready.
          </label>
          <textarea
            id={`terminal-draft-${paneId}`}
            aria-label="Terminal text draft"
            value={draft}
            onChange={(event) => updateDraft(event.target.value)}
            rows={3}
            autoFocus
            autoCapitalize="sentences"
            className="block max-h-40 w-full resize-y rounded-lg border border-neutral-700 bg-neutral-900 p-2 text-base text-white"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={!connected || !draft}
              className={keyClass}
              onClick={() => {
                if (controlTerminal(paneId, { kind: "paste", text: draft })) {
                  updateDraft("");
                  setTextOpen(false);
                }
              }}
            >
              Paste into terminal
            </button>
            <button
              type="button"
              className={keyClass}
              onClick={() => setTextOpen(false)}
            >
              Close
            </button>
          </div>
        </div>
      )}
      <div
        role="toolbar"
        aria-label="Terminal keys"
        className="flex gap-1.5 overflow-x-auto px-2 py-2"
      >
        <button
          type="button"
          disabled={!connected}
          className={keyClass}
          onClick={() => controlTerminal(paneId, { kind: "focus" })}
        >
          Keyboard
        </button>
        <button
          type="button"
          disabled={!connected}
          className={keyClass}
          onClick={() => setTextOpen((open) => !open)}
        >
          Text
        </button>
        <button
          type="button"
          disabled={!connected}
          aria-pressed={ctrl}
          className={keyClass}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => setCtrl((value) => !value)}
        >
          Ctrl
        </button>
        {TERMINAL_KEYS.map((item) => (
          <button
            key={item.ariaLabel}
            type="button"
            aria-label={item.ariaLabel}
            disabled={!connected}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => key(item.data)}
            className={keyClass}
          >
            {item.label}
          </button>
        ))}
        <button
          type="button"
          disabled={!connected}
          className={keyClass}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() =>
            controlTerminal(paneId, { kind: "page", direction: -1 })
          }
        >
          PgUp
        </button>
        <button
          type="button"
          disabled={!connected}
          className={keyClass}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() =>
            controlTerminal(paneId, { kind: "page", direction: 1 })
          }
        >
          PgDn
        </button>
      </div>
      {ctrl && (
        <div
          role="toolbar"
          aria-label="Control letters"
          className="flex gap-1 overflow-x-auto px-2 pb-2"
        >
          {"abcdefghijklmnopqrstuvwxyz".split("").map((letter) => (
            <button
              key={letter}
              type="button"
              disabled={!connected}
              className={keyClass}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => {
                key(String.fromCharCode(letter.charCodeAt(0) - 96));
                setCtrl(false);
              }}
            >
              Ctrl-{letter.toUpperCase()}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
