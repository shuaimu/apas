"use client";

import { memo } from "react";
import { Message } from "@/lib/store";
import { AlertTriangle, User } from "lucide-react";

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

export const UserMessage = memo(function UserMessage({ message }: UserMessageProps) {
  // The server refused this send. The text stays in the bubble to recover,
  // with the reason where the send is, instead of looking delivered.
  const refused = message.deliveryError;
  return (
    <div className="flex gap-2 sm:gap-3 justify-end min-w-0">
      <div className="max-w-[85%] sm:max-w-[80%] min-w-0">
        <div className={`bg-blue-500 text-white rounded-2xl rounded-tr-sm px-3 sm:px-4 py-2${refused ? " ring-2 ring-red-400 dark:ring-red-500" : ""}`}>
          <p className="whitespace-pre-wrap break-words text-base">{message.content}</p>
        </div>
        {refused && (
          <p role="status" className="mt-1 flex items-start justify-end gap-1 text-right text-xs text-red-600 dark:text-red-400">
            <AlertTriangle aria-hidden="true" className="mt-px h-3.5 w-3.5 flex-shrink-0" />
            <span className="min-w-0 break-words">Not delivered: {refused}</span>
          </p>
        )}
        <div className="text-xs text-gray-400 mt-1 text-right">
          {formatTimestamp(message.timestamp)}
        </div>
      </div>
      <div className="flex-shrink-0 w-6 h-6 sm:w-8 sm:h-8 rounded-full bg-blue-100 dark:bg-blue-900 flex items-center justify-center">
        <User className="w-4 h-4 sm:w-5 sm:h-5 text-blue-500" />
      </div>
    </div>
  );
});
