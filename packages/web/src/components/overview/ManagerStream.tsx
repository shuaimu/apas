"use client";

/**
 * Manager v2b — embedded view of the manager pane's iteration stream
 * on the right half of the Overview tab.
 *
 * Renders messages from `paneMessages[paneKey(managerPane.pane_id)]` using
 * the same UserMessage / AssistantMessage components the regular pane chat
 * uses. Auto-scrolls to the bottom when at-or-near the bottom; sits still
 * when the user has scrolled up to read.
 *
 * When no manager pane exists yet, shows a placeholder pointing the user
 * at the Start manager button above.
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

interface ManagerStreamProps {
  managerPane?: PaneConfig;
  onOpenPane: (paneId: number) => void;
}

export function ManagerStream({ managerPane, onOpenPane }: ManagerStreamProps) {
  const messages = useStore((s) =>
    managerPane ? s.paneMessages[paneKey(managerPane.pane_id)] ?? [] : [],
  );

  if (!managerPane) {
    return (
      <div className="flex h-full flex-col items-center justify-center rounded border border-dashed border-violet-300 bg-violet-50/50 p-6 text-center text-sm text-violet-700 dark:border-violet-700 dark:bg-violet-950/20 dark:text-violet-300">
        <p className="font-medium">No manager running.</p>
        <p className="mt-1 text-xs">
          Click <span className="font-mono">Start manager</span> above to spawn the Tech-Lead deadloop. Its iteration stream will appear here.
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col rounded border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      <div className="flex items-center justify-between border-b border-gray-200 px-3 py-2 text-xs dark:border-gray-700">
        <span className="font-medium text-gray-700 dark:text-gray-200">
          Manager iteration stream
        </span>
        <button
          type="button"
          onClick={() => onOpenPane(managerPane.pane_id)}
          className="flex items-center gap-1 rounded border border-gray-300 bg-gray-50 px-1.5 py-0.5 text-[11px] text-gray-700 hover:bg-gray-100 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
          title="Open the manager's full pane view"
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
        No iterations yet. The manager will speak here when it runs.
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
