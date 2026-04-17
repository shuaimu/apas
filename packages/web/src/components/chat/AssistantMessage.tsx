"use client";

import { memo } from "react";
import { Message } from "@/lib/store";
import { Bot } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/components/code/CodeBlock";
import { ToolCard } from "@/components/tools/ToolCard";
import { ApprovalPrompt } from "@/components/tools/ApprovalPrompt";

interface AssistantMessageProps {
  message: Message;
}

interface FoldableItemEventPreview {
  eventType: string;
  title: string;
  meta: string | null;
  formattedPayload: string;
}

function formatTimestamp(date: Date): string {
  const now = new Date();
  const isToday = date.toDateString() === now.toDateString();

  if (isToday) {
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  } else {
    return date.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' +
           date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
}

const FOLDABLE_EVENT_TYPES = ["item.completed", "item.started"];

function parseFoldableItemEventPreview(content: string): FoldableItemEventPreview | null {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{") || !FOLDABLE_EVENT_TYPES.some(t => trimmed.includes(t))) {
    return null;
  }

  try {
    const parsed = JSON.parse(trimmed) as {
      type?: string;
      item?: {
        id?: unknown;
        type?: unknown;
        command?: unknown;
        status?: unknown;
        exit_code?: unknown;
        aggregated_output?: unknown;
        items?: Array<{ text?: unknown; completed?: unknown }>;
      };
    };

    if (
      !parsed.type ||
      !FOLDABLE_EVENT_TYPES.includes(parsed.type) ||
      !parsed.item ||
      typeof parsed.item !== "object"
    ) {
      return null;
    }

    const itemType = typeof parsed.item.type === "string" ? parsed.item.type : "unknown";

    if (itemType === "command_execution") {
      const command = typeof parsed.item.command === "string" ? parsed.item.command : null;
      const status = typeof parsed.item.status === "string" ? parsed.item.status : null;
      const exitCode = typeof parsed.item.exit_code === "number" ? parsed.item.exit_code : null;
      const aggregatedOutput = typeof parsed.item.aggregated_output === "string" ? parsed.item.aggregated_output : null;
      const outputLineCount = aggregatedOutput ? aggregatedOutput.split("\n").length : null;
      const metaParts = [
        status ? `status: ${status}` : null,
        exitCode !== null ? `exit: ${exitCode}` : null,
        outputLineCount !== null ? `${outputLineCount} output lines` : null,
      ].filter(Boolean);

      return {
        eventType: parsed.type,
        title: command ? truncateValue(command) : "command_execution",
        meta: metaParts.length > 0 ? metaParts.join(" • ") : null,
        formattedPayload: JSON.stringify(parsed, null, 2),
      };
    }

    if (itemType === "todo_list") {
      const items = Array.isArray(parsed.item.items) ? parsed.item.items : [];
      const completed = items.filter((item) => item?.completed === true).length;
      const firstText = items.find(
        (item): item is { text: string; completed?: unknown } =>
          typeof item?.text === "string" && item.text.trim().length > 0,
      )?.text;
      const metaParts = [`${completed}/${items.length} completed`];
      if (firstText) {
        metaParts.push(`next: ${truncateValue(firstText.replace(/\s+/g, " "), 90)}`);
      }

      return {
        eventType: parsed.type,
        title: "todo_list",
        meta: metaParts.join(" • "),
        formattedPayload: JSON.stringify(parsed, null, 2),
      };
    }

    const itemId = typeof parsed.item.id === "string" ? parsed.item.id : null;
    return {
      eventType: parsed.type,
      title: itemType,
      meta: itemId ? `id: ${itemId}` : null,
      formattedPayload: JSON.stringify(parsed, null, 2),
    };
  } catch {
    const eventTypeMatch = trimmed.match(/"type":"(item\.(?:completed|started))"/);
    if (
      !eventTypeMatch ||
      !trimmed.includes('"type":"command_execution"')
    ) {
      return null;
    }

    const commandMatch = trimmed.match(/"command":"([\s\S]*?)","aggregated_output":/);
    const statusMatch = trimmed.match(/"status":"([^"]+)"/);
    const exitCodeMatch = trimmed.match(/"exit_code":(-?\d+)/);
    const aggregatedOutputMatch = trimmed.match(/"aggregated_output":"([\s\S]*?)","exit_code":/);

    const aggregatedOutput = aggregatedOutputMatch ? aggregatedOutputMatch[1].replace(/\\n/g, "\n") : null;
    const outputLineCount = aggregatedOutput ? aggregatedOutput.split("\n").length : null;
    const metaParts = [
      statusMatch ? `status: ${statusMatch[1]}` : null,
      exitCodeMatch ? `exit: ${Number(exitCodeMatch[1])}` : null,
      outputLineCount !== null ? `${outputLineCount} output lines` : null,
    ].filter(Boolean);

    return {
      eventType: eventTypeMatch[1],
      title: commandMatch ? truncateValue(commandMatch[1]) : "command_execution",
      meta: metaParts.length > 0 ? metaParts.join(" • ") : null,
      formattedPayload: trimmed,
    };
  }
}

