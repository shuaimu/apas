"use client";

import { useEffect, useRef, useCallback } from "react";
import { useStore, Message } from "@/lib/store";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";

export function MessageList() {
  const messages = useStore((state) => state.messages);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);

  // Check if user is near the bottom (within 100px)
  const checkIfAtBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) return true;
    const threshold = 100;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    return distanceFromBottom <= threshold;
  }, []);

  // Update auto-scroll flag on scroll
  const handleScroll = useCallback(() => {
    shouldAutoScroll.current = checkIfAtBottom();
  }, [checkIfAtBottom]);

  // Auto-scroll to bottom only if user was already at bottom
  useEffect(() => {
    if (shouldAutoScroll.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
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
    <div
      ref={containerRef}
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto px-4 py-6 space-y-4"
    >
      {messages.map((message) => (
        <MessageComponent key={message.id} message={message} />
      ))}
      <div ref={messagesEndRef} />
    </div>
  );
}

function formatTime(date: Date): string {
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
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
          <span>{message.content}</span>
          <span className="text-xs text-gray-400 ml-2">{formatTime(message.timestamp)}</span>
        </div>
      );
    default:
      return null;
  }
}
