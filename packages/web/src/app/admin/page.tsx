"use client";

import { useStore } from "@/lib/store";
import { isRetiredLaunchProfileKey } from "@/lib/providerOptions";
import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Activity,
  ArrowLeft,
  FolderOpen,
  RefreshCw,
  Shield,
} from "lucide-react";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
type Tab = "overview" | "users" | "projects" | "audit";

interface Page<T> { items: T[]; limit: number; offset: number }
interface SystemStats {
  total_users: number;
  recent_users_7d: number;
  total_sessions: number;
  active_sessions_24h: number;
  total_cli_clients: number;
  online_cli_clients: number;
  total_shares: number;
}
interface ClusterUser {
  id: string;
  email: string;
  cluster_role: "admin" | "user";
  account_status: "active" | "suspended";
  created_at?: string;
}
interface Policy {
  team_available: boolean;
  allowed_launch_profiles: string[];
  version: number;
  project_suspended: boolean;
}
interface LaunchProfile { key: string; label: string }
interface ProjectSummary {
  id: string;
  project_name?: string | null;
  hostname?: string | null;
  owner_user_id: string;
  owner_email: string;
  lifecycle_status: "active" | "suspended";
  member_count: number;
  active_session_count: number;
  last_activity?: string;
  connected: boolean;
  effective_policy: Policy;
}
interface ProjectMember { user_id: string; email: string; created_at?: string }
interface ProjectDetail {
  project: Omit<ProjectSummary, "connected" | "effective_policy">;
  members: ProjectMember[];
  policy: Policy;
  policy_override?: {
    team_available: boolean | null;
    allowed_launch_profiles: string[] | null;
    version: number;
    legacy_imported: boolean;
    legacy_conflict?: string | null;
  } | null;
}
interface AuditEvent {
  id: number;
  actor_user_id: string;
  action: string;
  target_type: string;
  target_id: string;
  details?: string;
  created_at?: string;
}

function StatCard({ label, value, detail }: { label: string; value: number; detail?: string }) {
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
      <div className="text-2xl font-semibold">{value}</div>
      <div className="text-sm text-gray-500">{label}</div>
      {detail && <div className="mt-1 text-xs text-gray-400">{detail}</div>}
    </div>
  );
}

function ErrorBanner({ message }: { message: string | null }) {
  return message ? (
    <div className="rounded-lg border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300">
      {message}
    </div>
  ) : null;
}

