"use client";

import { useStore } from "@/lib/store";
import { ThemePicker } from "@/components/ThemePicker";
import { FolderOpen, RefreshCw, Share2, Users, X, Crown, Trash2, ChevronLeft, ChevronDown, ChevronRight, BarChart3, Server, Plus, LogOut, ArrowRightLeft, AlertTriangle, MoreHorizontal } from "lucide-react";
import { CreateInstanceModal } from "./CreateInstanceModal";
import Link from "next/link";
import { useMemo, useState, useEffect } from "react";
import { createPortal } from "react-dom";
import {
  paneUsageLimit,
  usageLimitedLabel,
  usageLimitResetLabel,
} from "@/lib/usageLimitStatus";
import { compareRecentlyIdle } from "@/lib/idlePaneOrdering";

// Truncate path in the middle to preserve the folder name at the end
// e.g., "/home/shuai/workspace/long-project" -> "/home/.../long-project"
function truncatePath(path: string, maxLength: number = 22): string {
  if (path.length <= maxLength) return path;

  // Handle absolute paths (starting with /)
  const isAbsolute = path.startsWith('/');
  const parts = path.split('/').filter(p => p.length > 0);
  if (parts.length <= 1) return path;

  // Always keep the last part (folder name)
  const lastPart = parts[parts.length - 1];
  const suffix = `.../${lastPart}`;

  // If even with truncation it's too long, just show .../folder
  if (suffix.length + 1 >= maxLength) {
    return (isAbsolute ? '/' : '') + suffix;
  }

  // Try to fit as much of the beginning as possible
  const prefix = isAbsolute ? '/' : '';
  let result = prefix + parts[0];
  const fullSuffix = `/.../${lastPart}`;
  const availableLength = maxLength - fullSuffix.length;

  // Add more path segments from the start if they fit
  for (let i = 1; i < parts.length - 1; i++) {
    const next = result + '/' + parts[i];
    if (next.length > availableLength) break;
    result = next;
  }

  return result + fullSuffix;
}

interface SidebarProps {
  onClose?: () => void;
  onCollapse?: () => void;
  width?: number;
}

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
type ProjectRole = "owner" | "user";

// Group key + localStorage key for collapsed repo groups in the sidebar.
const NO_REMOTE_KEY = "__no_remote__";
type SidebarView = "projects" | "idle" | "limited";

/// The same name the project list shows, so an agent row and its project agree.
function projectNameFor(session: { workingDir?: string; projectId?: string; id: string }): string {
  return session.workingDir?.split("/").pop()
    || `Project ${(session.projectId || session.id).slice(0, 8)}`;
}

/// Matches the session screen's fallback so one pane reads the same everywhere.
function paneRowLabel(pane: { label?: string | null; kind: string; pane_id: number }): string {
  return pane.label?.trim() || `${pane.kind === "terminal" ? "Terminal" : "Pane"} ${pane.pane_id}`;
}
const COLLAPSED_GROUPS_STORAGE_KEY = "apas_collapsed_repo_groups";

// Turn a canonical `host/owner/repo` remote into a compact header label.
// GitHub repos drop the host (`github.com/shuaimu/apas` -> `shuaimu/apas`);
// other hosts keep it so self-hosted/GitLab repos stay distinguishable.
function repoDisplayLabel(remote: string): string {
  return remote.startsWith("github.com/")
    ? remote.slice("github.com/".length)
    : remote;
}

function parseProjectRole(raw: unknown): ProjectRole {
  if (typeof raw !== "string") return "user";
  const normalized = raw.trim().toLowerCase();
  return normalized === "owner" ? "owner" : "user";
}

function projectRole(project: { isShared?: boolean; shareRole?: ProjectRole }): ProjectRole {
  return project.shareRole ?? (project.isShared ? "user" : "owner");
}

function canAdminActOnRole(viewerRole: ProjectRole, _targetRole: ProjectRole): boolean {
  return viewerRole === "owner";
}

function roleLabel(role: ProjectRole): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

interface ShareUser {
  user_id: string;
  user_email: string;
  is_owner: boolean;
  role: ProjectRole;
  created_at?: string;
}

