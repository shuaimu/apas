"use client";

/**
 * v3 — embedded view of the Tech Lead pane's iteration stream on the
 * right half of the Overview tab.
 *
 * Renders messages from `paneMessages[paneKey(techLeadPane.pane_id)]`
 * using the same UserMessage / AssistantMessage components the regular
 * pane chat uses. Auto-scrolls to the bottom when parked there; sits
 * still when the user has scrolled up to read.
 *
 * When no Tech Lead pane exists, shows a placeholder pointing the user
 * at the Start Tech Lead button above. The Tech Lead is optional in v3 —
 * a Manager can run alone for simple "I drive everything" usage.
 */
import { memo, useEffect, useRef } from "react";
import { ExternalLink } from "lucide-react";
import {
  Message,
  PaneConfig,
  paneKey,
  useStore,
} from "@/lib/store";
import { UserMessage } from "@/components/chat/UserMessage";
import { AssistantMessage } from "@/components/chat/AssistantMessage";

interface TechLeadStreamProps {
  techLeadPane?: PaneConfig;
  onOpenPane: (paneId: number) => void;
}

// Stable reference for the empty case — `?? []` inline would return a
// fresh array each selector call. Zustand uses Object.is to compare, so
// a new array reference looks "changed" every time, the component
// re-renders, the selector returns yet another new array, and React
// bails out with #185 (max update depth exceeded).
const EMPTY_MESSAGES: Message[] = [];

export function TechLeadStream({ techLeadPane, onOpenPane }: TechLeadStreamProps) {
  const messages = useStore((s) => {
    if (!techLeadPane) return EMPTY_MESSAGES;
    return s.paneMessages[paneKey(techLeadPane.pane_id)] ?? EMPTY_MESSAGES;
  });

  if (!techLeadPane) {
    return (
      <div className="flex h-full flex-col items-center justify-center rounded border border-dashed border-indigo-300 bg-indigo-50/50 p-6 text-center text-sm text-indigo-700 dark:border-indigo-700 dark:bg-indigo-950/20 dark:text-indigo-300">
        <p className="font-medium">No Tech Lead running.</p>
        <p className="mt-1 text-xs">
          Click <span className="font-mono">Start Tech Lead</span> above to
          spawn the autonomous orchestrator deadloop. Its iteration stream
          will appear here.
        </p>
        <p className="mt-2 text-[11px] text-indigo-600/80 dark:text-indigo-400/80">
          (Optional — your Manager can run alone if you drive everything
          yourself.)
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col rounded border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      <div className="flex items-center justify-between border-b border-gray-200 px-3 py-2 text-xs dark:border-gray-700">
        <span className="font-medium text-gray-700 dark:text-gray-200">
          Tech Lead iteration stream
        </span>
        <button
          type="button"
          onClick={() => onOpenPane(techLeadPane.pane_id)}
          className="flex items-center gap-1 rounded border border-gray-300 bg-gray-50 px-1.5 py-0.5 text-[11px] text-gray-700 hover:bg-gray-100 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
          title="Open the Tech Lead's full pane view"
        >
          <ExternalLink className="h-3 w-3" /> Open pane
        </button>
      </div>
      <StreamBody messages={messages} />
    </div>
  );
}

function StreamBody({ messages }: { messages: Message[] }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);

  // Track whether the user is parked near the bottom so new messages
  // pull them along; once they scroll up, stop auto-scrolling.
  const handleScroll = () => {
    const c = containerRef.current;
    if (!c) return;
    const dist = c.scrollHeight - c.scrollTop - c.clientHeight;
    shouldAutoScroll.current = dist <= 80;
  };

  useEffect(() => {
    const c = containerRef.current;
    if (!c) return;
    if (shouldAutoScroll.current) {
      c.scrollTop = c.scrollHeight;
    }
  }, [messages.length]);

  if (messages.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-4 text-xs italic text-gray-500 dark:text-gray-400">
        No iterations yet. The Tech Lead will speak here when it runs.
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      onScroll={handleScroll}
      className="flex-1 space-y-3 overflow-y-auto overflow-x-hidden px-2 py-3"
    >
      {messages.map((m) => (
        <MessageItem key={m.id} message={m} />
      ))}
    </div>
  );
}

const MessageItem = memo(function MessageItem({ message }: { message: Message }) {
  switch (message.role) {
    case "user":
      return <UserMessage message={message} />;
    case "assistant":
      return <AssistantMessage message={message} />;
    case "system":
      return (
        <div className="text-center text-xs italic text-gray-500 dark:text-gray-400">
          {message.content}
        </div>
      );
    default:
      return null;
  }
});
