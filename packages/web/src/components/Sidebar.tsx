"use client";

import { useStore } from "@/lib/store";
import { Monitor, Plus, RefreshCw } from "lucide-react";

export function Sidebar() {
  const { cliClients, startSession, attachSession, refreshCliClients, sessionId, connected } = useStore();

  return (
    <div className="w-64 border-r border-gray-200 dark:border-gray-800 flex flex-col bg-gray-50 dark:bg-gray-900">
      {/* Header */}
      <div className="p-4 border-b border-gray-200 dark:border-gray-800">
        <div className="flex items-center justify-between">
          <h2 className="font-semibold text-sm text-gray-600 dark:text-gray-400">
            CLI Clients
          </h2>
          <button
            onClick={refreshCliClients}
            disabled={!connected}
            className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded disabled:opacity-50"
            title="Refresh"
          >
            <RefreshCw className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Client list */}
      <div className="flex-1 overflow-y-auto p-2">
        {cliClients.length === 0 ? (
          <div className="text-center text-gray-400 text-sm py-8">
            <Monitor className="w-8 h-8 mx-auto mb-2 opacity-50" />
            <p>No CLI clients connected</p>
            <p className="text-xs mt-1">Run `apas` on a machine to connect</p>
          </div>
        ) : (
          <div className="space-y-1">
            {cliClients.map((client) => (
              <button
                key={client.id}
                onClick={() => {
                  if (client.activeSession) {
                    // Attach to existing session
                    attachSession(client.activeSession);
                  } else {
                    // Start new session with this CLI
                    startSession(client.id);
                  }
                }}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left text-sm transition-colors ${
                  sessionId && (client.activeSession === sessionId || client.id === sessionId)
                    ? "bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300"
                    : "hover:bg-gray-200 dark:hover:bg-gray-800"
                }`}
              >
                <div
                  className={`w-2 h-2 rounded-full ${
                    client.status === "online"
                      ? "bg-green-500"
                      : client.status === "busy"
                      ? "bg-yellow-500"
                      : "bg-gray-400"
                  }`}
                />
                <div className="flex-1 min-w-0">
                  <div className="font-medium truncate">
                    {client.name || `CLI ${client.id.slice(0, 8)}`}
                  </div>
                  <div className="text-xs text-gray-500 truncate">
                    {client.activeSession ? "Active session" : client.status}
                  </div>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="p-4 border-t border-gray-200 dark:border-gray-800">
        <button
          onClick={() => startSession()}
          disabled={!connected || cliClients.length === 0}
          className="w-full flex items-center justify-center gap-2 px-3 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          <Plus className="w-4 h-4" />
          <span>New Session</span>
        </button>
      </div>
    </div>
  );
}
