"use client";

import { Message } from "@/lib/store";
import { User } from "lucide-react";

interface UserMessageProps {
  message: Message;
}

export function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="flex gap-3 justify-end">
      <div className="max-w-[80%] bg-blue-500 text-white rounded-2xl rounded-tr-sm px-4 py-2">
        <p className="whitespace-pre-wrap">{message.content}</p>
      </div>
      <div className="flex-shrink-0 w-8 h-8 rounded-full bg-blue-100 dark:bg-blue-900 flex items-center justify-center">
        <User className="w-5 h-5 text-blue-500" />
      </div>
    </div>
  );
}
