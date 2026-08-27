"use client";

/**
 * The account's own virtual cluster: the machines its clients registered, plus
 * every project hosted on them — including projects another account owns. This
 * needs no special authority; the server scopes each request to the caller's
 * own cluster. Deployment-wide administration is a separate surface with its
 * own login and is deliberately not linked from anywhere in the app.
 *
 * The route stays /machines so existing links and bookmarks keep working.
 */

import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Copy, FolderOpen, Play, Plus, RefreshCw, RotateCcw, Square } from "lucide-react";
import { useStore } from "@/lib/store";
import { CreateInstanceModal } from "@/components/CreateInstanceModal";
import { AllProvidersUsage } from "@/components/UsageLimits";
import { EffectivePolicy, LaunchProfile, PolicyEditor } from "@/components/PolicyEditor";
import { isRetiredLaunchProfileKey } from "@/lib/providerOptions";
import {
  daemonVersionLabel,
  isMachineBehind,
  latestSeenVersion,
  rebootActionLabelFor,
  rebootLabelFor,
} from "@/lib/daemonVersion";

const DEEPSEEK_API_BASE_URL = "https://api.deepseek.com/anthropic";
const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
const MOBILE_CLUSTER_STORAGE_KEY = "apas_mobile_cluster_owner";

interface Page<T> { items: T[]; limit: number; offset: number }
interface ClusterProject {
  id: string;
  project_name?: string | null;
  hostname?: string | null;
  owner_user_id: string;
  owner_email: string;
  lifecycle_status: "active" | "suspended";
  member_count: number;
  active_session_count: number;
  hosting_emails: string[];
  last_activity?: string;
  connected: boolean;
  effective_policy: EffectivePolicy;
}
interface ProjectMember { user_id: string; email: string; created_at?: string }
interface ClusterProjectDetail {
  project: Omit<ClusterProject, "connected" | "effective_policy">;
  members: ProjectMember[];
  policy: EffectivePolicy;
}
interface ClusterPolicy {
  cluster: {
    user_id: string;
    team_available: boolean | null;
    allowed_launch_profiles: string[] | null;
    version: number;
  } | null;
  deployment: EffectivePolicy;
}
interface AuditEvent {
  id: number;
  actor_kind: string;
  actor_user_id: string;
  action: string;
  target_type: string;
  target_id: string;
  details?: string;
  created_at?: string;
}
interface ClusterReference {
  owner_user_id: string;
  owner_email: string;
  access: "owner" | "member";
  accepted_at?: string | null;
}
interface ClusterInvitation {
  id: string;
  invitee_email: string;
  expires_at: string;
  status: "pending" | "accepted" | "expired" | "revoked";
}
interface ClusterMembership {
  user_id: string;
  user_email: string;
  status: string;
  accepted_at?: string | null;
}
interface UsageCounters {
  prompts: number;
  responses: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  cost_usd_reported: boolean;
}
interface ClusterUsageReport {
  lifetime: UsageCounters;
  last_7d: UsageCounters;
  today: UsageCounters;
  projects: Array<{
    project_id: string;
    project_name?: string | null;
    owner_email: string;
    usage: { lifetime: UsageCounters; last_7d: UsageCounters; today: UsageCounters };
  }>;
}

const TRUST_WARNING = "Projects run on the cluster owner's machines. The owner can access files, processes, terminal output, and credentials exposed to those processes. Join only a cluster owner you trust.";

function formatMemory(memoryKb?: number): string {
  if (memoryKb == null) return "";
  if (memoryKb >= 1024 * 1024) return ` · ${(memoryKb / (1024 * 1024)).toFixed(1)} GiB`;
  if (memoryKb >= 1024) return ` · ${Math.round(memoryKb / 1024)} MiB`;
  return ` · ${memoryKb} KiB`;
}

