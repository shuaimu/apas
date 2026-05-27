"use client";

/**
 * Manager v2c / naming pass — the "Secretary" panel.
 *
 * Mental model: the manager is the deadloop agent that does the real
 * orchestration. The *secretary* is this UI — a proxy that you talk to,
 * which writes your notes down for the manager to read at its next
 * iteration boundary. The metaphor explains why there's latency: you tell
 * your secretary, your secretary leaves a note on the manager's desk, the
 * manager reads the note when they next come up for air.
 *
 * Under the hood it's still the same `manager-directives.jsonl` channel —
 * the secretary metaphor is purely user-facing naming. Wire / store /
 * file names keep their existing identifiers.
 *
 * Conversation rendering:
 * - Your notes show as violet bubbles on the right ("you").
 * - The manager's text replies (from its iteration stream) show as gray
 *   bubbles on the left ("manager"). Tool calls are filtered out — those
 *   live on the right column (ManagerStream) for the full picture.
 *
 * Persistence: notes sent in earlier sessions don't appear here yet (no
 * server→web echo of the on-disk file). Manager replies do persist since
 * they're stored as pane messages.
 */
import { useMemo, useRef, useState, useEffect } from "react";
import { Send } from "lucide-react";
import { useStore, type Message, paneKey } from "@/lib/store";

interface SentNote {
  id: string;
  ts: Date;
  text: string;
}

type Turn =
  | { kind: "note"; id: string; ts: Date; text: string }
  | { kind: "manager"; id: string; ts: Date; content: string };

export function SecretaryPanel() {
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

  const [note, setNote] = useState("");
  const [sent, setSent] = useState<SentNote[]>([]);

  // Filter manager messages to text-only assistant replies and interleave
  // with the notes by timestamp. Tool calls live on the right column
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
    const noteTurns: Turn[] = sent.map((d) => ({
      kind: "note",
      id: d.id,
      ts: d.ts,
      text: d.text,
    }));
    return [...managerTurns, ...noteTurns].sort(
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
    const text = note.trim();
    if (!text) return;
    // Same wire underneath — the secretary still appends to
    // manager-directives.jsonl for the manager to read.
    addManagerDirective(text);
    setSent((prev) => [
      ...prev,
      {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        ts: new Date(),
        text,
      },
    ]);
    setNote("");
    // No toast — the note appearing as a user bubble is its own feedback,
    // and toasts on every send is noisy when chatting.
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
        Talk to your <strong>secretary</strong>. Your notes get written down
        on the manager&apos;s desk (
        <span className="font-mono">manager-directives.jsonl</span>); the
        manager reads them at its next iteration boundary, and its replies
        show up here interleaved with your notes.
      </div>

      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 space-y-3 overflow-y-auto px-3 py-3"
      >
        {turns.length === 0 ? (
          <p className="text-center text-xs italic text-gray-400">
            No conversation yet. Leave a note for the secretary below.
          </p>
        ) : (
          turns.map((t) =>
            t.kind === "note" ? (
              <NoteBubble key={t.id} ts={t.ts} text={t.text} />
            ) : (
              <ManagerBubble key={t.id} ts={t.ts} content={t.content} />
            ),
          )
        )}
      </div>

      <div className="border-t border-gray-200 p-3 dark:border-gray-700">
        <textarea
          value={note}
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={handleKey}
          rows={3}
          placeholder="Note for the secretary to leave on the manager's desk (Cmd-Enter to send)"
          className="w-full rounded border border-gray-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <p className="text-[11px] text-gray-500 dark:text-gray-400">
            Notes are picked up at the next loop boundary, not mid-action.
          </p>
          <button
            type="button"
            onClick={handleSend}
            disabled={!note.trim()}
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

function NoteBubble({ ts, text }: { ts: Date; text: string }) {
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