interface ShareListState {
  owner?: ShareUser;
  shares: ShareUser[];
  viewerRole: ProjectRole;
  canManage: boolean;
}

export function Sidebar({ onClose, onCollapse, width }: SidebarProps) {
  const { cliClients, sessions, machines, usageLimits, attachSession, openSessionPane, forgetProject, refreshCliClients, listSessions, sessionId, connected, token } = useStore();
  // Which list the sidebar is showing. Projects answer "where do I want to go";
  // idle agents answer "who is waiting for me", which the project view cannot
  // express — a project with one busy pane reads as working, hiding the rest.
  const [view, setView] = useState<SidebarView>("projects");
  const unreadSessions = useStore((s) => s.unreadSessions);
  const [shareModalOpen, setShareModalOpen] = useState(false);
  const [mounted, setMounted] = useState(false);
  const [availabilityNow, setAvailabilityNow] = useState(() => Date.now());

  // For portal - need to wait for client-side mount
  useEffect(() => {
    setMounted(true);
  }, []);
  useEffect(() => {
    const timer = window.setInterval(() => setAvailabilityNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  const [shareSessionId, setShareSessionId] = useState<string | null>(null);
  const [shareProjectId, setShareProjectId] = useState<string | null>(null);
  const [shareCode, setShareCode] = useState<string | null>(null);
  const [shareUrl, setShareUrl] = useState<string | null>(null);
  const [shareLoading, setShareLoading] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [shareTab, setShareTab] = useState<"invite" | "manage">("invite");
  const [shareUsers, setShareUsers] = useState<ShareListState>({
    shares: [],
    viewerRole: "user",
    canManage: false,
  });
  const [manageLoading, setManageLoading] = useState(false);
  const [removingUserId, setRemovingUserId] = useState<string | null>(null);
  const [transferringUserId, setTransferringUserId] = useState<string | null>(null);
  const [lifecycleLoading, setLifecycleLoading] = useState(false);
  const [deleteConfirmation, setDeleteConfirmation] = useState("");
  const [deletionInProgress, setDeletionInProgress] = useState(false);

  // Merge CLI clients (active) and sessions (historical) into unified project list.
  // Deduplicate by project_id (the stable .apas id) so moving a project directory
  // doesn't show up as a second project. Falls back to id for legacy rows.
  const projects = useMemo(() => {
    const projectMap = new Map<string, {
      id: string;
      projectId: string;
      name: string;
      workingDir: string;
      hostname?: string;
      gitRemote?: string;
      gitRemoteUrl?: string;
      isActive: boolean;
      createdAt?: string;
      isShared?: boolean;
      ownerEmail?: string;
      shareRole?: ProjectRole;
      cliClientId?: string;
    }>();

    // Sort sessions by date (newest first) so we keep the most recent per project
    const sortedSessions = [...sessions].sort((a, b) => {
      if (a.createdAt && b.createdAt) {
        return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
      }
      return 0;
    });

    // Add sessions, deduplicating by project_id
    // Active sessions take precedence over inactive ones for the same project
    for (const session of sortedSessions) {
      const projectKey = session.projectId || session.id;
      const workingDir = session.workingDir || session.id;
      const name = projectNameFor(session);

      const existing = projectMap.get(projectKey);
      if (!existing || (session.isActive && !existing.isActive)) {
        projectMap.set(projectKey, {
          id: session.id,
          projectId: projectKey,
          name,
          workingDir,
          hostname: session.hostname,
          gitRemote: session.gitRemote,
          gitRemoteUrl: session.gitRemoteUrl,
          isActive: session.isActive || false,
          createdAt: session.createdAt,
          isShared: session.isShared,
          ownerEmail: session.ownerEmail,
          shareRole: session.shareRole,
          cliClientId: session.cliClientId,
        });
      }
    }

    // Also mark projects as active if current user has a connected CLI client
    // (this handles the case where server hasn't refreshed yet)
    for (const client of cliClients) {
      if (client.activeSession) {
        for (const project of projectMap.values()) {
          if (project.id === client.activeSession) {
            project.isActive = true;
            project.cliClientId = client.id;
            break;
          }
        }
      }
    }

    // Also mark projects as active if daemon reports them as running.
    // Daemon's project_id is the .apas id, so match against the project key.
    for (const machine of machines) {
      for (const mp of machine.projects) {
        if (mp.isRunning) {
          const project = projectMap.get(mp.projectId);
          if (project) {
            project.isActive = true;
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
  }, [cliClients, sessions, machines]);

  // Group the deduped projects by the repo they belong to. Always emit a header
  // per group (including the "(no remote)" bucket). Named-repo groups keep the
  // activity/recency order inherited from `projects` (Array#sort is stable, so
  // returning 0 preserves first-seen order); the no-remote bucket sinks last.
  const waitingAgents = useMemo(
    () => sessions
      // A stopped project's agents are not idle, they are not running.
      .filter((session) => session.isActive)
      .flatMap((session) =>
        (session.panes ?? [])
          .filter((pane) => !pane.is_working)
          .map((pane) => ({
            session,
            pane,
            usageLimit: paneUsageLimit(session, pane, usageLimits, availabilityNow),
          })),
      ),
    [availabilityNow, sessions, usageLimits],
  );
  const idleAgents = useMemo(
    () => waitingAgents
      .filter((agent) => !agent.usageLimit)
      .sort(compareRecentlyIdle),
    [waitingAgents],
  );
  const limitedAgents = useMemo(
    () => waitingAgents.filter((agent) => agent.usageLimit),
    [waitingAgents],
  );
  const visibleAgents = view === "limited" ? limitedAgents : idleAgents;

  const repoGroups = useMemo(() => {
    const byKey = new Map<
      string,
      { key: string; label: string; isNoRemote: boolean; cloneUrl?: string; projects: typeof projects }
    >();
    for (const project of projects) {
      const key = project.gitRemote ?? NO_REMOTE_KEY;
      let group = byKey.get(key);
      if (!group) {
        group = {
          key,
          label: project.gitRemote ? repoDisplayLabel(project.gitRemote) : "(no remote)",
          isNoRemote: !project.gitRemote,
          projects: [],
        };
        byKey.set(key, group);
      }
      // Remember a representative clone URL from any project in the group.
      if (!group.cloneUrl && project.gitRemoteUrl) {
        group.cloneUrl = project.gitRemoteUrl;
      }
      group.projects.push(project);
    }
    return Array.from(byKey.values()).sort((a, b) => {
      if (a.isNoRemote !== b.isNoRemote) return a.isNoRemote ? 1 : -1;
      return 0;
    });
  }, [projects]);

  // Collapsed repo groups, persisted to localStorage so the choice survives
  // reloads and the `repoGroups` recompute. Groups default to expanded.
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => {
    if (typeof window === "undefined") return new Set();
    try {
      const raw = window.localStorage.getItem(COLLAPSED_GROUPS_STORAGE_KEY);
      return new Set<string>(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
      return new Set();
    }
  });

  const toggleGroup = (key: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      try {
        window.localStorage.setItem(
          COLLAPSED_GROUPS_STORAGE_KEY,
          JSON.stringify(Array.from(next)),
        );
      } catch {
        // Ignore storage failures (private mode, quota).
      }
      return next;
    });
  };

  // Repo whose "New instance" modal is open (null = closed).
  const [newInstanceRepo, setNewInstanceRepo] = useState<
    { gitRemote: string; cloneUrl?: string } | null
  >(null);

  const handleRefresh = () => {
    if (isRefreshing) return;
    setIsRefreshing(true);
    refreshCliClients();
    listSessions();
    // Clear refreshing state after a short delay
    setTimeout(() => {
      setIsRefreshing(false);
    }, 1000);
  };

  const handleProjectClick = (projectId: string) => {
    // Always attach so the server's per-connection session_id stays in sync
    // with the project the user is viewing. Branching on a stale isActive
    // could leave the server routing input to a previously-attached session,
    // surfacing as "Pane worker unavailable" when the wrong CLI receives a
    // pane_id it doesn't know.
    attachSession(projectId);
    // Close sidebar on mobile after selecting a project
    onClose?.();
  };

  const handleShareClick = async (
    e: React.MouseEvent,
    representativeSessionId: string,
    canonicalProjectId: string,
    role: ProjectRole,
  ) => {
    e.stopPropagation();
    setShareSessionId(representativeSessionId);
    setShareProjectId(canonicalProjectId);
    setShareTab(role === "owner" ? "invite" : "manage");
    setShareCode(null);
    setShareUrl(null);
    setShareError(null);
    setShareUsers({
      owner: undefined,
      shares: [],
      viewerRole: role,
      canManage: role === "owner",
    });
    setDeleteConfirmation("");
    setDeletionInProgress(false);
    setShareModalOpen(true);
    if (role !== "owner") return;
    setShareLoading(true);

    try {
      const response = await fetch(`${API_URL}/share/generate`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ session_id: representativeSessionId }),
      });

      if (!response.ok) {
        const error = await response.json();
        throw new Error(error.error || error.message || "Failed to generate share code");
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
    setShareError(null);
    try {
      const response = await fetch(`${API_URL}/share/list/${sessionId}`, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      });

      if (!response.ok) {
        const error = await response.json().catch(() => null);
        throw new Error(error?.message || "Failed to fetch share list");
      }

      const data = await response.json();
      const owner: ShareUser | undefined = data.owner
        ? {
            user_id: data.owner.user_id,
            user_email: data.owner.user_email,
            is_owner: true,
            role: "owner",
            created_at: data.owner.created_at,
          }
        : undefined;
      const shares: ShareUser[] = Array.isArray(data.shares)
        ? data.shares.map((u: Record<string, unknown>) => ({
            user_id: String(u.user_id || ""),
            user_email: String(u.user_email || ""),
            is_owner: false,
            role: parseProjectRole(u.role),
            created_at: typeof u.created_at === "string" ? u.created_at : undefined,
          }))
        : [];
      setShareUsers({
        owner,
        shares,
        viewerRole: parseProjectRole(data.viewer_role),
        canManage: Boolean(data.can_manage),
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to fetch share list";
      console.error("Failed to fetch shares:", message);
      setShareError(message);
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
        const error = await response.json().catch(() => null);
        throw new Error(error?.message || "Failed to remove user");
      }

      // Refresh the list
      await fetchShareUsers(shareSessionId);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to remove user";
      console.error("Failed to remove user:", message);
      setShareError(message);
    } finally {
      setRemovingUserId(null);
    }
  };

  const handleTransferOwner = async (user: ShareUser) => {
    if (!shareProjectId) return;
    if (!window.confirm(
      `Transfer this project to ${user.user_email}? You will become an ordinary project user.`,
    )) return;
    setTransferringUserId(user.user_id);
    setShareError(null);
    try {
      const response = await fetch(`${API_URL}/projects/${shareProjectId}/owner`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ user_id: user.user_id }),
      });
      if (!response.ok) {
        const error = await response.json().catch(() => null);
        throw new Error(error?.error || error?.message || "Failed to transfer ownership");
      }
      setShareModalOpen(false);
      listSessions();
    } catch (err) {
      setShareError(err instanceof Error ? err.message : "Failed to transfer ownership");
    } finally {
      setTransferringUserId(null);
    }
  };

  const handleLeaveProject = async () => {
    if (!shareProjectId) return;
    if (!window.confirm(
      "Leave this project? You will immediately lose access to every instance and its history.",
    )) return;
    setLifecycleLoading(true);
    setShareError(null);
    try {
      const response = await fetch(`${API_URL}/projects/${shareProjectId}/members/me`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      });
      if (!response.ok) {
        const error = await response.json().catch(() => null);
        throw new Error(error?.error || error?.message || "Failed to leave project");
      }
      forgetProject(shareProjectId);
      setShareModalOpen(false);
      listSessions();
    } catch (err) {
      setShareError(err instanceof Error ? err.message : "Failed to leave project");
    } finally {
      setLifecycleLoading(false);
    }
  };

  const waitForDeletion = async (projectId: string) => {
    for (let attempt = 0; attempt < 120; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      const response = await fetch(`${API_URL}/projects/${projectId}/deletion`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (response.status === 404) {
        forgetProject(projectId);
        setShareModalOpen(false);
        listSessions();
        return;
      }
      if (!response.ok && response.status !== 409) {
        const error = await response.json().catch(() => null);
        throw new Error(error?.error || error?.message || "Failed to check deletion status");
      }
    }
    throw new Error("Deletion is still in progress. You can safely close this dialog.");
  };

  const handleDeleteProject = async () => {
    if (!shareProjectId || deleteConfirmation !== shareProjectId) return;
    setLifecycleLoading(true);
    setShareError(null);
    try {
      const response = await fetch(`${API_URL}/projects/${shareProjectId}/delete`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ confirmation: deleteConfirmation }),
      });
      if (response.status !== 202) {
        const error = await response.json().catch(() => null);
        throw new Error(error?.error || error?.message || "Failed to delete project");
      }
      setDeletionInProgress(true);
      setLifecycleLoading(false);
      await waitForDeletion(shareProjectId);
    } catch (err) {
      setShareError(err instanceof Error ? err.message : "Failed to delete project");
      setLifecycleLoading(false);
    }
  };

  const handleTabChange = (tab: "invite" | "manage") => {
    setShareTab(tab);
    if (tab === "manage" && shareSessionId && shareUsers.viewerRole === "owner") {
      fetchShareUsers(shareSessionId);
    }
  };

  return (
    <div className="w-full h-full border-r border-gray-200 dark:border-gray-800 flex flex-col bg-gray-50 dark:bg-gray-900">
      {/* Header */}
      <div className="p-4 border-b border-gray-200 dark:border-gray-800 flex-shrink-0">
        <div className="flex items-center justify-between">
          <h2 className="font-semibold text-sm text-gray-600 dark:text-gray-400">
            Projects
          </h2>
          <div className="flex items-center gap-1">
            <button
              onClick={handleRefresh}
              disabled={!connected || isRefreshing}
              className={`p-1 rounded disabled:opacity-50 ${
                isRefreshing
                  ? "bg-blue-100 dark:bg-blue-900/30"
                  : "hover:bg-gray-200 dark:hover:bg-gray-700"
              }`}
              title={isRefreshing ? "Refreshing..." : "Refresh"}
            >
              <RefreshCw className={`w-4 h-4 ${isRefreshing ? "animate-spin text-blue-500" : ""}`} />
            </button>
            {/* Collapse button - only visible on desktop */}
            {onCollapse && (
              <button
                onClick={onCollapse}
                className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded hidden md:block"
                title="Collapse sidebar"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
            )}
            {/* Close button - only visible on mobile */}
            {onClose && (
              <button
                onClick={onClose}
                className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded md:hidden"
                title="Close sidebar"
              >
                <X className="w-4 h-4" />
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Which question the list is answering */}
      <div className="flex gap-1 border-b border-gray-200 px-2 py-2 dark:border-gray-800">
        {([
          ["projects", "All projects"],
          ["idle", "Idle sessions"],
          ["limited", "Usage limited"],
        ] as const).map(([key, label]) => (
          <button
            key={key}
            type="button"
            aria-pressed={view === key}
            onClick={() => setView(key)}
            className={`flex-1 rounded px-2 py-1 text-xs font-semibold ${
              view === key
                ? "bg-white text-gray-900 shadow-sm dark:bg-gray-800 dark:text-gray-100"
                : "text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {view === "idle" || view === "limited" ? (
        <div className="flex-1 overflow-y-auto p-2">
          {visibleAgents.length > 0 ? (
            <div className="space-y-1.5">
              {visibleAgents.map(({ session, pane, usageLimit }) => {
                const name = projectNameFor(session);
                const resetLabel = usageLimit ? usageLimitResetLabel(usageLimit, availabilityNow) : null;
                return (
                  <button
                    key={`${session.id}:${pane.pane_id}`}
                    type="button"
                    aria-label={`Open ${paneRowLabel(pane)} in ${name}`}
                    onClick={() => {
                      // Names an agent, so it opens that agent rather than
                      // whichever tab the project was last left on.
                      openSessionPane(session.id, pane.pane_id);
                      onClose?.();
                    }}
                    className="w-full rounded-lg border border-gray-200 bg-white p-2.5 text-left hover:border-gray-300 dark:border-gray-800 dark:bg-gray-800/60 dark:hover:border-gray-700"
                  >
                    {/* Project first, then the agent, both emphasised: the
                        project places the row and the agent is what you are
                        opening. The host is the only detail that can be quiet. */}
                    <div className="flex items-center justify-between gap-2">
                      <span className="flex min-w-0 flex-1 items-baseline gap-1 truncate text-sm">
                        <span className="shrink-0 truncate font-semibold">{name}</span>
                        <span aria-hidden="true" className="shrink-0 text-gray-400 dark:text-gray-600">/</span>
                        <span className="min-w-0 truncate font-semibold text-indigo-600 dark:text-indigo-400">{paneRowLabel(pane)}</span>
                      </span>
                      <span className={`shrink-0 rounded-full px-2 py-0.5 text-[0.65rem] font-semibold ${
                        usageLimit
                          ? "bg-red-100 text-red-700 dark:bg-red-950/60 dark:text-red-300"
                          : "bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-300"
                      }`}>
                        {usageLimit ? usageLimitedLabel(usageLimit) : "Idle"}
                      </span>
                    </div>
                    {(session.hostname || resetLabel) && (
                      <div className="mt-1 truncate text-xs text-gray-500 dark:text-gray-400">
                        {[session.hostname, resetLabel].filter(Boolean).join(" · ")}
                      </div>
                    )}
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="py-8 text-center text-sm text-gray-400">
              <FolderOpen className="mx-auto mb-2 h-8 w-8 opacity-50" />
              <p>{view === "limited" ? "No usage-limited sessions" : "No idle sessions"}</p>
              <p className="mt-1 text-xs">
                {sessions.length
                  ? view === "limited"
                    ? "No provider is currently blocking an agent."
                    : "Every available agent that reported in is working."
                  : "Run `apas` in a directory to start"}
              </p>
            </div>
          )}
        </div>
      ) : (
      <div className="flex-1 overflow-y-auto p-2">
        {projects.length === 0 ? (
          <div className="text-center text-gray-400 text-sm py-8">
            <FolderOpen className="w-8 h-8 mx-auto mb-2 opacity-50" />
            <p>No projects yet</p>
            <p className="text-xs mt-1">Run `apas` in a directory to start</p>
          </div>
        ) : (
          <div className="space-y-2">
            {repoGroups.map((group) => {
              const collapsed = collapsedGroups.has(group.key);
              return (
                <div key={group.key}>
                  <div className="flex items-center gap-1 px-1 py-1">
                    <button
                      onClick={() => toggleGroup(group.key)}
                      className="flex flex-1 min-w-0 items-center gap-1 text-xs font-semibold text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 select-none"
                      title={group.isNoRemote ? "Projects with no git remote" : group.key}
                    >
                      {collapsed ? (
                        <ChevronRight className="w-3.5 h-3.5 flex-shrink-0" />
                      ) : (
                        <ChevronDown className="w-3.5 h-3.5 flex-shrink-0" />
                      )}
                      <span className="truncate flex-1 text-left">{group.label}</span>
                      <span className="font-normal text-gray-400 dark:text-gray-500">
                        {group.projects.length}
                      </span>
                    </button>
                    {!group.isNoRemote && (
                      <button
                        onClick={() =>
                          setNewInstanceRepo({ gitRemote: group.key, cloneUrl: group.cloneUrl })
                        }
                        className="flex-shrink-0 rounded p-0.5 text-gray-400 hover:bg-gray-200 hover:text-emerald-600 dark:hover:bg-gray-700"
                        title={`Create a new instance under ${group.label}`}
                      >
                        <Plus className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                  {!collapsed && (
                    <div className="mt-1 ml-2 pl-1.5 border-l border-gray-200 dark:border-gray-800 space-y-1">
                      {group.projects.map((project) => (
              <div key={project.id}>
                <div
                  onClick={() => handleProjectClick(project.id)}
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
                    <div className="font-medium overflow-hidden whitespace-nowrap flex items-center gap-1.5" title={project.workingDir}>
                      <span className="truncate">
                        {truncatePath(project.workingDir, width ? Math.max(12, Math.floor((width - 56) / 7.5)) : 22)}
                      </span>
                      {unreadSessions.has(project.id) && sessionId !== project.id && (
                        <span
                          className="inline-block w-2 h-2 rounded-full bg-blue-500 flex-shrink-0 animate-pulse"
                          title="New activity since you last viewed this session"
                        />
                      )}
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
                  <button
                    onClick={(e) => handleShareClick(
                      e,
                      project.id,
                      project.projectId,
                      projectRole(project),
                    )}
                    className="p-1 hover:bg-gray-300 dark:hover:bg-gray-600 rounded opacity-50 hover:opacity-100 flex-shrink-0"
                    title={projectRole(project) === "owner" ? "Manage project access" : "Project actions"}
                  >
                    {projectRole(project) === "owner" ? (
                      <Share2 className="w-4 h-4" />
                    ) : (
                      <MoreHorizontal className="w-4 h-4" />
                    )}
                  </button>
                </div>
              </div>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
      )}

      {/* Every account administers its own virtual cluster here: the machines
          it registered and the projects hosted on them. System administration
          is a separate surface with its own login, deliberately unlinked. */}
      <div className="border-t border-gray-200 dark:border-gray-800 p-2 flex-shrink-0">
        <Link
          href="/machines"
          className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-800 transition-colors"
        >
          <Server className="w-4 h-4" />
          My Cluster
        </Link>
      </div>

      {/* Share Modal - rendered via portal to escape transform context */}
      {shareModalOpen && mounted && createPortal(
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[100]" onClick={() => setShareModalOpen(false)}>
          <div className="bg-white dark:bg-gray-800 rounded-lg max-w-md w-full mx-4 shadow-xl" onClick={(e) => e.stopPropagation()}>
            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
              <h3 className="text-lg font-semibold">Project Access</h3>
              <button
                onClick={() => setShareModalOpen(false)}
                className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* Tabs */}
            <div className="flex border-b border-gray-200 dark:border-gray-700">
              {shareUsers.viewerRole === "owner" && (
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
              )}
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
              ) : shareUsers.viewerRole === "user" ? (
                <div className="space-y-4">
                  <div>
                    <h4 className="font-medium">Leave project</h4>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      You will immediately lose access to every project instance and its history.
                    </p>
                  </div>
                  {shareError && <p className="text-sm text-red-500">{shareError}</p>}
                  <button
                    onClick={handleLeaveProject}
                    disabled={lifecycleLoading}
                    className="flex w-full items-center justify-center gap-2 rounded bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50"
                  >
                    <LogOut className="h-4 w-4" />
                    {lifecycleLoading ? "Leaving…" : "Leave project"}
                  </button>
                </div>
              ) : (
                <>
                  {manageLoading ? (
                    <div className="text-center py-8">
                      <div className="animate-spin w-8 h-8 border-4 border-blue-500 border-t-transparent rounded-full mx-auto"></div>
                      <p className="mt-2 text-gray-500">Loading users...</p>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      <p className="text-sm text-gray-600 dark:text-gray-400 mb-1">
                        Users with access to this session:
                      </p>
                      <p className="text-xs text-gray-500 mb-3">
                        Your role: {roleLabel(shareUsers.viewerRole)}
                      </p>
                      {shareError && (
                        <p className="text-sm text-red-500 mb-3">{shareError}</p>
                      )}
                      {/* Owner */}
                      {shareUsers.owner && (
                        <div className="flex items-center justify-between gap-3 p-3 bg-gray-50 dark:bg-gray-700 rounded-lg">
                          <div className="flex items-center gap-2 min-w-0">
                            <Crown className="w-4 h-4 text-yellow-500 flex-shrink-0" />
                            <div className="min-w-0">
                              <div className="font-medium text-sm truncate">{shareUsers.owner.user_email}</div>
                              <div className="text-xs text-gray-500">Owner</div>
                            </div>
                          </div>
                          <span className="text-xs px-2 py-1 rounded bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-300">
                            Owner
                          </span>
                        </div>
                      )}

                      {/* Shared users */}
                      {shareUsers.shares.length === 0 ? (
                        <div className="text-center py-4 text-gray-500 text-sm">
                          No users have been invited yet
                        </div>
                      ) : (
                        shareUsers.shares.map((user) => {
                          const canActOnUser = shareUsers.canManage && canAdminActOnRole(shareUsers.viewerRole, user.role);
                          const removePending = removingUserId === user.user_id;

                          return (
                            <div
                              key={user.user_id}
                              className="flex items-center justify-between gap-3 p-3 bg-gray-50 dark:bg-gray-700 rounded-lg"
                            >
                              <div className="flex items-center gap-2 min-w-0">
                                <Users className="w-4 h-4 text-blue-500 flex-shrink-0" />
                                <div className="min-w-0">
                                  <div className="font-medium text-sm truncate">{user.user_email}</div>
                                  <div className="text-xs text-gray-500">
                                    {user.created_at
                                      ? `Shared ${new Date(user.created_at).toLocaleDateString()}`
                                      : "Shared"}
                                  </div>
                                </div>
                              </div>
                              <div className="flex items-center gap-2">
                                <span className="text-xs px-2 py-1 rounded bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300">
                                  User
                                </span>
                                <button
                                  onClick={() => handleTransferOwner(user)}
                                  disabled={!canActOnUser || transferringUserId !== null}
                                  className="p-2 text-amber-600 hover:bg-amber-100 dark:hover:bg-amber-900/30 rounded transition-colors disabled:opacity-50"
                                  title="Transfer ownership"
                                >
                                  {transferringUserId === user.user_id ? (
                                    <div className="w-4 h-4 border-2 border-amber-600 border-t-transparent rounded-full animate-spin" />
                                  ) : (
                                    <ArrowRightLeft className="w-4 h-4" />
                                  )}
                                </button>
                                <button
                                  onClick={() => handleRemoveUser(user.user_id)}
                                  disabled={!canActOnUser || removePending}
                                  className="p-2 text-red-500 hover:bg-red-100 dark:hover:bg-red-900/30 rounded transition-colors disabled:opacity-50"
                                  title="Remove access"
                                >
                                  {removePending ? (
                                    <div className="w-4 h-4 border-2 border-red-500 border-t-transparent rounded-full animate-spin" />
                                  ) : (
                                    <Trash2 className="w-4 h-4" />
                                  )}
                                </button>
                              </div>
                            </div>
                          );
                        })
                      )}

                      <div className="mt-6 border-t border-red-200 pt-4 dark:border-red-900/60">
                        <div className="flex items-center gap-2 text-red-600 dark:text-red-400">
                          <AlertTriangle className="h-4 w-4" />
                          <h4 className="font-semibold">Danger zone</h4>
                        </div>
                        <p className="mt-2 text-xs text-gray-600 dark:text-gray-400">
                          Permanently deletes APAS project sessions, messages, terminal state, invitations, and audit records. Type the canonical project ID to confirm:
                        </p>
                        <code className="mt-2 block break-all rounded bg-gray-100 p-2 text-xs dark:bg-gray-900">
                          {shareProjectId}
                        </code>
                        <input
                          aria-label="Project deletion confirmation"
                          value={deleteConfirmation}
                          onChange={(event) => setDeleteConfirmation(event.target.value)}
                          disabled={deletionInProgress}
                          className="mt-2 w-full rounded border border-red-300 bg-transparent px-3 py-2 text-sm dark:border-red-800"
                          placeholder="Type the project ID"
                        />
                        <button
                          onClick={handleDeleteProject}
                          disabled={
                            lifecycleLoading ||
                            deletionInProgress ||
                            !shareProjectId ||
                            deleteConfirmation !== shareProjectId
                          }
                          className="mt-2 flex w-full items-center justify-center gap-2 rounded bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50"
                        >
                          <Trash2 className="h-4 w-4" />
                          {deletionInProgress
                            ? "Deletion in progress…"
                            : lifecycleLoading
                              ? "Starting deletion…"
                              : "Permanently delete project"}
                        </button>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>
          </div>
        </div>,
        document.body
      )}

      {newInstanceRepo && (
        <CreateInstanceModal
          key={newInstanceRepo.gitRemote}
          open
          onClose={() => setNewInstanceRepo(null)}
          gitRemote={newInstanceRepo.gitRemote}
          cloneUrl={newInstanceRepo.cloneUrl}
        />
      )}
      {/* Sidebar footer: low-traffic, always-visible chrome. */}
      <div className="mt-auto flex-shrink-0 border-t border-gray-200 p-3 dark:border-gray-800">
        <ThemePicker />
      </div>
    </div>
  );
}