function truncateValue(value: string, max = 140): string {
  if (value.length <= max) {
    return value;
  }
  return `${value.slice(0, max)}...`;
}

export const AssistantMessage = memo(function AssistantMessage({ message }: AssistantMessageProps) {
  const outputType = message.outputType;

  return (
    <div className="flex gap-2 sm:gap-3 min-w-0">
      <div className="flex-shrink-0 w-6 h-6 sm:w-8 sm:h-8 rounded-full bg-purple-100 dark:bg-purple-900 flex items-center justify-center">
        <Bot className="w-4 h-4 sm:w-5 sm:h-5 text-purple-500" />
      </div>
      <div className="flex-1 min-w-0 overflow-hidden">
        {renderContent(message, outputType)}
        <div className="text-xs text-gray-400 mt-1">
          {formatTimestamp(message.timestamp)}
        </div>
      </div>
    </div>
  );
});

function renderContent(message: Message, outputType: Message["outputType"]) {
  if (!outputType) {
    return <TextContent content={message.content} />;
  }

  switch (outputType.type) {
    case "text":
      return <TextContent content={message.content} />;

    case "code":
      return <CodeBlock code={message.content} language={outputType.language} />;

    case "tool_use":
      return (
        <ToolCard
          tool={outputType.tool}
          input={outputType.input}
          type="use"
        />
      );

    case "tool_result":
      return (
        <ToolCard
          tool={outputType.tool}
          result={message.content}
          success={outputType.success}
          type="result"
        />
      );

    case "approval_request":
      return (
        <ApprovalPrompt
          toolCallId={outputType.toolCallId}
          tool={outputType.tool}
          description={outputType.description}
        />
      );

    case "error":
      return (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg px-4 py-2 text-red-600 dark:text-red-400">
          {message.content}
        </div>
      );

    case "system":
      return (
        <div className="text-gray-500 text-sm italic">{message.content}</div>
      );

    default:
      return <TextContent content={message.content} />;
  }
}

/** Detect raw Claude stream-json messages (e.g. user tool_result) that leaked through as text */
function parseRawStreamMessage(content: string): { msgType: string; summary: string; formatted: string } | null {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed.type && parsed.message && typeof parsed.type === "string") {
      const msgType = parsed.type as string;
      let summary = msgType;
      if (parsed.message?.content && Array.isArray(parsed.message.content)) {
        const blockTypes = (parsed.message.content as Array<{ type?: string }>)
          .map((b) => b.type || "unknown");
        summary = `${msgType} [${blockTypes.join(", ")}]`;
      }
      return { msgType, summary, formatted: JSON.stringify(parsed, null, 2) };
    }
  } catch { /* not JSON */ }
  return null;
}

