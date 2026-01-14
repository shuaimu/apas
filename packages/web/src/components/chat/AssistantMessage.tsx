"use client";

import { Message } from "@/lib/store";
import { Bot } from "lucide-react";
import ReactMarkdown from "react-markdown";
import { CodeBlock } from "@/components/code/CodeBlock";
import { ToolCard } from "@/components/tools/ToolCard";
import { ApprovalPrompt } from "@/components/tools/ApprovalPrompt";

interface AssistantMessageProps {
  message: Message;
}

export function AssistantMessage({ message }: AssistantMessageProps) {
  const outputType = message.outputType;

  return (
    <div className="flex gap-3">
      <div className="flex-shrink-0 w-8 h-8 rounded-full bg-purple-100 dark:bg-purple-900 flex items-center justify-center">
        <Bot className="w-5 h-5 text-purple-500" />
      </div>
      <div className="max-w-[80%] flex-1">
        {renderContent(message, outputType)}
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
    <div className="bg-gray-100 dark:bg-gray-800 rounded-2xl rounded-tl-sm px-4 py-2 prose dark:prose-invert max-w-none">
      <ReactMarkdown
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
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
