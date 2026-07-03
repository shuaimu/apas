"use client";

import { useMemo, useState } from "react";
import { ChevronRight, ChevronDown, Wrench } from "lucide-react";
import type { Message } from "@/lib/store";
import { AssistantMessage } from "./AssistantMessage";

interface ToolGroupCardProps {
  items: Message[];
}

/// Minimum number of consecutive tool_use / tool_result messages needed
/// to collapse them into a ToolGroupCard. 2 = one full tool call
/// (use + result), so even a lone "Using Edit" / "Edit succeeded" pair
/// folds. A single unpaired tool_use stays inline — while a tool is
/// still running the user should see what it is without expanding.
export const TOOL_GROUP_MIN_ITEMS = 2;

/// A single item in the rendered stream — either a plain Message or a
/// synthetic group of consecutive tool-ish messages that render inside
/// a two-level-collapse card.
export type RenderItem =
  | { kind: "message"; message: Message }
  | { kind: "tool-group"; id: string; items: Message[] };

function isToolLikeMessage(m: Message): boolean {
  const ot = m.outputType;
  if (!ot) return false;
  if (ot.type === "tool_use") {
    // AskUserQuestion cards need to stay inline — the user has to see
    // and click them; hiding one inside a collapsed group would strand
    // an unanswered question.
    return ot.tool !== "AskUserQuestion";
  }
  if (ot.type === "tool_result") {
    return ot.tool !== "AskUserQuestion";
  }
  return false;
}

export function groupMessagesForRender(messages: Message[]): RenderItem[] {
  const out: RenderItem[] = [];
  let buf: Message[] = [];
  const flush = () => {
    if (buf.length === 0) return;
    if (buf.length >= TOOL_GROUP_MIN_ITEMS) {
      out.push({ kind: "tool-group", id: `tg-${buf[0].id}`, items: buf });
    } else {
      for (const m of buf) out.push({ kind: "message", message: m });
    }
    buf = [];
  };
  for (const m of messages) {
    if (isToolLikeMessage(m)) {
      buf.push(m);
    } else {
      flush();
      out.push({ kind: "message", message: m });
    }
  }
  flush();
  return out;
}

/// A single collapsed row summarising `items` consecutive tool_use /
/// tool_result messages. Expanded, it renders each item as a regular
/// AssistantMessage — every ToolCard inside remains independently
/// foldable, so this is a strict second level of collapse on top of
/// the existing per-card one.
///
/// The rule for what counts as "tool-ish" and the threshold for
/// grouping live in `groupMessagesForRender` in MessageList — this
/// component is dumb, it just renders whatever slice it's given.
export function ToolGroupCard({ items }: ToolGroupCardProps) {
  const [expanded, setExpanded] = useState(false);

  const summary = useMemo(() => summariseGroup(items), [items]);

  return (
    <div className="rounded-md border border-gray-200 dark:border-gray-700 bg-gray-50/40 dark:bg-gray-800/30">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800/50 focus:outline-none"
        title={
          expanded
            ? "Collapse — hide individual tool call cards"
            : "Expand — show each tool call as its own foldable card"
        }
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 flex-shrink-0 text-gray-400" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 flex-shrink-0 text-gray-400" />
        )}
        <Wrench className="h-3.5 w-3.5 flex-shrink-0 text-gray-400" />
        <span className="font-mono font-medium">
          {summary.callCount} tool call{summary.callCount === 1 ? "" : "s"}
        </span>
        <span className="text-gray-500 dark:text-gray-400 truncate">
          {summary.breakdown}
        </span>
        {summary.errorCount > 0 && (
          <span className="ml-auto rounded bg-red-100 px-1.5 py-0.5 font-mono text-[10px] text-red-700 dark:bg-red-900/40 dark:text-red-300">
            {summary.errorCount} err
          </span>
        )}
      </button>
      {expanded && (
        <div className="border-t border-gray-200 dark:border-gray-700 p-2 space-y-2">
          {items.map((m) => (
            <AssistantMessage key={m.id} message={m} />
          ))}
        </div>
      )}
    </div>
  );
}

interface GroupSummary {
  callCount: number;
  breakdown: string;
  errorCount: number;
}

/// Count each tool name once per (tool_use, tool_result) pair, or
/// once per orphan tool_use if the result hasn't arrived yet. Failed
/// tool_results contribute to `errorCount` so a red badge surfaces
/// without needing to expand.
function summariseGroup(items: Message[]): GroupSummary {
  const counts = new Map<string, number>();
  let callCount = 0;
  let errorCount = 0;
  for (const m of items) {
    const ot = m.outputType;
    if (!ot) continue;
    if (ot.type === "tool_use") {
      callCount += 1;
      counts.set(ot.tool, (counts.get(ot.tool) ?? 0) + 1);
    } else if (ot.type === "tool_result") {
      if (!ot.success) errorCount += 1;
      // Don't double-count — the matching tool_use already bumped the
      // per-tool counter. But do include unpaired results (tool_use
      // rendered elsewhere / filtered out) so the summary reflects
      // what the group actually contains.
      if (
        !items.some(
          (other) =>
            other.outputType?.type === "tool_use" &&
            other.outputType.tool === ot.tool,
        )
      ) {
        callCount += 1;
        counts.set(ot.tool, (counts.get(ot.tool) ?? 0) + 1);
      }
    }
  }
  const breakdown = Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, 4)
    .map(([tool, n]) => (n === 1 ? tool : `${tool} × ${n}`))
    .join(", ");
  return {
    callCount,
    breakdown: breakdown || "tool activity",
    errorCount,
  };
}
