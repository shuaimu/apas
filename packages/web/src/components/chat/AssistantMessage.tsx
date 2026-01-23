"use client";

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

export function AssistantMessage({ message }: AssistantMessageProps) {
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
}

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

function TextContent({ content }: { content: string }) {
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
