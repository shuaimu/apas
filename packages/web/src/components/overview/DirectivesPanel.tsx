"use client";

/**
 * Manager v2c — conversation view of the user↔manager channel.
 *
 * The user side: every directive sent in this tab is appended to
 * `manager-directives.jsonl` AND mirrored locally so it renders as a
 * "user" bubble immediately.
 *
 * The manager side: text-only assistant messages from the manager
 * pane's chat history are pulled in and interleaved by timestamp. We
 * skip tool_use / tool_result so the conversation doesn't drown in
 * "Reading manager-directives.jsonl" noise — the raw iteration stream
 * is still available on the right column.
 *
 * Persistence: directives sent in earlier sessions don't appear here
 * yet (no server→web echo of the on-disk file). For now this is good
 * enough for "what did I just tell the manager + did it reply".
 */
import { useMemo, useRef, useState, useEffect } from "react";
import { Send } from "lucide-react";
import { useStore, type Message, paneKey } from "@/lib/store";

interface SentDirective {
  id: string;
  ts: Date;
  text: string;
}

type Turn =
  | { kind: "directive"; id: string; ts: Date; text: string }
  | { kind: "manager"; id: string; ts: Date; content: string };

export function DirectivesPanel() {
  const addManagerDirective = useStore((s) => s.addManagerDirective);
  const showToast = useStore((s) => s.showToast);
  const paneConfigs = useStore((s) => s.paneConfigs);
  const managerPane = useMemo(
    () =>
      paneConfigs.find((p) =>
        (p.role ?? "").toLowerCase().includes("manager"),
      ),
    [paneConfigs],
  );
  // Stable empty reference so the selector doesn't return a fresh array
  // every call — same React #185 trap that hit ManagerStream.
  const managerMessages = useStore((s) =>
    managerPane ? s.paneMessages[paneKey(managerPane.pane_id)] ?? EMPTY_MESSAGES : EMPTY_MESSAGES,
  );

  const [directive, setDirective] = useState("");
  const [sent, setSent] = useState<SentDirective[]>([]);

  // Filter manager messages to text-only assistant replies and interleave
  // with the directives by timestamp. Tool calls live on the right column
  // (ManagerStream) — we keep this view focused on the actual back-and-forth.
  const turns = useMemo<Turn[]>(() => {
    const managerTurns: Turn[] = managerMessages
      .filter(
        (m) =>
          m.role === "assistant" &&
          (m.outputType?.type === "text" || m.outputType === undefined),
      )
      .map((m) => ({
        kind: "manager",
        id: m.id,
        ts: m.timestamp,
        content: m.content,
      }));
    const directiveTurns: Turn[] = sent.map((d) => ({
      kind: "directive",
      id: d.id,
      ts: d.ts,
      text: d.text,
    }));
    return [...managerTurns, ...directiveTurns].sort(
      (a, b) => a.ts.getTime() - b.ts.getTime(),
    );
  }, [managerMessages, sent]);

  // Auto-scroll the conversation to the bottom when new turns arrive, but
  // only when the user is already parked at the bottom.
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
  }, [turns.length]);

  const handleSend = () => {
    const text = directive.trim();
    if (!text) return;
    addManagerDirective(text);
    setSent((prev) => [
      ...prev,
      {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        ts: new Date(),
        text,
      },
    ]);
    setDirective("");
    // No toast — the directive appearing as a user bubble is its own
    // feedback, and toasts on every send is noisy when chatting.
    void showToast;
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
        Conversation with the manager. Your directives append to{" "}
        <span className="font-mono">manager-directives.jsonl</span>; the
        manager&apos;s text replies are pulled from its iteration stream and
        interleaved here.
      </div>

      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 space-y-3 overflow-y-auto px-3 py-3"
      >
        {turns.length === 0 ? (
          <p className="text-center text-xs italic text-gray-400">
            No conversation yet. Type a directive below to start.
          </p>
        ) : (
          turns.map((t) =>
            t.kind === "directive" ? (
              <DirectiveBubble key={t.id} ts={t.ts} text={t.text} />
            ) : (
              <ManagerBubble key={t.id} ts={t.ts} content={t.content} />
            ),
          )
        )}
      </div>

      <div className="border-t border-gray-200 p-3 dark:border-gray-700">
        <textarea
          value={directive}
          onChange={(e) => setDirective(e.target.value)}
          onKeyDown={handleKey}
          rows={3}
          placeholder="Strategy nudge / correction / question (Cmd-Enter to send)"
          className="w-full rounded border border-gray-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <p className="text-[11px] text-gray-500 dark:text-gray-400">
            Directives are absorbed at the next loop boundary, not mid-action.
          </p>
          <button
            type="button"
            onClick={handleSend}
            disabled={!directive.trim()}
            className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Send className="h-3.5 w-3.5" /> Send
          </button>
        </div>
      </div>
    </div>
  );
}

const EMPTY_MESSAGES: Message[] = [];

function formatTime(d: Date): string {
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function DirectiveBubble({ ts, text }: { ts: Date; text: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-lg bg-violet-600 px-3 py-2 text-sm text-white shadow-sm">
        <div className="whitespace-pre-wrap break-words">{text}</div>
        <div className="mt-1 text-right text-[10px] text-violet-200">
          {formatTime(ts)}
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
