"use client";

import { useState, useRef, useEffect, KeyboardEvent } from "react";
import { useStore } from "@/lib/store";
import { Send, StopCircle } from "lucide-react";

export function InputBox() {
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const { sendMessage, connected } = useStore();

  // Auto-resize textarea
  useEffect(() => {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = "auto";
      textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
    }
  }, [input]);

  const handleSubmit = () => {
    if (!input.trim() || !connected) return;

    sendMessage(input.trim());
    setInput("");
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <div className="border-t border-gray-200 dark:border-gray-800 p-4">
      <div className="max-w-4xl mx-auto">
        <div className="flex items-end gap-2 bg-gray-100 dark:bg-gray-800 rounded-2xl px-4 py-2">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={connected ? "Type a message..." : "Connect to start chatting"}
            disabled={!connected}
            rows={1}
            className="flex-1 bg-transparent border-none outline-none resize-none text-foreground placeholder-gray-400 disabled:opacity-50"
          />
          <button
            onClick={handleSubmit}
            disabled={!input.trim() || !connected}
            className="p-2 rounded-full bg-blue-500 text-white disabled:opacity-50 disabled:cursor-not-allowed hover:bg-blue-600 transition-colors"
          >
            <Send className="w-5 h-5" />
          </button>
        </div>
        <p className="text-xs text-gray-400 mt-2 text-center">
          Press Enter to send, Shift+Enter for new line
        </p>
      </div>
    </div>
  );
}
