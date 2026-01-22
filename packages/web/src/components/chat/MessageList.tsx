"use client";

import { useEffect, useRef, useCallback } from "react";
import { useStore, Message } from "@/lib/store";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";

export function MessageList() {
  const messages = useStore((state) => state.messages);
  const hasMoreMessages = useStore((state) => state.hasMoreMessages);
  const isLoadingMore = useStore((state) => state.isLoadingMore);
  const loadMoreMessages = useStore((state) => state.loadMoreMessages);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const previousScrollHeight = useRef<number>(0);
  const previousMessageCount = useRef<number>(0);

  // Check if user is near the bottom (within 100px)
  const checkIfAtBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) return true;
    const threshold = 100;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    return distanceFromBottom <= threshold;
  }, []);

  // Check if user is near the top (within 50px)
  const checkIfAtTop = useCallback(() => {
    const container = containerRef.current;
    if (!container) return false;
    return container.scrollTop <= 50;
  }, []);

  // Update auto-scroll flag on scroll and check for loading more
  const handleScroll = useCallback(() => {
    shouldAutoScroll.current = checkIfAtBottom();

    // Load more when scrolled to top
    if (checkIfAtTop() && hasMoreMessages && !isLoadingMore) {
      loadMoreMessages();
    }
  }, [checkIfAtBottom, checkIfAtTop, hasMoreMessages, isLoadingMore, loadMoreMessages]);

  // Preserve scroll position when prepending messages
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // If messages were prepended (count increased but we were loading more)
    if (messages.length > previousMessageCount.current && previousScrollHeight.current > 0) {
      const scrollDiff = container.scrollHeight - previousScrollHeight.current;
      if (scrollDiff > 0 && container.scrollTop < 100) {
        // Restore scroll position relative to old content
        container.scrollTop = scrollDiff;
      }
    }

    previousScrollHeight.current = container.scrollHeight;
    previousMessageCount.current = messages.length;
  }, [messages]);

  // Auto-scroll to bottom only if user was already at bottom (for new messages at bottom)
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
      className="flex-1 overflow-y-auto overflow-x-hidden px-2 sm:px-4 py-4 sm:py-6 space-y-3 sm:space-y-4"
    >
      {/* Loading indicator at top */}
      {isLoadingMore && (
        <div className="text-center text-sm text-gray-400 py-2">
          Loading older messages...
        </div>
      )}
      {/* Show hint if there are more messages */}
      {hasMoreMessages && !isLoadingMore && (
        <div className="text-center text-xs text-gray-500 py-1">
          Scroll up to load more
        </div>
      )}
      {messages.map((message) => (
        <MessageComponent key={message.id} message={message} />
      ))}
      <div ref={messagesEndRef} />
    </div>
  );
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
          <span className="text-xs text-gray-400 ml-2">{formatTimestamp(message.timestamp)}</span>
        </div>
      );
    default:
      return null;
  }
}
