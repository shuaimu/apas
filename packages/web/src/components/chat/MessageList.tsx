"use client";

import { useEffect, useRef } from "react";
import { useStore, Message } from "@/lib/store";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";

export function MessageList() {
  const messages = useStore((state) => state.messages);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new messages arrive
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  if (messages.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400">
        <div className="text-center">
          <p className="text-lg">No messages yet</p>
          <p className="text-sm mt-1">Start a conversation with Claude</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto px-4 py-6 space-y-4">
      {messages.map((message) => (
        <MessageComponent key={message.id} message={message} />
      ))}
      <div ref={messagesEndRef} />
    </div>
  );
}

function MessageComponent({ message }: { message: Message }) {
  switch (message.role) {
    case "user":
      return <UserMessage message={message} />;
    case "assistant":
      return <AssistantMessage message={message} />;
    case "system":
      return (
        <div className="text-center text-sm text-gray-500 py-2">
          {message.content}
        </div>
      );
    default:
      return null;
  }
}