export default function AdminPage() {
  const router = useRouter();
  const { token, clusterRole } = useStore();
  const [tab, setTab] = useState<Tab>("overview");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [users, setUsers] = useState<ClusterUser[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [auditOffset, setAuditOffset] = useState(0);
  const [profiles, setProfiles] = useState<LaunchProfile[]>([]);
  const [defaultPolicy, setDefaultPolicy] = useState<Policy | null>(null);
  const [selectedProject, setSelectedProject] = useState<ProjectDetail | null>(null);
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteUrl, setInviteUrl] = useState("");
  const [memberUserId, setMemberUserId] = useState("");
  const [ownerUserId, setOwnerUserId] = useState("");

  const api = useCallback(async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetch(`${API_URL}${path}`, {
      ...init,
      headers: {
        ...(init?.body ? { "Content-Type": "application/json" } : {}),
        Authorization: `Bearer ${token}`,
        ...init?.headers,
      },
    });
    if (!response.ok) {
      const body = await response.json().catch(() => null);
      throw new Error(body?.message || `Request failed (${response.status})`);
    }
    return response.json() as Promise<T>;
  }, [token]);

  const loadProject = useCallback(async (projectId: string) => {
    const detail = await api<ProjectDetail>(`/admin/projects/${encodeURIComponent(projectId)}`);
    setSelectedProject(detail);
    setOwnerUserId(detail.project.owner_user_id);
  }, [api]);

  const load = useCallback(async () => {
    if (!token || clusterRole !== "admin") return;
    setLoading(true);
    setError(null);
    try {
      if (tab === "overview") {
        const [nextStats, nextProfiles, nextDefault] = await Promise.all([
          api<SystemStats>("/admin/stats"),
          api<LaunchProfile[]>("/admin/launch-profiles"),
          api<Policy>("/admin/policy/default"),
        ]);
        setStats(nextStats);
        setProfiles(nextProfiles.filter((profile) => !isRetiredLaunchProfileKey(profile.key)));
        setDefaultPolicy(nextDefault);
      } else if (tab === "users") {
        setUsers((await api<Page<ClusterUser>>("/admin/users?limit=200")).items);
      } else if (tab === "projects") {
        const [projectPage, nextProfiles] = await Promise.all([
          api<Page<ProjectSummary>>("/admin/projects?limit=200"),
          api<LaunchProfile[]>("/admin/launch-profiles"),
        ]);
        setProjects(projectPage.items);
        setProfiles(nextProfiles.filter((profile) => !isRetiredLaunchProfileKey(profile.key)));
      } else {
        setAudit((await api<Page<AuditEvent>>(`/admin/audit?limit=50&offset=${auditOffset}`)).items);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Request failed");
    } finally {
      setLoading(false);
    }
  }, [api, auditOffset, clusterRole, tab, token]);

  useEffect(() => {
    if (!token) router.replace("/login?redirect=/admin");
    else if (clusterRole && clusterRole !== "admin") router.replace("/");
  }, [clusterRole, router, token]);

  useEffect(() => { void load(); }, [load]);

  async function mutate(path: string, init: RequestInit, after?: () => Promise<void>) {
    setError(null);
    try {
      await api(path, init);
      if (after) await after();
      else await load();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Request failed");
    }
  }

  if (!token || clusterRole !== "admin") return null;

  return (
    <main className="min-h-screen bg-gray-50 text-gray-900 dark:bg-gray-950 dark:text-gray-100">
      <div className="mx-auto max-w-7xl p-4 md:p-6">
        <header className="mb-6 flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <button aria-label="Back" onClick={() => router.push("/")} className="rounded-lg p-2 hover:bg-gray-200 dark:hover:bg-gray-800">
              <ArrowLeft className="h-5 w-5" />
            </button>
            <Shield className="h-6 w-6 text-blue-600" />
            <div>
              <h1 className="text-2xl font-bold">Cluster Administration</h1>
              <p className="text-sm text-gray-500">Accounts, projects, policy, and audit</p>
            </div>
          </div>
          <button aria-label="Refresh" onClick={() => void load()} className="rounded-lg border border-gray-300 p-2 dark:border-gray-700">
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </button>
        </header>

        <nav className="mb-6 flex gap-1 overflow-x-auto rounded-xl border border-gray-200 bg-white p-1 dark:border-gray-800 dark:bg-gray-900">
          {(["overview", "users", "projects", "audit"] as Tab[]).map((item) => (
            <button key={item} onClick={() => setTab(item)} className={`rounded-lg px-4 py-2 text-sm capitalize ${tab === item ? "bg-blue-600 text-white" : "hover:bg-gray-100 dark:hover:bg-gray-800"}`}>
              {item}
            </button>
          ))}
        </nav>

        <ErrorBanner message={error} />

        {tab === "overview" && stats && (
          <section className="mt-4 space-y-6">
            <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
              <StatCard label="Cluster users" value={stats.total_users} detail={`${stats.recent_users_7d} added this week`} />
              <StatCard label="Projects / sessions" value={stats.total_sessions} detail={`${stats.active_sessions_24h} active in 24h`} />
              <StatCard label="CLI clients" value={stats.total_cli_clients} detail={`${stats.online_cli_clients} online`} />
              <StatCard label="Legacy shares" value={stats.total_shares} />
            </div>
            {defaultPolicy && (
              <PolicyEditor
                title="Cluster default policy"
                policy={defaultPolicy}
                profiles={profiles}
                onSave={(policy) => mutate("/admin/policy/default", { method: "PATCH", body: JSON.stringify(policy) })}
              />
            )}
          </section>
        )}

        {tab === "users" && (
          <section className="mt-4 space-y-4">
            <form className="flex flex-col gap-2 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900 sm:flex-row" onSubmit={(event) => {
              event.preventDefault();
              void (async () => {
                try {
                  const result = await api<{ registration_url: string }>("/admin/users/invitations", { method: "POST", body: JSON.stringify({ email: inviteEmail }) });
                  setInviteUrl(result.registration_url);
                  setInviteEmail("");
                } catch (cause) { setError(cause instanceof Error ? cause.message : "Invitation failed"); }
              })();
            }}>
              <input aria-label="Invitation email" value={inviteEmail} onChange={(event) => setInviteEmail(event.target.value)} type="email" required placeholder="new.user@example.com" className="min-w-0 flex-1 rounded-lg border border-gray-300 bg-transparent px-3 py-2 dark:border-gray-700" />
              <button className="rounded-lg bg-blue-600 px-4 py-2 text-white">Create invitation</button>
            </form>
            {inviteUrl && <div className="rounded-lg bg-green-50 p-3 text-sm text-green-800 dark:bg-green-950/40 dark:text-green-300">Registration URL: <span className="break-all font-mono">{inviteUrl}</span></div>}
            <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
              <table className="w-full text-left text-sm">
                <thead className="bg-gray-100 dark:bg-gray-800"><tr><th className="p-3">Account</th><th className="p-3">Role</th><th className="p-3">Status</th><th className="p-3">Created</th></tr></thead>
                <tbody>{users.map((user) => <tr key={user.id} className="border-t border-gray-100 dark:border-gray-800">
                  <td className="p-3"><div>{user.email}</div><div className="text-xs text-gray-400">{user.id}</div></td>
                  <td className="p-3"><select aria-label={`Role for ${user.email}`} value={user.cluster_role} onChange={(event) => void mutate(`/admin/users/${user.id}`, { method: "PATCH", body: JSON.stringify({ cluster_role: event.target.value }) })} className="rounded border border-gray-300 bg-transparent p-1 dark:border-gray-700"><option value="user">User</option><option value="admin">Admin</option></select></td>
                  <td className="p-3"><button onClick={() => void mutate(`/admin/users/${user.id}`, { method: "PATCH", body: JSON.stringify({ account_status: user.account_status === "active" ? "suspended" : "active" }) })} className={`rounded-full px-3 py-1 text-xs ${user.account_status === "active" ? "bg-green-100 text-green-700" : "bg-amber-100 text-amber-800"}`}>{user.account_status}</button></td>
                  <td className="p-3 text-gray-500">{user.created_at ? new Date(user.created_at).toLocaleString() : "—"}</td>
                </tr>)}</tbody>
              </table>
            </div>
          </section>
        )}

        {tab === "projects" && (
          <section className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(360px,1fr)]">
            <div className="overflow-hidden rounded-xl border border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
              {projects.map((project) => <button key={project.id} onClick={() => void loadProject(project.id)} className="flex w-full items-center gap-3 border-b border-gray-100 p-4 text-left hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-800">
                <FolderOpen className="h-5 w-5 text-blue-500" />
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">{project.project_name || "Unnamed project"}</div>
                  <div className="truncate text-xs text-gray-500">Host {project.hostname || "Unknown"} · Owner {project.owner_email} · {project.member_count} members · {project.active_session_count} active</div>
                  <div className="truncate font-mono text-[11px] text-gray-400">{project.id}</div>
                </div>
                <span className={`h-2 w-2 rounded-full ${project.connected ? "bg-green-500" : "bg-gray-400"}`} title={project.connected ? "Connected" : "Offline"} />
                <span className="text-xs capitalize">{project.lifecycle_status}</span>
              </button>)}
            </div>
            {selectedProject ? (
              <div className="space-y-4">
                <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
                  <h2 className="mb-3 font-semibold">Project control</h2>
                  <div className="mb-3 break-all font-mono text-xs">{selectedProject.project.id}</div>
                  <div className="flex flex-wrap gap-2">
                    <button onClick={() => {
                      const suspending = selectedProject.project.lifecycle_status === "active";
                      if (suspending && !window.confirm("Suspend this project and stop its connected runtime? Project data is preserved.")) return;
                      void mutate(`/admin/projects/${selectedProject.project.id}/lifecycle`, { method: "PATCH", body: JSON.stringify({ status: suspending ? "suspended" : "active" }) }, () => loadProject(selectedProject.project.id));
                    }} className="rounded-lg border border-gray-300 px-3 py-2 text-sm dark:border-gray-700">{selectedProject.project.lifecycle_status === "active" ? "Suspend project" : "Reactivate project"}</button>
                    <button onClick={() => {
                      if (!window.confirm("Stop this project's current runtime? Project data and future access are preserved.")) return;
                      void mutate(`/admin/projects/${selectedProject.project.id}/stop-runtime`, { method: "POST" }, () => loadProject(selectedProject.project.id));
                    }} className="rounded-lg border border-red-300 px-3 py-2 text-sm text-red-600 dark:border-red-800">Stop runtime</button>
                  </div>
                  <form className="mt-4 flex gap-2" onSubmit={(event) => { event.preventDefault(); void mutate(`/admin/projects/${selectedProject.project.id}/owner`, { method: "PATCH", body: JSON.stringify({ user_id: ownerUserId }) }, () => loadProject(selectedProject.project.id)); }}>
                    <input aria-label="New owner user ID" value={ownerUserId} onChange={(event) => setOwnerUserId(event.target.value)} className="min-w-0 flex-1 rounded border border-gray-300 bg-transparent px-2 py-1 text-sm dark:border-gray-700" />
                    <button className="rounded bg-gray-900 px-3 py-1 text-sm text-white dark:bg-gray-100 dark:text-gray-900">Transfer</button>
                  </form>
                </div>
                <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
                  <h2 className="mb-3 font-semibold">Members</h2>
                  {selectedProject.members.map((member) => <div key={member.user_id} className="flex items-center justify-between border-b border-gray-100 py-2 text-sm dark:border-gray-800"><span>{member.email}</span><button onClick={() => void mutate(`/admin/projects/${selectedProject.project.id}/members/${member.user_id}`, { method: "DELETE" }, () => loadProject(selectedProject.project.id))} className="text-red-600">Remove</button></div>)}
                  <form className="mt-3 flex gap-2" onSubmit={(event) => { event.preventDefault(); void mutate(`/admin/projects/${selectedProject.project.id}/members`, { method: "POST", body: JSON.stringify({ user_id: memberUserId }) }, async () => { setMemberUserId(""); await loadProject(selectedProject.project.id); }); }}>
                    <input aria-label="Member user ID" value={memberUserId} onChange={(event) => setMemberUserId(event.target.value)} placeholder="User ID" className="min-w-0 flex-1 rounded border border-gray-300 bg-transparent px-2 py-1 text-sm dark:border-gray-700" />
                    <button className="rounded bg-blue-600 px-3 py-1 text-sm text-white">Add</button>
                  </form>
                </div>
                <PolicyEditor
                  title="Project policy override"
                  policy={selectedProject.policy}
                  profiles={profiles}
                  onSave={(policy) => mutate(`/admin/projects/${selectedProject.project.id}/policy`, { method: "PATCH", body: JSON.stringify(policy) }, () => loadProject(selectedProject.project.id))}
                  onInherit={() => mutate(`/admin/projects/${selectedProject.project.id}/policy`, { method: "PATCH", body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }) }, () => loadProject(selectedProject.project.id))}
                />
                {selectedProject.policy_override?.legacy_conflict && (
                  <div className="rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-300">
                    <div className="font-semibold">Conflicting legacy policy snapshot</div>
                    <div className="mt-1 break-all font-mono text-xs">{selectedProject.policy_override.legacy_conflict}</div>
                    <p className="mt-2 text-xs">The first imported or administrator-set policy remains effective.</p>
                  </div>
                )}
              </div>
            ) : <div className="rounded-xl border border-dashed border-gray-300 p-10 text-center text-gray-500 dark:border-gray-700">Select a project to manage it.</div>}
          </section>
        )}

        {tab === "audit" && (
          <section className="mt-4 space-y-3">
            <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900"><table className="w-full text-left text-sm"><thead className="bg-gray-100 dark:bg-gray-800"><tr><th className="p-3">Time</th><th className="p-3">Actor</th><th className="p-3">Action</th><th className="p-3">Target</th><th className="p-3">Details</th></tr></thead><tbody>{audit.map((event) => <tr key={event.id} className="border-t border-gray-100 dark:border-gray-800"><td className="whitespace-nowrap p-3">{event.created_at ? new Date(event.created_at).toLocaleString() : "—"}</td><td className="p-3 font-mono text-xs">{event.actor_user_id}</td><td className="p-3">{event.action}</td><td className="p-3">{event.target_type}: {event.target_id}</td><td className="max-w-xs truncate p-3 font-mono text-xs" title={event.details}>{event.details || "—"}</td></tr>)}</tbody></table></div>
            <div className="flex justify-between"><button disabled={auditOffset === 0} onClick={() => setAuditOffset(Math.max(0, auditOffset - 50))} className="rounded border px-3 py-1 disabled:opacity-40">Previous</button><button disabled={audit.length < 50} onClick={() => setAuditOffset(auditOffset + 50)} className="rounded border px-3 py-1 disabled:opacity-40">Next</button></div>
          </section>
        )}

        {loading && <div className="mt-6 flex items-center justify-center gap-2 text-sm text-gray-500"><Activity className="h-4 w-4 animate-pulse" /> Loading…</div>}
      </div>
    </main>
  );
}

