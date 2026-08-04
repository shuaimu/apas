"use client";

import { useEffect, useRef, useState } from "react";
import { useStore } from "@/lib/store";

/**
 * Text input for a terminal pane's conversation view.
 *
 * This exists mainly for phones. An xterm TUI on a small screen is close to
 * unusable — no modifier keys, tiny hit targets, and scrolling that fights the
 * page — so the conversation view plus this box is the practical way to drive
 * an agent from a phone.
 *
 * There is no MCP involved and there could not be: MCP is agent-pull, so a
 * server can answer a tool call but can never push a turn into a running
 * conversation. Text goes where a keystroke goes — straight into the pty, via
 * the same `TerminalInput` path the xterm view uses. The TUI cannot tell the
 * difference between this and typing.
 *
 * **It is sent blind**, which is the real caveat. Unlike the terminal view you
 * cannot see the TUI's state first, so text arriving while the agent is
 * mid-turn may queue or be swallowed, and text arriving while a menu or prompt
 * is open is interpreted as commands rather than a message.
 */
export function TerminalChatInput({ paneId }: { paneId: number }) {
  const [text, setText] = useState("");
  const [justSent, setJustSent] = useState(false);
  const sendTerminalInput = useStore((s) => s.sendTerminalInput);
  const connected = useStore((s) => s.connected);
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  useEffect(() => {
    if (!justSent) return;
    const t = setTimeout(() => setJustSent(false), 1800);
    return () => clearTimeout(t);
  }, [justSent]);

  const send = () => {
    const body = text.trim();
    if (!body || !connected) return;

    if (body.includes("\n")) {
      // Multi-line goes as a bracketed paste, which is how a real paste
      // arrives: the TUI takes it as one atomic insert instead of treating
      // each newline as "submit", which would fire the first line as a whole
      // message and leave the rest as a second one.
      //
      // Only for multi-line: if a TUI has not enabled bracketed paste
      // (DECSET 2004) the wrapper is interpreted as literal keystrokes, so the
      // common single-line case takes the boring path and cannot be corrupted
      // by it.
      sendTerminalInput(paneId, `\x1b[200~${body}\x1b[201~`);
    } else {
      sendTerminalInput(paneId, body);
    }
    // Separate carriage return so the TUI sees a deliberate submit after the
    // text has landed, rather than a newline embedded in the paste.
    sendTerminalInput(paneId, "\r");

    setText("");
    setJustSent(true);
  };

  return (
    <div className="border-t border-gray-200 bg-white px-3 py-2 dark:border-gray-700 dark:bg-gray-900">
      <div className="flex items-end gap-2">
        <textarea
          ref={ref}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            // Shift+Enter for a newline matches every chat box; plain Enter
            // sends. On a phone there is no Shift, which is why the multi-line
            // path has to survive being pasted rather than typed.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          rows={1}
          placeholder={connected ? "Message the agent…" : "Disconnected"}
          disabled={!connected}
          enterKeyHint="send"
          autoCapitalize="sentences"
          className="min-h-[40px] flex-1 resize-none rounded border border-gray-300 px-3 py-2 text-base outline-none focus:border-gray-500 disabled:opacity-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
        />
        <button
          type="button"
          onClick={send}
          disabled={!connected || !text.trim()}
          // Deliberately a large tap target: this is the phone path.
          className="h-10 rounded bg-gray-700 px-4 text-sm font-medium text-white transition-colors hover:bg-gray-600 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-gray-600 dark:hover:bg-gray-500"
        >
          Send
        </button>
      </div>
      <p className="mt-1 text-[11px] text-gray-500 dark:text-gray-400">
        {justSent
          ? "Sent to the terminal — the reply appears here once the transcript is read."
          : "Typed straight into the terminal. Switch to Terminal view to see its live state."}
      </p>
    </div>
  );
}
