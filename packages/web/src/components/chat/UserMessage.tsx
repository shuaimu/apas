"use client";

import { Message } from "@/lib/store";
import { User } from "lucide-react";

interface UserMessageProps {
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

export function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="flex gap-2 sm:gap-3 justify-end min-w-0">
      <div className="max-w-[85%] sm:max-w-[80%] min-w-0">
        <div className="bg-blue-500 text-white rounded-2xl rounded-tr-sm px-3 sm:px-4 py-2">
          <p className="whitespace-pre-wrap break-words text-sm sm:text-base">{message.content}</p>
        </div>
        <div className="text-xs text-gray-400 mt-1 text-right">
          {formatTimestamp(message.timestamp)}
        </div>
      </div>
      <div className="flex-shrink-0 w-6 h-6 sm:w-8 sm:h-8 rounded-full bg-blue-100 dark:bg-blue-900 flex items-center justify-center">
        <User className="w-4 h-4 sm:w-5 sm:h-5 text-blue-500" />
      </div>
    </div>
  );
}
