"use client";

/**
 * Manager v2b — directive composer + recent-directives history.
 *
 * The composer appends a `{ts, text}` record to manager-directives.jsonl on
 * the CLI host. The web doesn't currently re-read the file, so the
 * "recent directives" list is a session-local mirror of what was sent
 * from this tab. Good enough for "what did I just tell the manager?"
 * affordance; a v3 could echo back the file contents.
 */
import { useState } from "react";
import { Send } from "lucide-react";
import { useStore } from "@/lib/store";

interface SentDirective {
  id: string;
  ts: Date;
  text: string;
}

export function DirectivesPanel() {
  const addManagerDirective = useStore((s) => s.addManagerDirective);
  const showToast = useStore((s) => s.showToast);

  const [directive, setDirective] = useState("");
  const [history, setHistory] = useState<SentDirective[]>([]);

  const handleSend = () => {
    const text = directive.trim();
    if (!text) return;
    addManagerDirective(text);
    setHistory((prev) => [
      {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        ts: new Date(),
        text,
      },
      ...prev,
    ]);
    setDirective("");
    showToast("Directive queued for next manager iteration.", "info");
  };

  const handleKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex h-full flex-col gap-3">
      <div>
        <p className="mb-1 text-[11px] text-gray-500 dark:text-gray-400">
          Appended to <span className="font-mono">manager-directives.jsonl</span>.
          Manager tails this file at the start of every loop iteration — directives
          are absorbed at loop boundaries, never mid-action.
        </p>
        <textarea
          value={directive}
          onChange={(e) => setDirective(e.target.value)}
          onKeyDown={handleKey}
          rows={5}
          placeholder="Strategy nudge / correction / question (Cmd-Enter to send)"
          className="w-full rounded border border-gray-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-100"
        />
        <div className="mt-2 flex justify-end">
          <button
            type="button"
            onClick={handleSend}
            disabled={!directive.trim()}
            className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Send className="h-3.5 w-3.5" /> Send directive
          </button>
        </div>
      </div>

      {history.length > 0 && (
        <div className="flex-1 min-h-0 overflow-auto">
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
            Sent this session ({history.length})
          </p>
          <ul className="space-y-2">
            {history.map((d) => (
              <li
                key={d.id}
                className="rounded border border-gray-200 bg-gray-50 px-2 py-1.5 dark:border-gray-700 dark:bg-gray-800/40"
              >
                <div className="text-[10px] text-gray-400 dark:text-gray-500">
                  {d.ts.toLocaleTimeString()}
                </div>
                <div className="whitespace-pre-wrap text-xs text-gray-800 dark:text-gray-200">
                  {d.text}
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