function PolicyEditor({ title, policy, profiles, onSave, onInherit }: { title: string; policy: Policy; profiles: LaunchProfile[]; onSave: (policy: Pick<Policy, "team_available" | "allowed_launch_profiles">) => void; onInherit?: () => void }) {
  const [team, setTeam] = useState(policy.team_available);
  const [allowed, setAllowed] = useState<string[]>(policy.allowed_launch_profiles);
  useEffect(() => { setTeam(policy.team_available); setAllowed(policy.allowed_launch_profiles); }, [policy]);
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
      <div className="mb-3 flex items-center justify-between"><h2 className="font-semibold">{title}</h2><span className="text-xs text-gray-500">v{policy.version}</span></div>
      <label className="mb-3 flex items-center gap-2 text-sm"><input type="checkbox" checked={team} onChange={(event) => setTeam(event.target.checked)} /> Team launch available</label>
      <div className="grid gap-2 sm:grid-cols-2">
        {profiles.map((profile) => <label key={profile.key} className="flex items-start gap-2 rounded border border-gray-200 p-2 text-xs dark:border-gray-700"><input type="checkbox" checked={allowed.includes(profile.key)} onChange={(event) => setAllowed((current) => event.target.checked ? [...current, profile.key] : current.filter((key) => key !== profile.key))} /><span><span className="block font-medium">{profile.label}</span><span className="font-mono text-gray-400">{profile.key}</span></span></label>)}
      </div>
      <div className="mt-4 flex flex-wrap gap-2">
        <button onClick={() => onSave({ team_available: team, allowed_launch_profiles: allowed })} className="rounded-lg bg-blue-600 px-4 py-2 text-sm text-white">Save policy</button>
        {onInherit && <button onClick={onInherit} className="rounded-lg border border-gray-300 px-4 py-2 text-sm dark:border-gray-700">Use cluster defaults</button>}
      </div>
    </div>
  );
}
