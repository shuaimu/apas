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

interface CommandExecutionEventPreview {
  eventType: string;
  command: string | null;
  status: string | null;
  exitCode: number | null;
  aggregatedOutput: string | null;
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

function parseCommandExecutionEventPreview(content: string): CommandExecutionEventPreview | null {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{") || !FOLDABLE_EVENT_TYPES.some(t => trimmed.includes(t))) {
    return null;
  }

  try {
    const parsed = JSON.parse(trimmed) as {
      type?: string;
      item?: {
        type?: string;
        command?: unknown;
        status?: unknown;
        exit_code?: unknown;
        aggregated_output?: unknown;
      };
    };

    if (!parsed.type || !FOLDABLE_EVENT_TYPES.includes(parsed.type) || parsed.item?.type !== "command_execution") {
      return null;
    }

    const command = typeof parsed.item.command === "string" ? parsed.item.command : null;
    const status = typeof parsed.item.status === "string" ? parsed.item.status : null;
    const exitCode = typeof parsed.item.exit_code === "number" ? parsed.item.exit_code : null;
    const aggregatedOutput = typeof parsed.item.aggregated_output === "string" ? parsed.item.aggregated_output : null;

    return {
      eventType: parsed.type,
      command,
      status,
      exitCode,
      aggregatedOutput,
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

    return {
      eventType: eventTypeMatch[1],
      command: commandMatch ? commandMatch[1] : null,
      status: statusMatch ? statusMatch[1] : null,
      exitCode: exitCodeMatch ? Number(exitCodeMatch[1]) : null,
      aggregatedOutput: aggregatedOutputMatch ? aggregatedOutputMatch[1].replace(/\\n/g, "\n") : null,
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

function TextContent({ content }: { content: string }) {
  const commandExecutionEvent = parseCommandExecutionEventPreview(content);

  if (commandExecutionEvent) {
    const commandLabel = commandExecutionEvent.command
      ? truncateValue(commandExecutionEvent.command)
      : "command_execution";
    const outputLineCount = commandExecutionEvent.aggregatedOutput
      ? commandExecutionEvent.aggregatedOutput.split("\n").length
      : null;
    const metaParts = [
      commandExecutionEvent.status ? `status: ${commandExecutionEvent.status}` : null,
      commandExecutionEvent.exitCode !== null ? `exit: ${commandExecutionEvent.exitCode}` : null,
      outputLineCount !== null ? `${outputLineCount} output lines` : null,
    ].filter(Boolean);

    return (
      <div className="bg-gray-100 dark:bg-gray-800 rounded-2xl rounded-tl-sm px-3 sm:px-4 py-2 max-w-full">
        <details>
          <summary className="cursor-pointer select-none text-sm font-medium text-gray-700 dark:text-gray-200">
            <span className="mr-2 rounded bg-gray-200 dark:bg-gray-700 px-2 py-0.5 text-xs font-semibold text-gray-600 dark:text-gray-300">
              {commandExecutionEvent.eventType}
            </span>
            <span className="break-all">{commandLabel}</span>
          </summary>
          {metaParts.length > 0 ? (
            <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">{metaParts.join(" • ")}</div>
          ) : null}
          <CodeBlock code={commandExecutionEvent.formattedPayload} language="json" />
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
              <tr className="even:bg-gray-50 dark:even:bg-gray-750">
                {children}
              </tr>
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
