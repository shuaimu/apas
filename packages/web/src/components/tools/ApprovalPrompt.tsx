"use client";

import { useStore } from "@/lib/store";
import { AlertTriangle, Check, X } from "lucide-react";

interface ApprovalPromptProps {
  toolCallId: string;
  tool: string;
  description: string;
}

export function ApprovalPrompt({ toolCallId, tool, description }: ApprovalPromptProps) {
  const { approve, reject } = useStore();

  return (
    <div className="border-2 border-yellow-400 dark:border-yellow-600 rounded-lg overflow-hidden my-2">
      <div className="flex items-center gap-2 px-4 py-2 bg-yellow-50 dark:bg-yellow-900/20">
        <AlertTriangle className="w-5 h-5 text-yellow-500" />
        <span className="font-medium text-yellow-700 dark:text-yellow-400">
          Permission Required
        </span>
      </div>

      <div className="px-4 py-3">
        <p className="text-sm text-gray-600 dark:text-gray-300 mb-3">
          Claude wants to use <strong>{tool}</strong>:
        </p>
        <p className="text-sm bg-gray-100 dark:bg-gray-800 rounded px-3 py-2 mb-4">
          {description}
        </p>

        <div className="flex gap-2">
          <button
            onClick={() => approve(toolCallId)}
            className="flex-1 flex items-center justify-center gap-2 px-4 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 transition-colors"
          >
            <Check className="w-4 h-4" />
            Approve
          </button>
          <button
            onClick={() => reject(toolCallId)}
            className="flex-1 flex items-center justify-center gap-2 px-4 py-2 bg-red-500 text-white rounded-lg hover:bg-red-600 transition-colors"
          >
            <X className="w-4 h-4" />
            Reject
          </button>
        </div>
      </div>
    </div>
  );
}
