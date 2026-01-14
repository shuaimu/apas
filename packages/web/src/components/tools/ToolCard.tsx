"use client";

import { useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Edit,
  Terminal,
  Search,
  CheckCircle,
  XCircle,
  Wrench,
} from "lucide-react";

interface ToolCardProps {
  tool: string;
  input?: unknown;
  result?: string;
  success?: boolean;
  type: "use" | "result";
}

const toolIcons: Record<string, React.ReactNode> = {
  Read: <FileText className="w-4 h-4" />,
  Edit: <Edit className="w-4 h-4" />,
  Write: <FileText className="w-4 h-4" />,
  Bash: <Terminal className="w-4 h-4" />,
  Grep: <Search className="w-4 h-4" />,
  Glob: <Search className="w-4 h-4" />,
};

export function ToolCard({ tool, input, result, success, type }: ToolCardProps) {
  const [expanded, setExpanded] = useState(false);

  const icon = toolIcons[tool] || <Wrench className="w-4 h-4" />;

  if (type === "use") {
    return (
      <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden my-2">
        <button
          onClick={() => setExpanded(!expanded)}
          className="w-full flex items-center gap-2 px-3 py-2 bg-gray-50 dark:bg-gray-800 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
        >
          {expanded ? (
            <ChevronDown className="w-4 h-4 text-gray-400" />
          ) : (
            <ChevronRight className="w-4 h-4 text-gray-400" />
          )}
          <span className="text-blue-500">{icon}</span>
          <span className="font-medium text-sm">Using {tool}</span>
        </button>

        {expanded && input !== undefined ? (
          <div className="px-3 py-2 bg-gray-100 dark:bg-gray-900 text-sm">
            <pre className="overflow-x-auto text-gray-600 dark:text-gray-300">
              {typeof input === "string" ? input : JSON.stringify(input, null, 2)}
            </pre>
          </div>
        ) : null}
      </div>
    );
  }

  // Result type
  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden my-2">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-2 bg-gray-50 dark:bg-gray-800 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
      >
        {expanded ? (
          <ChevronDown className="w-4 h-4 text-gray-400" />
        ) : (
          <ChevronRight className="w-4 h-4 text-gray-400" />
        )}
        {success ? (
          <CheckCircle className="w-4 h-4 text-green-500" />
        ) : (
          <XCircle className="w-4 h-4 text-red-500" />
        )}
        <span className="font-medium text-sm">
          {tool} {success ? "succeeded" : "failed"}
        </span>
      </button>

      {expanded && result && (
        <div className="px-3 py-2 bg-gray-100 dark:bg-gray-900 text-sm">
          <pre className="overflow-x-auto text-gray-600 dark:text-gray-300 whitespace-pre-wrap">
            {result}
          </pre>
        </div>
      )}
    </div>
  );
}
