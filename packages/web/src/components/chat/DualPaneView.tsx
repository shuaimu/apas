"use client";

import { useRef, useCallback, useEffect, useState } from "react";
import { useStore, Message, PaneType } from "@/lib/store";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";
import { InputBox } from "./InputBox";

export function DualPaneView() {
  const deadloopMessages = useStore((state) => state.deadloopMessages);
  const interactiveMessages = useStore((state) => state.interactiveMessages);
  const sendMessageToPane = useStore((state) => state.sendMessageToPane);
  const loadMoreMessages = useStore((state) => state.loadMoreMessages);
  const isLoadingMore = useStore((state) => state.isLoadingMore);
  const hasMoreMessages = useStore((state) => state.hasMoreMessages);

  return (
    <div className="flex h-full">
      {/* Left Pane - Deadloop */}
      <div className="w-1/2 border-r border-gray-200 dark:border-gray-700 flex flex-col">
        <PaneHeader title="Deadloop (Autonomous)" type="deadloop" />
        <MessagePane
          messages={deadloopMessages}
          onLoadMore={loadMoreMessages}
          isLoading={isLoadingMore}
          hasMore={hasMoreMessages}
        />
      </div>

      {/* Right Pane - Interactive */}
      <div className="w-1/2 flex flex-col">
        <PaneHeader title="Interactive" type="interactive" />
        <MessagePane
          messages={interactiveMessages}
          onLoadMore={loadMoreMessages}
          isLoading={isLoadingMore}
          hasMore={hasMoreMessages}
        />
        <div className="p-4 border-t border-gray-200 dark:border-gray-700">
          <InteractiveInput
            onSend={(text) => sendMessageToPane(text, "interactive")}
          />
        </div>
      </div>
    </div>
  );
}

interface PaneHeaderProps {
  title: string;
  type: PaneType;
}

function PaneHeader({ title, type }: PaneHeaderProps) {
  return (
    <div className={`px-4 py-2 border-b ${
      type === "deadloop"
        ? "bg-amber-50 dark:bg-amber-900/20 border-amber-200 dark:border-amber-800"
        : "bg-cyan-50 dark:bg-cyan-900/20 border-cyan-200 dark:border-cyan-800"
    }`}>
      <h2 className={`font-semibold ${
        type === "deadloop"
          ? "text-amber-700 dark:text-amber-300"
          : "text-cyan-700 dark:text-cyan-300"
      }`}>
        {title}
      </h2>
    </div>
  );
}

interface MessagePaneProps {
  messages: Message[];
  onLoadMore?: () => void;
  isLoading?: boolean;
  hasMore?: boolean;
}

function MessagePane({ messages, onLoadMore, isLoading, hasMore }: MessagePaneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const prevScrollHeight = useRef<number>(0);

  const checkIfAtBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) return true;
    const threshold = 100;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    return distanceFromBottom <= threshold;
  }, []);

  const checkIfNearTop = useCallback(() => {
    const container = containerRef.current;
    if (!container) return false;
    return container.scrollTop < 100;
  }, []);

  const handleScroll = useCallback(() => {
    shouldAutoScroll.current = checkIfAtBottom();

    // Check if near top and should load more
    if (checkIfNearTop() && onLoadMore && !isLoading && hasMore) {
      prevScrollHeight.current = containerRef.current?.scrollHeight || 0;
      onLoadMore();
    }
  }, [checkIfAtBottom, checkIfNearTop, onLoadMore, isLoading, hasMore]);

  // Maintain scroll position when prepending messages
  useEffect(() => {
    if (prevScrollHeight.current > 0 && containerRef.current) {
      const newScrollHeight = containerRef.current.scrollHeight;
      const scrollDiff = newScrollHeight - prevScrollHeight.current;
      if (scrollDiff > 0) {
        containerRef.current.scrollTop = scrollDiff;
      }
      prevScrollHeight.current = 0;
    }
  }, [messages.length]);

  useEffect(() => {
    if (shouldAutoScroll.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages]);

  if (messages.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400">
        <p className="text-sm">No messages yet</p>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto px-4 py-4 space-y-3"
    >
      {isLoading && (
        <div className="text-center text-gray-400 text-sm py-2">Loading...</div>
      )}
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
        <div className="text-center text-xs text-gray-500 py-1">
          <span>{message.content}</span>
        </div>
      );
    default:
      return null;
  }
}

interface InteractiveInputProps {
  onSend: (text: string) => void;
}

function InteractiveInput({ onSend }: InteractiveInputProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSubmit = () => {
    const text = textareaRef.current?.value.trim();
    if (text) {
      onSend(text);
      if (textareaRef.current) {
        textareaRef.current.value = "";
        textareaRef.current.style.height = "auto";
      }
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleInput = () => {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = "auto";
      textarea.style.height = Math.min(textarea.scrollHeight, 150) + "px";
    }
  };

  return (
    <div className="flex gap-2">
      <textarea
        ref={textareaRef}
        rows={1}
        placeholder="Type a message..."
        className="flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500"
        onKeyDown={handleKeyDown}
        onInput={handleInput}
      />
      <button
        onClick={handleSubmit}
        className="px-4 py-2 bg-cyan-500 hover:bg-cyan-600 text-white rounded-lg text-sm font-medium transition-colors"
      >
        Send
      </button>
    </div>
  );
}
