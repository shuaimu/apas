"use client";

/**
 * v3 — chat directly with the Manager pane (interactive).
 *
 * Previously a "secretary" passthrough that appended your notes to
 * manager-directives.jsonl for a deadloop manager to read. v3 promotes
 * the user-facing layer to a real agent: the Manager IS interactive,
 * receives messages via the standard sendMessageToPane wire, and replies
 * in real time. The Tech Lead pane is the autonomous deadloop sibling.
 *
 * Conversation rendering:
 * - User messages and Manager replies BOTH come from the same source —
 *   the Manager pane's chat history (paneMessages[paneKey(managerId)]).
 * - We render user-role messages on the right (violet) and
 *   assistant-role text on the left (gray). Tool calls are filtered
 *   out so the conversation stays clean — the raw stream lives in the
 *   right column (TechLeadStream) when a Tech Lead is running.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { Send } from "lucide-react";
import { useStore, type Message, paneKey } from "@/lib/store";

const EMPTY_MESSAGES: Message[] = [];

function isManagerPane(role: string | undefined): boolean {
  if (!role) return false;
  const lower = role.toLowerCase();
  return lower.includes("manager") && !lower.includes("tech lead");
}

export function ManagerChatPanel() {
  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const showToast = useStore((s) => s.showToast);
  const paneConfigs = useStore((s) => s.paneConfigs);
  const managerPane = useMemo(
    () => paneConfigs.find((p) => isManagerPane(p.role) && p.mode === "interactive"),
    [paneConfigs],
  );
  const managerMessages = useStore((s) =>
    managerPane ? s.paneMessages[paneKey(managerPane.pane_id)] ?? EMPTY_MESSAGES : EMPTY_MESSAGES,
  );

  const [draft, setDraft] = useState("");

  // Filter to user + text-only assistant messages so tool calls /
  // tool_results don't pollute the human-facing conversation.
  const visible = useMemo(
    () =>
      managerMessages.filter(
        (m) =>
          m.role === "user" ||
          (m.role === "assistant" &&
            (m.outputType?.type === "text" || m.outputType === undefined)),
      ),
    [managerMessages],
  );

  // Auto-scroll the conversation to the bottom when new turns arrive,
  // but only when the user is parked at the bottom.
  const scrollRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const handleScroll = () => {
    const c = scrollRef.current;
    if (!c) return;
    shouldAutoScroll.current =
      c.scrollHeight - c.scrollTop - c.clientHeight <= 80;
  };
  useEffect(() => {
    const c = scrollRef.current;
    if (!c) return;
    if (shouldAutoScroll.current) c.scrollTop = c.scrollHeight;
  }, [visible.length]);

  const handleSend = () => {
    const text = draft.trim();
    if (!text) return;
    if (!managerPane) {
      showToast(
        "No Manager pane running. Click Start Manager above to spawn one.",
        "error",
      );
      return;
    }
    const result = sendMessageToPane(text, managerPane.pane_id);
    if (result.success) {
      setDraft("");
    } else {
      showToast(result.error ?? "Failed to reach Manager", "error");
    }
  };

  const handleKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex h-full min-h-[60vh] flex-col rounded border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      <div className="border-b border-gray-200 px-3 py-2 text-xs text-gray-600 dark:border-gray-700 dark:text-gray-400">
        {managerPane ? (
          <>
            Chat with your <strong>Manager</strong> — the user-facing role. The
            Manager keeps <span className="font-mono">project_goal.md</span> in
            sync with this conversation and delegates orchestration to the Tech
            Lead.
          </>
        ) : (
          <>
            No Manager pane yet. Click <strong>Start Manager</strong> above to
            spawn one — they&apos;ll be your point of contact for the team.
          </>
        )}
      </div>

      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 space-y-3 overflow-y-auto px-3 py-3"
      >
        {visible.length === 0 ? (
          <p className="text-center text-xs italic text-gray-400">
            {managerPane
              ? "No conversation yet. Say hi to your Manager below."
              : "Spawn a Manager and start chatting."}
          </p>
        ) : (
          visible.map((m) =>
            m.role === "user" ? (
              <UserBubble key={m.id} ts={m.timestamp} text={m.content} />
            ) : (
              <ManagerBubble key={m.id} ts={m.timestamp} content={m.content} />
            ),
          )
        )}
      </div>

      <div className="border-t border-gray-200 p-3 dark:border-gray-700">
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKey}
          rows={3}
          placeholder={
            managerPane
              ? "Talk to your Manager (Cmd-Enter to send)"
              : "Spawn a Manager first…"
          }
          disabled={!managerPane}
          className="w-full rounded border border-gray-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 disabled:opacity-50 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
        <div className="mt-2 flex items-center justify-end">
          <button
            type="button"
            onClick={handleSend}
            disabled={!draft.trim() || !managerPane}
            className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Send className="h-3.5 w-3.5" /> Send
          </button>
        </div>
      </div>
    </div>
  );
}

function formatTime(d: Date): string {
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function UserBubble({ ts, text }: { ts: Date; text: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-lg bg-violet-600 px-3 py-2 text-sm text-white shadow-sm">
        <div className="whitespace-pre-wrap break-words">{text}</div>
        <div className="mt-1 text-right text-[10px] text-violet-200">
          you · {formatTime(ts)}
        </div>
      </div>
    </div>
  );
}

function ManagerBubble({ ts, content }: { ts: Date; content: string }) {
  return (
    <div className="flex justify-start">
      <div className="max-w-[85%] rounded-lg border border-gray-200 bg-gray-50 px-3 py-2 text-sm text-gray-900 shadow-sm dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100">
        <div className="mb-0.5 text-[10px] font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
          manager · {formatTime(ts)}
        </div>
        <div className="whitespace-pre-wrap break-words">{content}</div>
      </div>
    </div>
  );
}
