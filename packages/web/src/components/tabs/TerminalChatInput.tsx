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
  const sendTerminalConversationMessage = useStore((s) => s.sendTerminalConversationMessage);
  const connected = useStore((s) => s.connected);
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  const send = () => {
    const body = text.trim();
    if (!body || !connected) return;

    if (sendTerminalConversationMessage(paneId, body).success) setText("");
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
          placeholder="Message the agent…"
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
    </div>
  );
}