/** Detect pipe-table blocks that lack a header separator and inject one so remark-gfm renders them. */
function fixHeaderlessTables(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  const isTableRow = (l: string) => /^\|.*\|.*\|/.test(l.trim());
  const isSeparator = (l: string) => /^\|[\s:]*-+[\s:]*/.test(l.trim());

  let i = 0;
  while (i < lines.length) {
    if (isTableRow(lines[i]) && !(i + 1 < lines.length && isSeparator(lines[i + 1]))) {
      // Start of a headerless table block — collect all consecutive rows
      const blockStart = i;
      while (i < lines.length && isTableRow(lines[i])) i++;
      const firstRow = lines[blockStart];
      const cols = firstRow.trim().replace(/^\||\|$/g, "").split("|").length;
      const separator = "|" + " --- |".repeat(cols);
      out.push(firstRow);
      out.push(separator);
      for (let j = blockStart + 1; j < i; j++) out.push(lines[j]);
    } else {
      out.push(lines[i]);
      i++;
    }
  }
  return out.join("\n");
}

function TextContent({ content }: { content: string }) {
  const foldableItemEvent = parseFoldableItemEventPreview(content);

  if (foldableItemEvent) {
    return (
      <div className="bg-gray-100 dark:bg-gray-800 rounded-2xl rounded-tl-sm px-3 sm:px-4 py-2 max-w-full">
        <details>
          <summary className="cursor-pointer select-none text-sm font-medium text-gray-700 dark:text-gray-200">
            <span className="mr-2 rounded bg-gray-200 dark:bg-gray-700 px-2 py-0.5 text-xs font-semibold text-gray-600 dark:text-gray-300">
              {foldableItemEvent.eventType}
            </span>
            <span className="break-all">{foldableItemEvent.title}</span>
          </summary>
          {foldableItemEvent.meta ? (
            <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">{foldableItemEvent.meta}</div>
          ) : null}
          <CodeBlock code={foldableItemEvent.formattedPayload} language="json" />
        </details>
      </div>
    );
  }

  // Fold raw Claude stream-json messages that leaked through as text output
  const rawStream = parseRawStreamMessage(content);
  if (rawStream) {
    return (
      <div className="bg-gray-100 dark:bg-gray-800 rounded-2xl rounded-tl-sm px-3 sm:px-4 py-2 max-w-full">
        <details>
          <summary className="cursor-pointer select-none text-sm font-medium text-gray-700 dark:text-gray-200">
            <span className="mr-2 rounded bg-gray-200 dark:bg-gray-700 px-2 py-0.5 text-xs font-semibold text-gray-600 dark:text-gray-300">
              {rawStream.msgType}
            </span>
            <span className="break-all">{rawStream.summary}</span>
          </summary>
          <CodeBlock code={rawStream.formatted} language="json" />
        </details>
      </div>
    );
  }

  const processed = fixHeaderlessTables(content);

  return (
    <div className="bg-gray-100 dark:bg-gray-800 rounded-2xl rounded-tl-sm px-3 sm:px-4 py-2 prose dark:prose-invert prose-sm sm:prose-base max-w-full overflow-x-auto">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || "");
            const isInline = !match;

            if (isInline) {
              return (
                <code
                  className="bg-gray-200 dark:bg-gray-700 px-1 py-0.5 rounded text-sm"
                  {...props}
                >
                  {children}
                </code>
              );
            }

            return (
              <CodeBlock
                code={String(children).replace(/\n$/, "")}
                language={match[1]}
              />
            );
          },
          table({ children }) {
            return (
              <div className="overflow-x-auto my-2">
                <table className="min-w-full border-collapse text-sm">
                  {children}
                </table>
              </div>
            );
          },
          thead({ children }) {
            return (
              <thead className="bg-gray-200 dark:bg-gray-700">
                {children}
              </thead>
            );
          },
          th({ children }) {
            return (
              <th className="border border-gray-300 dark:border-gray-600 px-3 py-2 text-left font-semibold">
                {children}
              </th>
            );
          },
          td({ children }) {
            return (
              <td className="border border-gray-300 dark:border-gray-600 px-3 py-2">
                {children}
              </td>
            );
          },
          tr({ children }) {
            return (
              <tr className="even:bg-gray-50 dark:even:bg-gray-800">
                {children}
              </tr>
            );
          },
        }}
      >
        {processed}
      </ReactMarkdown>
    </div>
  );
}
