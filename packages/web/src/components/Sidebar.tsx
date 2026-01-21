"use client";

import { useStore } from "@/lib/store";
import { FolderOpen, RefreshCw, Share2, Users, X, Crown, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://apas.mpaxos.com:8080";

interface ShareUser {
  user_id: string;
  user_email: string;
  is_owner: boolean;
  created_at?: string;
}

export function Sidebar() {
  const { cliClients, sessions, attachSession, loadSessionMessages, refreshCliClients, listSessions, sessionId, connected, token } = useStore();
  const [shareModalOpen, setShareModalOpen] = useState(false);
  const [shareSessionId, setShareSessionId] = useState<string | null>(null);
  const [shareCode, setShareCode] = useState<string | null>(null);
  const [shareUrl, setShareUrl] = useState<string | null>(null);
  const [shareLoading, setShareLoading] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const [shareTab, setShareTab] = useState<"invite" | "manage">("invite");
  const [shareUsers, setShareUsers] = useState<{ owner?: ShareUser; shares: ShareUser[] }>({ shares: [] });
  const [manageLoading, setManageLoading] = useState(false);
  const [removingUserId, setRemovingUserId] = useState<string | null>(null);

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
      isShared?: boolean;
      ownerEmail?: string;
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
          // Don't trust database status - only trust actual CLI connections
          isActive: false,
          createdAt: session.createdAt,
          isShared: session.isShared,
          ownerEmail: session.ownerEmail,
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

  const handleShareClick = async (e: React.MouseEvent, projectId: string) => {
    e.stopPropagation();
    setShareSessionId(projectId);
    setShareCode(null);
    setShareUrl(null);
    setShareError(null);
    setShareModalOpen(true);
    setShareLoading(true);

    try {
      const response = await fetch(`${API_URL}/share/generate`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ session_id: projectId }),
      });

      if (!response.ok) {
        const error = await response.json();
        throw new Error(error.message || "Failed to generate share code");
      }

      const data = await response.json();
      setShareCode(data.code);
      setShareUrl(data.share_url);
    } catch (err) {
      setShareError(err instanceof Error ? err.message : "Failed to generate share code");
    } finally {
      setShareLoading(false);
    }
  };

  const [copied, setCopied] = useState(false);

  const copyShareLink = () => {
    if (!shareUrl) return;

    // Try modern clipboard API first, fallback to textarea method
    if (navigator.clipboard && window.isSecureContext) {
      navigator.clipboard.writeText(shareUrl).then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      });
    } else {
      // Fallback for HTTP
      const textArea = document.createElement("textarea");
      textArea.value = shareUrl;
      textArea.style.position = "fixed";
      textArea.style.left = "-999999px";
      document.body.appendChild(textArea);
      textArea.focus();
      textArea.select();
      try {
        document.execCommand("copy");
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      } catch (err) {
        console.error("Failed to copy:", err);
      }
      document.body.removeChild(textArea);
    }
  };

  const fetchShareUsers = async (sessionId: string) => {
    setManageLoading(true);
    try {
      const response = await fetch(`${API_URL}/share/list/${sessionId}`, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      });

      if (!response.ok) {
        throw new Error("Failed to fetch share list");
      }

      const data = await response.json();
      setShareUsers({ owner: data.owner, shares: data.shares });
    } catch (err) {
      console.error("Failed to fetch shares:", err);
    } finally {
      setManageLoading(false);
    }
  };

  const handleRemoveUser = async (userId: string) => {
    if (!shareSessionId) return;

    setRemovingUserId(userId);
    try {
      const response = await fetch(`${API_URL}/share/${shareSessionId}/${userId}`, {
        method: "DELETE",
        headers: {
          Authorization: `Bearer ${token}`,
        },
      });

      if (!response.ok) {
        throw new Error("Failed to remove user");
      }

      // Refresh the list
      await fetchShareUsers(shareSessionId);
    } catch (err) {
      console.error("Failed to remove user:", err);
    } finally {
      setRemovingUserId(null);
    }
  };

  const handleTabChange = (tab: "invite" | "manage") => {
    setShareTab(tab);
    if (tab === "manage" && shareSessionId) {
      fetchShareUsers(shareSessionId);
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
              <div
                key={project.id}
                onClick={() => handleProjectClick(project.id, project.isActive)}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left text-sm transition-colors cursor-pointer ${
                  sessionId === project.id
                    ? "bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300"
                    : "hover:bg-gray-200 dark:hover:bg-gray-800"
                }`}
              >
                <div
                  className={`w-2 h-2 rounded-full flex-shrink-0 ${
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
                    {project.isShared ? (
                      <span className="flex items-center gap-1 text-blue-500">
                        <Users className="w-3 h-3" />
                        Shared by {project.ownerEmail}
                      </span>
                    ) : project.isActive ? (
                      "Active"
                    ) : project.createdAt ? (
                      new Date(project.createdAt).toLocaleDateString()
                    ) : (
                      ""
                    )}
                  </div>
                </div>
                {!project.isShared && (
                  <button
                    onClick={(e) => handleShareClick(e, project.id)}
                    className="p-1 hover:bg-gray-300 dark:hover:bg-gray-600 rounded opacity-50 hover:opacity-100 flex-shrink-0"
                    title="Share this session"
                  >
                    <Share2 className="w-4 h-4" />
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Share Modal */}
      {shareModalOpen && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50" onClick={() => setShareModalOpen(false)}>
          <div className="bg-white dark:bg-gray-800 rounded-lg max-w-md w-full mx-4 shadow-xl" onClick={(e) => e.stopPropagation()}>
            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
              <h3 className="text-lg font-semibold">Share Session</h3>
              <button
                onClick={() => setShareModalOpen(false)}
                className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* Tabs */}
            <div className="flex border-b border-gray-200 dark:border-gray-700">
              <button
                onClick={() => handleTabChange("invite")}
                className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
                  shareTab === "invite"
                    ? "border-b-2 border-blue-500 text-blue-600 dark:text-blue-400"
                    : "text-gray-500 hover:text-gray-700 dark:hover:text-gray-300"
                }`}
              >
                Invite
              </button>
              <button
                onClick={() => handleTabChange("manage")}
                className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
                  shareTab === "manage"
                    ? "border-b-2 border-blue-500 text-blue-600 dark:text-blue-400"
                    : "text-gray-500 hover:text-gray-700 dark:hover:text-gray-300"
                }`}
              >
                Manage Access
              </button>
            </div>

            {/* Content */}
            <div className="p-4">
              {shareTab === "invite" ? (
                <>
                  {shareLoading ? (
                    <div className="text-center py-8">
                      <div className="animate-spin w-8 h-8 border-4 border-blue-500 border-t-transparent rounded-full mx-auto"></div>
                      <p className="mt-2 text-gray-500">Generating share link...</p>
                    </div>
                  ) : shareError ? (
                    <div className="text-center py-4">
                      <p className="text-red-500">{shareError}</p>
                    </div>
                  ) : (
                    <div>
                      <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
                        Share this link with someone to give them access. The link expires in 24 hours.
                      </p>

                      <div className="mb-4">
                        <label className="block text-sm font-medium mb-1">Share Code</label>
                        <div className="font-mono text-2xl tracking-wider text-center py-2 bg-gray-100 dark:bg-gray-700 rounded">
                          {shareCode}
                        </div>
                      </div>

                      <div className="mb-4">
                        <label className="block text-sm font-medium mb-1">Share Link</label>
                        <div className="flex gap-2">
                          <input
                            type="text"
                            readOnly
                            value={shareUrl || ""}
                            className="flex-1 px-3 py-2 border rounded bg-gray-50 dark:bg-gray-700 text-sm font-mono"
                          />
                          <button
                            onClick={copyShareLink}
                            className={`px-4 py-2 text-white rounded transition-colors ${
                              copied ? "bg-green-500" : "bg-blue-500 hover:bg-blue-600"
                            }`}
                          >
                            {copied ? "Copied!" : "Copy"}
                          </button>
                        </div>
                      </div>
                    </div>
                  )}
                </>
              ) : (
                <>
                  {manageLoading ? (
                    <div className="text-center py-8">
                      <div className="animate-spin w-8 h-8 border-4 border-blue-500 border-t-transparent rounded-full mx-auto"></div>
                      <p className="mt-2 text-gray-500">Loading users...</p>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      <p className="text-sm text-gray-600 dark:text-gray-400 mb-3">
                        Users with access to this session:
                      </p>

                      {/* Owner */}
                      {shareUsers.owner && (
                        <div className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700 rounded-lg">
                          <div className="flex items-center gap-2">
                            <Crown className="w-4 h-4 text-yellow-500" />
                            <div>
                              <div className="font-medium text-sm">{shareUsers.owner.user_email}</div>
                              <div className="text-xs text-gray-500">Owner</div>
                            </div>
                          </div>
                        </div>
                      )}

                      {/* Shared users */}
                      {shareUsers.shares.length === 0 ? (
                        <div className="text-center py-4 text-gray-500 text-sm">
                          No users have been invited yet
                        </div>
                      ) : (
                        shareUsers.shares.map((user) => (
                          <div
                            key={user.user_id}
                            className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700 rounded-lg"
                          >
                            <div className="flex items-center gap-2">
                              <Users className="w-4 h-4 text-blue-500" />
                              <div>
                                <div className="font-medium text-sm">{user.user_email}</div>
                                <div className="text-xs text-gray-500">
                                  {user.created_at
                                    ? `Shared ${new Date(user.created_at).toLocaleDateString()}`
                                    : "Shared"}
                                </div>
                              </div>
                            </div>
                            <button
                              onClick={() => handleRemoveUser(user.user_id)}
                              disabled={removingUserId === user.user_id}
                              className="p-2 text-red-500 hover:bg-red-100 dark:hover:bg-red-900/30 rounded transition-colors disabled:opacity-50"
                              title="Remove access"
                            >
                              {removingUserId === user.user_id ? (
                                <div className="w-4 h-4 border-2 border-red-500 border-t-transparent rounded-full animate-spin" />
                              ) : (
                                <Trash2 className="w-4 h-4" />
                              )}
                            </button>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
