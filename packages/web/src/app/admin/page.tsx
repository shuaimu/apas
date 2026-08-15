"use client";

/**
 * APAS system administration — the whole deployment.
 *
 * Deliberately self-contained: it never reads the app's zustand store, is not
 * linked from any navigation, and authenticates a separate credential rather
 * than an account. Its token lives in `sessionStorage` under its own key so it
 * dies with the tab and cannot be picked up by ordinary app code.
 *
 * The login form is inline rather than a `/admin/login` route on purpose:
 * nginx proxies the whole `/admin/` prefix to `apas-server`, so a Next.js page
 * under it would never be served.
 */

import { isRetiredLaunchProfileKey } from "@/lib/providerOptions";
import { EffectivePolicy, LaunchProfile, PolicyEditor } from "@/components/PolicyEditor";
import { useCallback, useEffect, useState } from "react";
import { Activity, FolderOpen, RefreshCw, Server, Shield } from "lucide-react";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
const TOKEN_KEY = "apas_system_admin_token";
type Tab = "overview" | "users" | "clusters" | "projects" | "audit";

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
interface Account {
  id: string;
  email: string;
  account_status: "active" | "suspended";
  created_at?: string;
}
interface ClusterSummary {
  user_id: string;
  email: string;
  account_status: string;
  hosted_project_count: number;
  owned_project_count: number;
  active_session_count: number;
  last_activity?: string;
}
interface ProjectSummary {
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
interface ProjectDetail {
  project: Omit<ProjectSummary, "connected" | "effective_policy">;
  members: ProjectMember[];
  policy: EffectivePolicy;
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
  actor_kind: string;
  actor_user_id: string;
  action: string;
  target_type: string;
  target_id: string;
  cluster_user_id?: string | null;
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

function SignIn({ onSignedIn }: { onSignedIn: (token: string, bootstrapPending: boolean) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  return (
    <main className="flex min-h-screen items-center justify-center bg-gray-50 p-4 dark:bg-gray-950">
      <form
        className="w-full max-w-sm space-y-4 rounded-xl border border-gray-200 bg-white p-6 dark:border-gray-800 dark:bg-gray-900"
        onSubmit={(event) => {
          event.preventDefault();
          setBusy(true);
          setError(null);
          void (async () => {
            try {
              const response = await fetch(`${API_URL}/admin/auth/login`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ username, password }),
              });
              const body = await response.json().catch(() => null);
              if (!response.ok) throw new Error(body?.message || "Sign-in failed");
              onSignedIn(body.token as string, Boolean(body.bootstrap_pending));
            } catch (cause) {
              setError(cause instanceof Error ? cause.message : "Sign-in failed");
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        <div className="flex items-center gap-3">
          <Shield className="h-6 w-6 text-blue-600" />
          <div>
            <h1 className="text-lg font-bold">APAS system administration</h1>
            <p className="text-xs text-gray-500">Separate sign-in. Account credentials are not accepted.</p>
          </div>
        </div>
        <ErrorBanner message={error} />
        <input
          aria-label="System administrator username"
          autoComplete="username"
          value={username}
          onChange={(event) => setUsername(event.target.value)}
          placeholder="Username"
          className="w-full rounded-lg border border-gray-300 bg-transparent px-3 py-2 text-sm dark:border-gray-700"
        />
        <input
          aria-label="System administrator password"
          autoComplete="current-password"
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          placeholder="Password"
          className="w-full rounded-lg border border-gray-300 bg-transparent px-3 py-2 text-sm dark:border-gray-700"
        />
        <button disabled={busy} className="w-full rounded-lg bg-blue-600 px-4 py-2 text-sm text-white disabled:opacity-50">
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}

export default function SystemAdminPage() {
  const [token, setToken] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [bootstrapPending, setBootstrapPending] = useState(false);
  const [tab, setTab] = useState<Tab>("overview");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [clusters, setClusters] = useState<ClusterSummary[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [auditOffset, setAuditOffset] = useState(0);
  const [profiles, setProfiles] = useState<LaunchProfile[]>([]);
  const [defaultPolicy, setDefaultPolicy] = useState<EffectivePolicy | null>(null);
  const [selectedProject, setSelectedProject] = useState<ProjectDetail | null>(null);
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteUrl, setInviteUrl] = useState("");
  const [memberUserId, setMemberUserId] = useState("");
  const [ownerUserId, setOwnerUserId] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");

  // sessionStorage, not localStorage: this credential should not outlive the
  // tab, and must never be reachable from the ordinary app's storage keys.
  useEffect(() => {
    const stored = sessionStorage.getItem(TOKEN_KEY);
    if (!stored) {
      setReady(true);
      return;
    }
    void (async () => {
      try {
        const response = await fetch(`${API_URL}/admin/auth/me`, {
          headers: { Authorization: `Bearer ${stored}` },
        });
        if (!response.ok) throw new Error("expired");
        const body = await response.json();
        setBootstrapPending(Boolean(body.bootstrap_pending));
        setToken(stored);
      } catch {
        sessionStorage.removeItem(TOKEN_KEY);
      } finally {
        setReady(true);
      }
    })();
  }, []);

  const api = useCallback(async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetch(`${API_URL}${path}`, {
      ...init,
      headers: {
        ...(init?.body ? { "Content-Type": "application/json" } : {}),
        Authorization: `Bearer ${token}`,
        ...init?.headers,
      },
    });
    if (response.status === 401) {
      sessionStorage.removeItem(TOKEN_KEY);
      setToken(null);
    }
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
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      if (tab === "overview") {
        const [nextStats, nextProfiles, nextDefault] = await Promise.all([
          api<SystemStats>("/admin/stats"),
          api<LaunchProfile[]>("/admin/launch-profiles"),
          api<EffectivePolicy>("/admin/policy/default"),
        ]);
        setStats(nextStats);
        setProfiles(nextProfiles.filter((profile) => !isRetiredLaunchProfileKey(profile.key)));
        setDefaultPolicy(nextDefault);
      } else if (tab === "users") {
        setAccounts((await api<Page<Account>>("/admin/users?limit=200")).items);
      } else if (tab === "clusters") {
        setClusters((await api<Page<ClusterSummary>>("/admin/clusters")).items);
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
  }, [api, auditOffset, tab, token]);

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

  if (!ready) return null;
  if (!token) {
    return (
      <SignIn
        onSignedIn={(nextToken, pending) => {
          sessionStorage.setItem(TOKEN_KEY, nextToken);
          setBootstrapPending(pending);
          setToken(nextToken);
        }}
      />
    );
  }

  return (
    <main className="min-h-screen bg-gray-50 text-gray-900 dark:bg-gray-950 dark:text-gray-100">
      <div className="mx-auto max-w-7xl p-4 md:p-6">
        <header className="mb-6 flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <Shield className="h-6 w-6 text-blue-600" />
            <div>
              <h1 className="text-2xl font-bold">APAS System Administration</h1>
              <p className="text-sm text-gray-500">Every account, every virtual cluster, every project</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button aria-label="Refresh" onClick={() => void load()} className="rounded-lg border border-gray-300 p-2 dark:border-gray-700">
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            </button>
            <button
              onClick={() => { sessionStorage.removeItem(TOKEN_KEY); setToken(null); }}
              className="rounded-lg border border-gray-300 px-3 py-2 text-sm dark:border-gray-700"
            >
              Sign out
            </button>
          </div>
        </header>

        {bootstrapPending && (
          <form
            className="mb-6 rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200"
            onSubmit={(event) => {
              event.preventDefault();
              void (async () => {
                setError(null);
                try {
                  const body = await api<{ token: string }>("/admin/auth/password", {
                    method: "POST",
                    body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
                  });
                  sessionStorage.setItem(TOKEN_KEY, body.token);
                  setToken(body.token);
                  setBootstrapPending(false);
                  setCurrentPassword("");
                  setNewPassword("");
                } catch (cause) {
                  setError(cause instanceof Error ? cause.message : "Password change failed");
                }
              })();
            }}
          >
            <div className="font-semibold">This credential is still the one from the server configuration.</div>
            <p className="mt-1 text-xs">Rotate it now. Changing it signs out every other system-administration session.</p>
            <div className="mt-3 flex flex-col gap-2 sm:flex-row">
              <input aria-label="Current password" type="password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} placeholder="Current password" className="min-w-0 flex-1 rounded border border-amber-300 bg-transparent px-2 py-1 dark:border-amber-800" />
              <input aria-label="New password" type="password" value={newPassword} onChange={(event) => setNewPassword(event.target.value)} placeholder="New password (12+ characters)" className="min-w-0 flex-1 rounded border border-amber-300 bg-transparent px-2 py-1 dark:border-amber-800" />
              <button className="rounded bg-amber-600 px-3 py-1 text-white">Rotate</button>
            </div>
          </form>
        )}

        <nav className="mb-6 flex gap-1 overflow-x-auto rounded-xl border border-gray-200 bg-white p-1 dark:border-gray-800 dark:bg-gray-900">
          {(["overview", "users", "clusters", "projects", "audit"] as Tab[]).map((item) => (
            <button key={item} onClick={() => setTab(item)} className={`rounded-lg px-4 py-2 text-sm capitalize ${tab === item ? "bg-blue-600 text-white" : "hover:bg-gray-100 dark:hover:bg-gray-800"}`}>
              {item}
            </button>
          ))}
        </nav>

        <ErrorBanner message={error} />

        {tab === "overview" && stats && (
          <section className="mt-4 space-y-6">
            <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
              <StatCard label="Accounts" value={stats.total_users} detail={`${stats.recent_users_7d} added this week`} />
              <StatCard label="Projects / sessions" value={stats.total_sessions} detail={`${stats.active_sessions_24h} active in 24h`} />
              <StatCard label="CLI clients" value={stats.total_cli_clients} detail={`${stats.online_cli_clients} online`} />
              <StatCard label="Legacy shares" value={stats.total_shares} />
            </div>
            {defaultPolicy && (
              <PolicyEditor
                title="Deployment default policy"
                description="The outer bound for every virtual cluster. A cluster or project may narrow the allowed launch profiles but never widen them; team availability is a default that a cluster or project may set for itself."
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
                <thead className="bg-gray-100 dark:bg-gray-800"><tr><th className="p-3">Account</th><th className="p-3">Status</th><th className="p-3">Created</th></tr></thead>
                <tbody>{accounts.map((account) => <tr key={account.id} className="border-t border-gray-100 dark:border-gray-800">
                  <td className="p-3"><div>{account.email}</div><div className="text-xs text-gray-400">{account.id}</div></td>
                  <td className="p-3"><button onClick={() => void mutate(`/admin/users/${account.id}`, { method: "PATCH", body: JSON.stringify({ account_status: account.account_status === "active" ? "suspended" : "active" }) })} className={`rounded-full px-3 py-1 text-xs ${account.account_status === "active" ? "bg-green-100 text-green-700" : "bg-amber-100 text-amber-800"}`}>{account.account_status}</button></td>
                  <td className="p-3 text-gray-500">{account.created_at ? new Date(account.created_at).toLocaleString() : "—"}</td>
                </tr>)}</tbody>
              </table>
            </div>
          </section>
        )}

        {tab === "clusters" && (
          <section className="mt-4 space-y-3">
            <p className="text-sm text-gray-500">
              Every account operates one virtual cluster: the machines its clients registered and the projects hosted on them.
              A project may be owned by one account and hosted in another&apos;s cluster.
            </p>
            <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
              <table className="w-full text-left text-sm">
                <thead className="bg-gray-100 dark:bg-gray-800"><tr><th className="p-3">Cluster</th><th className="p-3">Hosted</th><th className="p-3">Owned</th><th className="p-3">Active sessions</th><th className="p-3">Last activity</th></tr></thead>
                <tbody>{clusters.map((cluster) => <tr key={cluster.user_id} className="border-t border-gray-100 dark:border-gray-800">
                  <td className="p-3">
                    <div className="flex items-center gap-2"><Server className="h-4 w-4 text-blue-500" />{cluster.email}</div>
                    <div className="text-xs text-gray-400">{cluster.user_id}{cluster.account_status !== "active" ? ` · ${cluster.account_status}` : ""}</div>
                  </td>
                  <td className="p-3">{cluster.hosted_project_count}</td>
                  <td className="p-3">{cluster.owned_project_count}</td>
                  <td className="p-3">{cluster.active_session_count}</td>
                  <td className="p-3 text-gray-500">{cluster.last_activity ? new Date(cluster.last_activity).toLocaleString() : "—"}</td>
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
                  <div className="truncate text-xs text-gray-500">Cluster {project.hosting_emails.length ? project.hosting_emails.join(", ") : "—"}</div>
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
            <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900"><table className="w-full text-left text-sm"><thead className="bg-gray-100 dark:bg-gray-800"><tr><th className="p-3">Time</th><th className="p-3">Actor</th><th className="p-3">Cluster</th><th className="p-3">Action</th><th className="p-3">Target</th><th className="p-3">Details</th></tr></thead><tbody>{audit.map((event) => <tr key={event.id} className="border-t border-gray-100 dark:border-gray-800"><td className="whitespace-nowrap p-3">{event.created_at ? new Date(event.created_at).toLocaleString() : "—"}</td><td className="p-3 font-mono text-xs">{event.actor_kind === "system_admin" ? "system administrator" : event.actor_user_id}</td><td className="p-3 font-mono text-xs">{event.cluster_user_id || "—"}</td><td className="p-3">{event.action}</td><td className="p-3">{event.target_type}: {event.target_id}</td><td className="max-w-xs truncate p-3 font-mono text-xs" title={event.details}>{event.details || "—"}</td></tr>)}</tbody></table></div>
            <div className="flex justify-between"><button disabled={auditOffset === 0} onClick={() => setAuditOffset(Math.max(0, auditOffset - 50))} className="rounded border px-3 py-1 disabled:opacity-40">Previous</button><button disabled={audit.length < 50} onClick={() => setAuditOffset(auditOffset + 50)} className="rounded border px-3 py-1 disabled:opacity-40">Next</button></div>
          </section>
        )}

        {loading && <div className="mt-6 flex items-center justify-center gap-2 text-sm text-gray-500"><Activity className="h-4 w-4 animate-pulse" /> Loading…</div>}
      </div>
    </main>
  );
}