export default function MachinesPage() {
  const router = useRouter();
  const {
    connected,
    connect,
    token,
    machines,
    listMachines,
    startMachineProjectCli,
    stopMachineProjectCli,
    setMachineDeepseekConfig,
    pendingInstances,
    rebootDaemon,
    serverVersion,
    userId,
    userEmail,
  } = useStore();
  const [deepseekDrafts, setDeepseekDrafts] = useState<Record<string, string>>({});
  const [deepseekSaved, setDeepseekSaved] = useState<Record<string, boolean>>({});
  const [clusterProjects, setClusterProjects] = useState<ClusterProject[]>([]);
  const [selected, setSelected] = useState<ClusterProjectDetail | null>(null);
  const [selectedUsage, setSelectedUsage] = useState<{ lifetime: UsageCounters; last_7d: UsageCounters; today: UsageCounters } | null>(null);
  const [clusterPolicy, setClusterPolicy] = useState<ClusterPolicy | null>(null);
  const [profiles, setProfiles] = useState<LaunchProfile[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [clusterError, setClusterError] = useState<string | null>(null);
  const [rebootTarget, setRebootTarget] =
    useState<{ id: string; hostname: string; behind: boolean } | null>(null);
  const [memberUserId, setMemberUserId] = useState("");
  const [ownerUserId, setOwnerUserId] = useState("");
  const [clusters, setClusters] = useState<ClusterReference[]>([]);
  const [selectedClusterId, setSelectedClusterId] = useState("");
  const [invitations, setInvitations] = useState<ClusterInvitation[]>([]);
  const [clusterMembers, setClusterMembers] = useState<ClusterMembership[]>([]);
  const [usage, setUsage] = useState<ClusterUsageReport | null>(null);
  const [usageWindow, setUsageWindow] = useState<"today" | "last_7d" | "lifetime">("last_7d");
  const [inviteEmail, setInviteEmail] = useState("");
  const [trustConfirmed, setTrustConfirmed] = useState(false);
  const [createdInviteLink, setCreatedInviteLink] = useState("");
  const [createOpen, setCreateOpen] = useState(false);

  const selectedCluster = clusters.find((cluster) => cluster.owner_user_id === selectedClusterId);
  const sharedView = selectedCluster?.access === "member";
  const visibleMachines = useMemo(
    () => machines.filter((entry) => {
      if (!selectedClusterId) return entry.clusterAccess !== "member";
      if (entry.clusterOwnerUserId) return entry.clusterOwnerUserId === selectedClusterId;
      return !sharedView;
    }),
    [machines, selectedClusterId, sharedView],
  );

  // Both sources, for the same reason the mobile list uses both: the server
  // catches a fleet uniformly behind a newer deployment, the machines catch a
  // rollout that has reached some hosts and not others.
  const latestVersion = useMemo(
    () =>
      latestSeenVersion([
        serverVersion,
        ...visibleMachines.map(({ machine }) => machine.daemonVersion),
      ]),
    [serverVersion, visibleMachines],
  );

  useEffect(() => {
    const storedToken = localStorage.getItem("apas_token");
    if (!storedToken && !token) {
      router.push("/login");
      return;
    }
    if (!connected) {
      connect();
      return;
    }
    listMachines();
  }, [connected, connect, listMachines, router, token]);

  // Cluster state rides the HTTP control plane rather than the WebSocket: it
  // is request/response administration, not live pane traffic.
  const api = useCallback(async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const authToken = token || (typeof window !== "undefined" ? localStorage.getItem("apas_token") : null);
    const response = await fetch(`${API_URL}${path}`, {
      ...init,
      headers: {
        ...(init?.body ? { "Content-Type": "application/json" } : {}),
        Authorization: `Bearer ${authToken}`,
        ...init?.headers,
      },
    });
    if (!response.ok) {
      const body = await response.json().catch(() => null);
      throw new Error(body?.error || body?.message || `Request failed (${response.status})`);
    }
    return response.json() as Promise<T>;
  }, [token]);

  const loadClusters = useCallback(async () => {
    try {
      const result = await api<unknown>("/clusters");
      if (!Array.isArray(result)) return;
      const next = result as ClusterReference[];
      setClusters(next);
      setSelectedClusterId((current) => {
        const stored = typeof window === "undefined" ? null : localStorage.getItem(MOBILE_CLUSTER_STORAGE_KEY);
        return current
          || next.find((cluster) => cluster.owner_user_id === stored)?.owner_user_id
          || next.find((cluster) => cluster.access === "owner")?.owner_user_id
          || next[0]?.owner_user_id
          || "";
      });
    } catch {
      // Compatibility with a server rolling out before explicit discovery.
      if (userId) {
        setClusters([{ owner_user_id: userId, owner_email: userEmail || "My cluster", access: "owner" }]);
        setSelectedClusterId((current) => current || userId);
      }
    }
  }, [api, userEmail, userId]);

  useEffect(() => { void loadClusters(); }, [loadClusters]);

  const loadCluster = useCallback(async () => {
    setClusterError(null);
    try {
      const projectPath = sharedView && selectedClusterId
        ? `/clusters/${encodeURIComponent(selectedClusterId)}/projects?limit=200`
        : "/cluster/projects?limit=200";
      const policyPath = sharedView && selectedClusterId
        ? `/clusters/${encodeURIComponent(selectedClusterId)}/policy/default`
        : "/cluster/policy/default";
      const [projectPage, policy, launchProfiles] = await Promise.all([
        api<Page<ClusterProject>>(projectPath),
        api<ClusterPolicy>(policyPath),
        api<LaunchProfile[]>("/cluster/launch-profiles"),
      ]);
      setClusterProjects(projectPage.items);
      setClusterPolicy(policy);
      setProfiles(launchProfiles.filter((profile) => !isRetiredLaunchProfileKey(profile.key)));
      if (sharedView) {
        setAudit([]);
        setInvitations([]);
        setClusterMembers([]);
        setUsage(null);
      } else {
        const [auditPage, invitationRows, memberRows, usageReport] = await Promise.all([
          api<Page<AuditEvent>>("/cluster/audit?limit=25"),
          api<unknown>("/cluster/invitations"),
          api<unknown>("/cluster/members"),
          api<ClusterUsageReport>("/cluster/usage?limit=200"),
        ]);
        setAudit(auditPage.items || []);
        setInvitations(Array.isArray(invitationRows) ? invitationRows as ClusterInvitation[] : []);
        setClusterMembers(Array.isArray(memberRows) ? memberRows as ClusterMembership[] : []);
        setUsage(usageReport?.projects ? usageReport : null);
      }
    } catch (cause) {
      setClusterError(cause instanceof Error ? cause.message : "Request failed");
    }
  }, [api, selectedClusterId, sharedView]);

  useEffect(() => { void loadCluster(); }, [loadCluster]);

  // The server refuses a project outside this cluster, and that refusal is the
  // user-visible answer — surface it rather than letting the rejection escape
  // into an unhandled promise where it would look like nothing happened.
  const loadProject = useCallback(async (projectId: string) => {
    setClusterError(null);
    try {
      const path = sharedView && selectedClusterId
        ? `/clusters/${encodeURIComponent(selectedClusterId)}/projects/${encodeURIComponent(projectId)}`
        : `/cluster/projects/${encodeURIComponent(projectId)}`;
      const [detail, projectUsage] = await Promise.all([
        api<ClusterProjectDetail>(path),
        api<{ lifetime: UsageCounters; last_7d: UsageCounters; today: UsageCounters }>(`/projects/${encodeURIComponent(projectId)}/usage`),
      ]);
      setSelected(detail);
      setSelectedUsage(projectUsage);
      setOwnerUserId(detail.project.owner_user_id);
    } catch (cause) {
      setSelected(null);
      setSelectedUsage(null);
      setClusterError(cause instanceof Error ? cause.message : "Request failed");
    }
  }, [api, selectedClusterId, sharedView]);

  async function mutate(path: string, init: RequestInit, after?: () => Promise<void>) {
    setClusterError(null);
    try {
      await api(path, init);
      if (after) await after();
      await loadCluster();
    } catch (cause) {
      setClusterError(cause instanceof Error ? cause.message : "Request failed");
    }
  }

  return (
    <main className="min-h-screen bg-gray-50 p-4 dark:bg-gray-950 md:p-6">
      <div className="mx-auto max-w-6xl space-y-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Link href="/" className="inline-flex items-center gap-1 rounded border border-gray-300 px-2 py-1 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800">
              <ArrowLeft className="h-4 w-4" /> Back
            </Link>
            <div>
              <h1 className="text-xl font-semibold">{sharedView ? "Shared Cluster" : "My Cluster"}</h1>
              <p className="text-xs text-gray-500">
                {sharedView ? `Owned by ${selectedCluster?.owner_email}` : "Your machines and the projects hosted on them"}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {clusters.length > 1 && (
              <select
                aria-label="Selected cluster"
                value={selectedClusterId}
                onChange={(event) => {
                  setSelected(null);
                  setSelectedClusterId(event.target.value);
                  localStorage.setItem(MOBILE_CLUSTER_STORAGE_KEY, event.target.value);
                }}
                className="max-w-56 rounded border border-gray-300 bg-white px-2 py-1.5 text-sm dark:border-gray-700 dark:bg-gray-900"
              >
                {clusters.map((cluster) => (
                  <option key={cluster.owner_user_id} value={cluster.owner_user_id}>
                    {cluster.access === "owner" ? "My cluster" : cluster.owner_email}
                  </option>
                ))}
              </select>
            )}
            <button aria-label="Refresh machines" onClick={() => { listMachines(); void loadClusters(); void loadCluster(); }} className="inline-flex items-center gap-1 rounded border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800">
              <RefreshCw className="h-4 w-4" /> Refresh
            </button>
          </div>
        </div>

        {sharedView && (
          <div className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950/30 dark:text-amber-100">
            <div className="font-semibold">Trusted compute boundary</div>
            <p className="mt-1">{TRUST_WARNING}</p>
          </div>
        )}

        <div className="flex justify-end">
          <button
            onClick={() => setCreateOpen(true)}
            disabled={sharedView && !visibleMachines.some((machine) => machine.sharedProvisioningAvailable)}
            className="inline-flex items-center gap-1 rounded bg-emerald-600 px-3 py-2 text-sm font-medium text-white disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Plus className="h-4 w-4" /> Create project from GitHub
          </button>
        </div>

        {visibleMachines.length === 0 && (
          <div className="rounded border border-dashed border-gray-300 bg-white p-6 text-sm text-gray-600 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-300">
            No machines reported yet. Start `apas daemon` on a machine first.
          </div>
        )}

        {!sharedView && <section className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm dark:border-gray-800 dark:bg-gray-900">
          <h2 className="mb-3 text-sm font-semibold text-gray-700 dark:text-gray-300">Usage</h2>
          <AllProvidersUsage />
        </section>}

        {visibleMachines.map(({ machine, projects, clusterAccess }) => {
          const behind = isMachineBehind(machine.daemonVersion, latestVersion);
          return (
          <section key={machine.machineId} className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900">
            <div className="flex flex-wrap items-start justify-between gap-3 border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="min-w-0">
                <div className="font-medium">{machine.hostname}</div>
                <div className="text-xs text-gray-500">
                  {machine.os}/{machine.arch}
                  {" • "}
                  {daemonVersionLabel(machine.daemonVersion)}
                  {machine.lastSeen ? ` • Last seen ${new Date(machine.lastSeen).toLocaleString()}` : ""}
                </div>
              </div>
              {/* A daemon is per-machine, so the restart is targeted by machine
                  rather than through any project running on it. */}
              {clusterAccess !== "member" && <button
                aria-label={rebootActionLabelFor(behind, machine.hostname)}
                onClick={() => setRebootTarget({ id: machine.machineId, hostname: machine.hostname, behind })}
                className="inline-flex shrink-0 items-center gap-1 rounded border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800"
              >
                <RotateCcw className="h-4 w-4" /> {rebootLabelFor(behind)}
              </button>}
            </div>

            {clusterAccess !== "member" && <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="mb-2 text-sm font-medium">DeepSeek Backend (Claude Runtime)</div>
              <div className="mb-2 text-xs text-gray-500">Backend URL: <span className="font-mono">{DEEPSEEK_API_BASE_URL}</span></div>
              <div className="grid gap-2 md:grid-cols-[1fr_auto_auto]">
                <input
                  aria-label={`DeepSeek API key for ${machine.hostname}`}
                  type="text"
                  value={deepseekDrafts[machine.machineId] ?? machine.deepseekBackend?.apiKey ?? ""}
                  onChange={(event) => {
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: event.target.value }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: false }));
                  }}
                  placeholder="DeepSeek API key"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  aria-label={`Save DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    const apiKey = (deepseekDrafts[machine.machineId] ?? machine.deepseekBackend?.apiKey ?? "").trim();
                    setMachineDeepseekConfig(machine.machineId, apiKey || undefined, false);
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: apiKey }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: true }));
                  }}
                  className="rounded bg-indigo-600 px-3 py-2 text-sm text-white hover:bg-indigo-700"
                >
                  {deepseekSaved[machine.machineId] ? "Saved" : "Save"}
                </button>
                <button
                  aria-label={`Clear DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    setMachineDeepseekConfig(machine.machineId, undefined, true);
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: "" }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: false }));
                  }}
                  className="rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
                >Clear Key</button>
              </div>
              <div className="mt-2 text-xs text-gray-500">
                {machine.deepseekBackend?.apiKeyConfigured ? "API key configured" : "API key not configured"}
              </div>
            </div>}

            <div className="divide-y divide-gray-200 dark:divide-gray-800">
              {Object.values(pendingInstances).filter((pending) => pending.machineId === machine.machineId).map((pending) => (
                <div key={pending.requestId} className="flex items-center justify-between px-4 py-3">
                  <div>
                    <div className="text-sm font-medium">{pending.instanceName}</div>
                    <div className="text-xs text-gray-500">Cloning {pending.gitRemote}…</div>
                  </div>
                  <span className="rounded bg-amber-100 px-2 py-1 text-xs text-amber-700 dark:bg-amber-900/30 dark:text-amber-300">Creating…</span>
                </div>
              ))}
              {projects.map((project) => (
                <div key={project.projectId} className="flex flex-col gap-3 px-4 py-3 md:flex-row md:items-center md:justify-between">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">{project.name || project.path.split("/").pop() || project.path}</div>
                    <div className="truncate text-xs text-gray-500">{project.path}</div>
                    {project.lastError && <div className="mt-1 text-xs text-red-600 dark:text-red-400">{project.lastError}</div>}
                  </div>
                  <div className="flex items-center gap-2">
                    <span className={`rounded px-2 py-1 text-xs ${project.isRunning ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-300" : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300"}`}>
                      {project.isRunning ? `Running${project.pid ? ` (pid ${project.pid})` : ""}${formatMemory(project.memoryKb)}` : "Stopped"}
                    </span>
                    {project.isRunning ? (
                      <button aria-label={`Stop ${project.name || project.path} on ${machine.hostname}`} onClick={() => stopMachineProjectCli(machine.machineId, project.projectId)} className="inline-flex items-center gap-1 rounded bg-red-600 px-3 py-1.5 text-sm text-white hover:bg-red-700"><Square className="h-4 w-4" /> Stop</button>
                    ) : (
                      <button aria-label={`Start ${project.name || project.path} on ${machine.hostname}`} onClick={() => startMachineProjectCli(machine.machineId, project.projectId)} className="inline-flex items-center gap-1 rounded bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700"><Play className="h-4 w-4" /> Start</button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </section>
          );
        })}

        {rebootTarget && (
          <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
            onClick={() => setRebootTarget(null)}
          >
            <div
              role="dialog"
              aria-modal="true"
              aria-label={rebootActionLabelFor(rebootTarget.behind, rebootTarget.hostname)}
              className="w-full max-w-md rounded-lg border border-gray-200 bg-white p-4 shadow-xl dark:border-gray-800 dark:bg-gray-900"
              onClick={(event) => event.stopPropagation()}
            >
              <h2 className="text-base font-semibold">
                {`${rebootActionLabelFor(rebootTarget.behind, rebootTarget.hostname)}?`}
              </h2>
              <p className="mt-2 text-sm text-gray-600 dark:text-gray-300">
                {rebootTarget.behind
                  ? "This machine is behind, so the reboot updates it first."
                  : "It updates to the latest version if one is available."}
                {" "}
                Projects, panes, and agents on this machine keep running — the daemon does not own
                them.
              </p>
              <div className="mt-4 flex justify-end gap-2">
                <button
                  onClick={() => setRebootTarget(null)}
                  className="rounded border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800"
                >
                  Cancel
                </button>
                <button
                  onClick={() => {
                    rebootDaemon(rebootTarget.id);
                    setRebootTarget(null);
                  }}
                  className="inline-flex items-center gap-1 rounded bg-indigo-600 px-3 py-1.5 text-sm text-white hover:bg-indigo-700"
                >
                  <RotateCcw className="h-4 w-4" /> {rebootLabelFor(rebootTarget.behind)}
                </button>
              </div>
            </div>
          </div>
        )}

        {clusterError && (
          <div className="rounded-lg border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300">
            {clusterError}
          </div>
        )}

        {!sharedView && (
          <section className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900">
            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Share this cluster</h2>
              <p className="mt-1 text-xs text-gray-500">Invite an existing APAS account. Members can run their own public GitHub projects on eligible machines.</p>
            </div>
            <div className="grid gap-5 p-4 lg:grid-cols-2">
              <div>
                <form
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void (async () => {
                      try {
                        const response = await api<{ token: string }>("/cluster/invitations", {
                          method: "POST",
                          body: JSON.stringify({ email: inviteEmail, trust_confirmed: trustConfirmed }),
                        });
                        const link = `${window.location.origin}/cluster-invitations/${response.token}`;
                        setCreatedInviteLink(link);
                        setInviteEmail("");
                        await loadCluster();
                      } catch (cause) {
                        setClusterError(cause instanceof Error ? cause.message : "Invitation failed");
                      }
                    })();
                  }}
                >
                  <input aria-label="Invitee account email" type="email" required value={inviteEmail} onChange={(event) => setInviteEmail(event.target.value)} placeholder="person@example.com" className="w-full rounded border border-gray-300 bg-transparent px-3 py-2 text-sm dark:border-gray-700" />
                  <label className="flex items-start gap-2 text-xs text-gray-600 dark:text-gray-300">
                    <input type="checkbox" checked={trustConfirmed} onChange={(event) => setTrustConfirmed(event.target.checked)} className="mt-0.5" />
                    <span>{TRUST_WARNING}</span>
                  </label>
                  <button disabled={!trustConfirmed || !inviteEmail.trim()} className="rounded bg-blue-600 px-3 py-2 text-sm text-white disabled:opacity-50">Create invitation</button>
                </form>
                {createdInviteLink && (
                  <div className="mt-3 flex items-center gap-2 rounded bg-gray-50 p-2 dark:bg-gray-800">
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{createdInviteLink}</span>
                    <button aria-label="Copy invitation link" onClick={() => void navigator.clipboard.writeText(createdInviteLink)} className="rounded border p-1.5"><Copy className="h-4 w-4" /></button>
                  </div>
                )}
                <div className="mt-4 space-y-2">
                  {invitations.map((invitation) => (
                    <div key={invitation.id} className="flex items-center justify-between rounded border border-gray-200 p-2 text-sm dark:border-gray-800">
                      <div><div>{invitation.invitee_email}</div><div className="text-xs capitalize text-gray-500">{invitation.status} · expires {new Date(invitation.expires_at).toLocaleString()}</div></div>
                      {invitation.status === "pending" && <button onClick={() => void mutate(`/cluster/invitations/${invitation.id}`, { method: "DELETE" })} className="text-xs text-red-600">Revoke</button>}
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <h3 className="mb-2 text-sm font-semibold">Cluster members</h3>
                {clusterMembers.length === 0 ? <p className="text-sm text-gray-500">No one has joined this cluster.</p> : clusterMembers.map((member) => (
                  <div key={member.user_id} className="flex items-center justify-between border-b border-gray-100 py-2 text-sm dark:border-gray-800">
                    <div><div>{member.user_email}</div><div className="text-xs capitalize text-gray-500">{member.status}</div></div>
                    {member.status === "active" && <button onClick={() => { if (window.confirm(`Revoke ${member.user_email}'s compute access?`)) void mutate(`/cluster/members/${member.user_id}`, { method: "DELETE" }); }} className="text-xs text-red-600">Revoke access</button>}
                  </div>
                ))}
              </div>
            </div>
          </section>
        )}

        {!sharedView && usage && (
          <section className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm dark:border-gray-800 dark:bg-gray-900">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div><h2 className="text-sm font-semibold">Cluster usage</h2><p className="text-xs text-gray-500">Informational totals; use policy, suspension, or membership revocation to manage consumption.</p></div>
              <select aria-label="Usage window" value={usageWindow} onChange={(event) => setUsageWindow(event.target.value as typeof usageWindow)} className="rounded border border-gray-300 bg-transparent px-2 py-1 text-sm dark:border-gray-700">
                <option value="today">Today (UTC)</option><option value="last_7d">Last 7 days</option><option value="lifetime">Lifetime</option>
              </select>
            </div>
            <UsageSummary counters={usage[usageWindow]} />
            <div className="mt-4 overflow-x-auto"><table className="w-full text-left text-sm"><thead><tr><th className="pb-2">Project</th><th className="pb-2">Owner</th><th className="pb-2">Tokens</th><th className="pb-2">Cost</th></tr></thead><tbody>{usage.projects.map((project) => { const counters = project.usage[usageWindow]; return <tr key={project.project_id} className="border-t border-gray-100 dark:border-gray-800"><td className="py-2">{project.project_name || project.project_id}</td><td>{project.owner_email}</td><td>{(counters.input_tokens + counters.output_tokens).toLocaleString()}</td><td>{counters.cost_usd_reported ? `$${counters.cost_usd.toFixed(2)}` : "Unavailable"}</td></tr>; })}</tbody></table></div>
          </section>
        )}

        <section className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900">
          <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
            <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Projects in this cluster</h2>
            <p className="mt-1 text-xs text-gray-500">
              Every project that runs under your account, including projects another account owns. You administer these
              without being a member of them.
            </p>
          </div>
          {clusterProjects.length === 0 ? (
            <div className="p-6 text-sm text-gray-600 dark:text-gray-300">No projects are hosted in your cluster yet.</div>
          ) : (
            <div className="grid gap-4 p-4 lg:grid-cols-[minmax(0,1fr)_minmax(320px,1fr)]">
              <div className="overflow-hidden rounded-lg border border-gray-200 dark:border-gray-800">
                {clusterProjects.map((project) => (
                  <button
                    key={project.id}
                    aria-label={`Manage ${project.project_name || project.id}`}
                    onClick={() => void loadProject(project.id)}
                    className="flex w-full items-center gap-3 border-b border-gray-100 p-3 text-left last:border-b-0 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-800"
                  >
                    <FolderOpen className="h-5 w-5 text-blue-500" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm font-medium">{project.project_name || "Unnamed project"}</div>
                      <div className="truncate text-xs text-gray-500">
                        Owner {project.owner_email} · {project.member_count} members · {project.active_session_count} active
                      </div>
                    </div>
                    <span className={`h-2 w-2 rounded-full ${project.connected ? "bg-green-500" : "bg-gray-400"}`} title={project.connected ? "Connected" : "Offline"} />
                    <span className="text-xs capitalize">{project.lifecycle_status}</span>
                  </button>
                ))}
              </div>
              {selected ? (
                <div className="space-y-4">
                  {sharedView && (
                    <div className="rounded-lg border border-gray-200 p-4 dark:border-gray-800">
                      <h3 className="text-sm font-semibold">{selected.project.project_name || selected.project.id}</h3>
                      <p className="mt-1 text-xs text-gray-500">Owned by {selected.project.owner_email}. Cluster membership does not expose any other hosted projects.</p>
                      {selectedUsage && <div className="mt-4"><UsageSummary counters={selectedUsage[usageWindow]} /></div>}
                    </div>
                  )}
                  {!sharedView && (
                  <div className="rounded-lg border border-gray-200 p-4 dark:border-gray-800">
                    <h3 className="mb-2 text-sm font-semibold">{selected.project.project_name || selected.project.id}</h3>
                    <div className="mb-3 break-all font-mono text-xs text-gray-500">{selected.project.id}</div>
                    <div className="flex flex-wrap gap-2">
                      <button
                        onClick={() => {
                          const suspending = selected.project.lifecycle_status === "active";
                          if (suspending && !window.confirm("Suspend this project and stop its runtime? Project data is preserved.")) return;
                          void mutate(
                            `/cluster/projects/${selected.project.id}/lifecycle`,
                            { method: "PATCH", body: JSON.stringify({ status: suspending ? "suspended" : "active" }) },
                            () => loadProject(selected.project.id),
                          );
                        }}
                        className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700"
                      >
                        {selected.project.lifecycle_status === "active" ? "Suspend project" : "Reactivate project"}
                      </button>
                      <button
                        onClick={() => {
                          if (!window.confirm("Stop this project's current runtime? Project data and future access are preserved.")) return;
                          void mutate(`/cluster/projects/${selected.project.id}/stop-runtime`, { method: "POST" }, () => loadProject(selected.project.id));
                        }}
                        className="rounded border border-red-300 px-3 py-2 text-sm text-red-600 dark:border-red-800"
                      >
                        Stop runtime
                      </button>
                    </div>
                    <form
                      className="mt-4 flex gap-2"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void mutate(
                          `/cluster/projects/${selected.project.id}/owner`,
                          { method: "PATCH", body: JSON.stringify({ user_id: ownerUserId }) },
                          () => loadProject(selected.project.id),
                        );
                      }}
                    >
                      <input aria-label="New owner user ID" value={ownerUserId} onChange={(event) => setOwnerUserId(event.target.value)} className="min-w-0 flex-1 rounded border border-gray-300 bg-transparent px-2 py-1 text-sm dark:border-gray-700" />
                      <button className="rounded bg-gray-900 px-3 py-1 text-sm text-white dark:bg-gray-100 dark:text-gray-900">Transfer</button>
                    </form>
                  </div>
                  )}

                  {!sharedView && (
                  <div className="rounded-lg border border-gray-200 p-4 dark:border-gray-800">
                    <h3 className="mb-2 text-sm font-semibold">Members</h3>
                    {selected.members.map((member) => (
                      <div key={member.user_id} className="flex items-center justify-between border-b border-gray-100 py-2 text-sm last:border-b-0 dark:border-gray-800">
                        <span>{member.email}</span>
                        <button
                          aria-label={`Remove ${member.email}`}
                          onClick={() => void mutate(`/cluster/projects/${selected.project.id}/members/${member.user_id}`, { method: "DELETE" }, () => loadProject(selected.project.id))}
                          className="text-red-600"
                        >
                          Remove
                        </button>
                      </div>
                    ))}
                    <form
                      className="mt-3 flex gap-2"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void mutate(
                          `/cluster/projects/${selected.project.id}/members`,
                          { method: "POST", body: JSON.stringify({ user_id: memberUserId }) },
                          async () => { setMemberUserId(""); await loadProject(selected.project.id); },
                        );
                      }}
                    >
                      <input aria-label="Member user ID" value={memberUserId} onChange={(event) => setMemberUserId(event.target.value)} placeholder="User ID" className="min-w-0 flex-1 rounded border border-gray-300 bg-transparent px-2 py-1 text-sm dark:border-gray-700" />
                      <button className="rounded bg-blue-600 px-3 py-1 text-sm text-white">Add</button>
                    </form>
                  </div>
                  )}

                  {!sharedView && (
                  <PolicyEditor
                    title="Project policy"
                    policy={selected.policy}
                    profiles={profiles}
                    bound={clusterPolicy?.deployment}
                    onSave={(policy) => mutate(`/cluster/projects/${selected.project.id}/policy`, { method: "PATCH", body: JSON.stringify(policy) }, () => loadProject(selected.project.id))}
                    onInherit={() => mutate(`/cluster/projects/${selected.project.id}/policy`, { method: "PATCH", body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }) }, () => loadProject(selected.project.id))}
                  />
                  )}
                </div>
              ) : (
                <div className="rounded-lg border border-dashed border-gray-300 p-8 text-center text-sm text-gray-500 dark:border-gray-700">
                  Select a project to manage it.
                </div>
              )}
            </div>
          )}
        </section>

        {clusterPolicy && !sharedView && (
          <PolicyEditor
            title="Cluster default policy"
            description="Applies to every project hosted in your cluster, including projects other accounts own. You can narrow what the deployment allows, never widen it."
            policy={{
              team_available: clusterPolicy.cluster?.team_available ?? clusterPolicy.deployment.team_available,
              allowed_launch_profiles:
                clusterPolicy.cluster?.allowed_launch_profiles ?? clusterPolicy.deployment.allowed_launch_profiles,
              version: clusterPolicy.cluster?.version,
            }}
            profiles={profiles}
            bound={clusterPolicy.deployment}
            saveLabel="Save cluster policy"
            onSave={(policy) => mutate("/cluster/policy/default", { method: "PATCH", body: JSON.stringify(policy) })}
            onInherit={() => mutate("/cluster/policy/default", { method: "PATCH", body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }) })}
          />
        )}

        {!sharedView && <section className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900">
          <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
            <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Cluster activity</h2>
          </div>
          {audit.length === 0 ? (
            <div className="p-6 text-sm text-gray-600 dark:text-gray-300">Nothing has been recorded for this cluster yet.</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm">
                <thead className="bg-gray-100 dark:bg-gray-800">
                  <tr><th className="p-3">Time</th><th className="p-3">Action</th><th className="p-3">Target</th></tr>
                </thead>
                <tbody>
                  {audit.map((event) => (
                    <tr key={event.id} className="border-t border-gray-100 dark:border-gray-800">
                      <td className="whitespace-nowrap p-3">{event.created_at ? new Date(event.created_at).toLocaleString() : "—"}</td>
                      <td className="p-3">{event.action}</td>
                      <td className="p-3 font-mono text-xs">{event.target_type}: {event.target_id}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>}

        <CreateInstanceModal
          open={createOpen}
          onClose={() => setCreateOpen(false)}
          gitRemote="github.com/owner/repository"
          cloneUrl="https://github.com/owner/repository"
          clusterOwnerUserId={selectedClusterId || undefined}
        />
      </div>
    </main>
  );
}

function UsageSummary({ counters }: { counters: UsageCounters }) {
  return (
    <div className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
      <div><div className="text-xs text-gray-500">Prompts</div><div className="font-semibold">{counters.prompts.toLocaleString()}</div></div>
      <div><div className="text-xs text-gray-500">Responses</div><div className="font-semibold">{counters.responses.toLocaleString()}</div></div>
      <div><div className="text-xs text-gray-500">Tokens</div><div className="font-semibold">{(counters.input_tokens + counters.output_tokens).toLocaleString()}</div></div>
      <div><div className="text-xs text-gray-500">Cost</div><div className="font-semibold">{counters.cost_usd_reported ? `$${counters.cost_usd.toFixed(2)}` : "Unavailable"}</div></div>
    </div>
  );
}
