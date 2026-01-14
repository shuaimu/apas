"use client";

import { useStore } from "@/lib/store";
import { FolderOpen, RefreshCw } from "lucide-react";
import { useMemo } from "react";

export function Sidebar() {
  const { cliClients, sessions, attachSession, loadSessionMessages, refreshCliClients, listSessions, sessionId, connected } = useStore();

  // Merge CLI clients (active) and sessions (historical) into unified project list
  // Deduplicate by working directory, keeping the most recent session
  const projects = useMemo(() => {
    const projectMap = new Map<string, {
      id: string;
      name: string;
      workingDir: string;
      hostname?: string;
      isActive: boolean;
      createdAt?: string;
    }>();

    // Sort sessions by date (newest first) so we keep the most recent per directory
    const sortedSessions = [...sessions].sort((a, b) => {
      if (a.createdAt && b.createdAt) {
        return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
      }
      return 0;
    });

    // Add sessions, deduplicating by working directory
    for (const session of sortedSessions) {
      const workingDir = session.workingDir || session.id;
      const name = session.workingDir?.split('/').pop() || `Project ${session.id.slice(0, 8)}`;

      // Only add if we haven't seen this directory yet
      if (!projectMap.has(workingDir)) {
        projectMap.set(workingDir, {
          id: session.id,
          name,
          workingDir,
          hostname: session.hostname,
          isActive: session.status === "active",
          createdAt: session.createdAt,
        });
      }
    }

    // Mark projects as active if they have a connected CLI client
    for (const client of cliClients) {
      if (client.activeSession) {
        // Find project by session ID and mark as active
        for (const project of projectMap.values()) {
          if (project.id === client.activeSession) {
            project.isActive = true;
            break;
          }
        }
      }
    }

    // Sort: active first, then by creation date (newest first)
    return Array.from(projectMap.values()).sort((a, b) => {
      if (a.isActive !== b.isActive) return a.isActive ? -1 : 1;
      if (a.createdAt && b.createdAt) {
        return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
      }
      return 0;
    });
  }, [cliClients, sessions]);

  const handleRefresh = () => {
    refreshCliClients();
    listSessions();
  };

  const handleProjectClick = (projectId: string, isActive: boolean) => {
    if (isActive) {
      attachSession(projectId);
    } else {
      loadSessionMessages(projectId);
    }
  };

  return (
    <div className="w-64 border-r border-gray-200 dark:border-gray-800 flex flex-col bg-gray-50 dark:bg-gray-900">
      {/* Header */}
      <div className="p-4 border-b border-gray-200 dark:border-gray-800">
        <div className="flex items-center justify-between">
          <h2 className="font-semibold text-sm text-gray-600 dark:text-gray-400">
            Projects
          </h2>
          <button
            onClick={handleRefresh}
            disabled={!connected}
            className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded disabled:opacity-50"
            title="Refresh"
          >
            <RefreshCw className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Project list */}
      <div className="flex-1 overflow-y-auto p-2">
        {projects.length === 0 ? (
          <div className="text-center text-gray-400 text-sm py-8">
            <FolderOpen className="w-8 h-8 mx-auto mb-2 opacity-50" />
            <p>No projects yet</p>
            <p className="text-xs mt-1">Run `apas` in a directory to start</p>
          </div>
        ) : (
          <div className="space-y-1">
            {projects.map((project) => (
              <button
                key={project.id}
                onClick={() => handleProjectClick(project.id, project.isActive)}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left text-sm transition-colors ${
                  sessionId === project.id
                    ? "bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300"
                    : "hover:bg-gray-200 dark:hover:bg-gray-800"
                }`}
              >
                <div
                  className={`w-2 h-2 rounded-full ${
                    project.isActive ? "bg-green-500" : "bg-gray-400"
                  }`}
                />
                <div className="flex-1 min-w-0">
                  {project.hostname && (
                    <div className="text-xs text-gray-500 truncate">
                      {project.hostname}
                    </div>
                  )}
                  <div className="font-medium truncate">
                    {project.workingDir}
                  </div>
                  <div className="text-xs text-gray-500 truncate">
                    {project.isActive
                      ? "Active"
                      : project.createdAt
                        ? new Date(project.createdAt).toLocaleDateString()
                        : ""}
                  </div>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
