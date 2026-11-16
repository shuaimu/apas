"use client";

/**
 * The account's own virtual cluster: the machines its clients registered, plus
 * every project hosted on them — including projects another account owns. This
 * needs no special authority; the server scopes each request to the caller's
 * own cluster. Deployment-wide administration is a separate surface with its
 * own login and is deliberately not linked from anywhere in the app.
 *
 * The route stays /machines so existing links and bookmarks keep working.
 *
 * The page is split into views, chosen by `?view=`: Machines (the default),
 * Projects, Sharing, Usage, Policies and Activity. A shared cluster offers
 * Machines and Projects. This file keeps the data and the writes; each view is
 * drawn by its component under components/cluster.
 */

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Plus, RefreshCw } from "lucide-react";
import { useStore } from "@/lib/store";
import { openStateNegotiated } from "@/lib/projectOpenState";
import { CreateInstanceModal } from "@/components/CreateInstanceModal";
import type { LaunchProfile } from "@/components/PolicyEditor";
import { isRetiredLaunchProfileKey } from "@/lib/providerOptions";
import {
  isMachineBehind,
  latestSeenVersion,
  rebootActionLabelFor,
  rebootEffectNote,
  rebootLabelFor,
} from "@/lib/daemonVersion";
import {
  CLUSTER_VIEWS,
  MEMBER_VIEWS,
  clusterViewOffered,
  clusterViewUrl,
  parseClusterView,
  sortMachines,
} from "@/lib/clusterView";
import {
  OverviewNotice,
  overviewButton,
  overviewMuted,
  overviewPage,
  overviewPrimaryButton,
  overviewQuietButton,
} from "@/components/overview/OverviewElements";
import type { UsagePeriod } from "@/components/overview/UsagePeriodControl";
import { useConfirmDialog } from "@/components/cluster/ConfirmDialog";
import { ClusterViewTabs } from "@/components/cluster/ClusterViewTabs";
import { MachinesView } from "@/components/cluster/MachinesView";
import { ProjectsView } from "@/components/cluster/ProjectsView";
import { SharingView } from "@/components/cluster/SharingView";
import { UsageView } from "@/components/cluster/UsageView";
import { PoliciesView } from "@/components/cluster/PoliciesView";
import { ActivityView } from "@/components/cluster/ActivityView";
import {
  TRUST_WARNING,
  type AuditEvent,
  type ClusterMembership,
  type ClusterPolicy,
  type ClusterProject,
  type ClusterProjectDetail,
  type ClusterReference,
  type ClusterUsageReport,
  type ClusterView,
  type LoadState,
  type Mutate,
  type Page,
  type UsageWindows,
} from "@/components/cluster/types";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
const MOBILE_CLUSTER_STORAGE_KEY = "apas_mobile_cluster_owner";
const NOTICE_MS = 4000;

function message(cause: unknown, fallback = "Request failed"): string {
  return cause instanceof Error ? cause.message : fallback;
}

/** A success text that clears itself after a few seconds. */
function useTimedNotice(): [string | null, (value: string | null) => void] {
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);
  return [notice, setNotice];
}

export default function MachinesPage() {
  const router = useRouter();
  const {
    connected,
    connect,
    token,
    machines,
    sessions,
    negotiatedCapabilities,
    listMachines,
    openProject,
    closeProject,
    setMachineDeepseekConfig,
    pendingInstances,
    rebootDaemon,
    rebootDaemons,
    serverVersion,
    userId,
    userEmail,
  } = useStore();
  const { requestConfirm, confirmDialog } = useConfirmDialog();

  const [clusters, setClusters] = useState<ClusterReference[]>([]);
  const [clustersResolved, setClustersResolved] = useState(false);
  const [selectedClusterId, setSelectedClusterId] = useState("");
  const [clusterProjects, setClusterProjects] = useState<ClusterProject[]>([]);
  const [projectsLoad, setProjectsLoad] = useState<LoadState>("loading");
  // The cluster the project list was loaded for, so a `?selected=` project
  // is opened from the right cluster's list.
  const [projectsLoadedFor, setProjectsLoadedFor] = useState<string | null>(null);
  const [clusterPolicy, setClusterPolicy] = useState<ClusterPolicy | null>(null);
  const [policyLoad, setPolicyLoad] = useState<LoadState>("loading");
  const [profiles, setProfiles] = useState<LaunchProfile[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [auditLoad, setAuditLoad] = useState<LoadState>("loading");
  const [clusterMembers, setClusterMembers] = useState<ClusterMembership[]>([]);
  const [membersLoad, setMembersLoad] = useState<LoadState>("loading");
  const [usage, setUsage] = useState<ClusterUsageReport | null>(null);
  const [usageLoad, setUsageLoad] = useState<LoadState>("loading");
  const [usagePeriod, setUsagePeriod] = useState<UsagePeriod>("last_7d");

  const [pageError, setPageError] = useState<string | null>(null);
  const [pageNotice, setPageNotice] = useTimedNotice();

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selected, setSelected] = useState<ClusterProjectDetail | null>(null);
  const [selectedUsage, setSelectedUsage] = useState<UsageWindows | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [projectNotice, setProjectNotice] = useTimedNotice();
  const latestProjectRequest = useRef<string | null>(null);
  const pendingSelection = useRef<string | null>(null);
  // A deletion's end can be reported after this page is gone. From then on
  // it has no list to bring up to date, and the address is another page's.
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  // Deletions the server accepted whose end the Delete dialog did not hear
  // of: the project, and the cluster whose list has it.
  const unconfirmedDeletions = useRef(new Map<string, string>());
  // What the page holds now, for what runs after a wait or after the Delete
  // dialog is gone: the selection, the view and the cluster may have changed.
  const pageNow = useRef<{
    selectedClusterId: string;
    closeGoneProject: (projectId: string) => void;
    loadCluster: () => Promise<ClusterProject[] | null>;
  } | null>(null);

  const [requestedView, setRequestedView] = useState<ClusterView>("machines");
  const [createOpen, setCreateOpen] = useState(false);
  const [registrationTarget, setRegistrationTarget] = useState<{ machineId: string; projectId: string } | null>(null);
  const revealedRegistration = useRef(false);
  const [stoppedToggled, setStoppedToggled] = useState<Record<string, boolean>>({});

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const machineId = params.get("machine");
    const projectId = params.get("project");
    if (machineId && projectId) setRegistrationTarget({ machineId, projectId });
    // Whether the cluster offers the view is decided once the clusters are
    // known; until then the asked-for view is kept as asked.
    const asked = parseClusterView(window.location.search, true);
    setRequestedView(asked.view);
    pendingSelection.current = asked.selected;
  }, []);

  const selectedCluster = clusters.find((cluster) => cluster.owner_user_id === selectedClusterId);
  const sharedView = selectedCluster?.access === "member";
  const view: ClusterView = clusterViewOffered(requestedView, !sharedView) ? requestedView : "machines";
  // In one order for every view: connected first, then by name.
  const visibleMachines = useMemo(
    () => sortMachines(machines.filter((entry) => {
      if (!selectedClusterId) return entry.clusterAccess !== "member";
      if (entry.clusterOwnerUserId) return entry.clusterOwnerUserId === selectedClusterId;
      return !sharedView;
    })),
    [machines, selectedClusterId, sharedView],
  );

  // The row a "View on Machines" link names, revealed once. Its machine's
  // stopped projects start unfolded, so the row is there to find.
  useEffect(() => {
    if (!registrationTarget || revealedRegistration.current) return;
    const row = document.getElementById(`project-${encodeURIComponent(registrationTarget.machineId)}-${encodeURIComponent(registrationTarget.projectId)}`);
    if (!row) return;
    row.scrollIntoView?.({ block: "center" });
    row.focus();
    revealedRegistration.current = true;
  }, [registrationTarget, visibleMachines]);

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
  const rebootableMachines = useMemo(
    () => visibleMachines.filter((entry) => entry.clusterAccess !== "member"),
    [visibleMachines],
  );
  const behindDaemonCount = useMemo(
    () => rebootableMachines.filter(({ machine }) =>
      isMachineBehind(machine.daemonVersion, latestVersion)).length,
    [latestVersion, rebootableMachines],
  );
  const memberProfiles = useMemo(() => {
    const allowed = clusterPolicy?.cluster?.allowed_launch_profiles
      ?? clusterPolicy?.deployment.allowed_launch_profiles
      ?? [];
    return profiles.filter((profile) => allowed.includes(profile.key));
  }, [clusterPolicy, profiles]);
  // Open states come with the machine and session lists; nothing here
  // attaches to a project to learn them.
  const openState = useMemo(
    () => ({ supported: openStateNegotiated({ negotiatedCapabilities }), machines, sessions }),
    [machines, negotiatedCapabilities, sessions],
  );
  const activeClusterMembers = useMemo(
    () => clusterMembers.filter((member) => member.status === "active"),
    [clusterMembers],
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
      const result = await api<unknown>("/cluster/contexts");
      if (!Array.isArray(result)) return;
      const next = result as ClusterReference[];
      setClusters(next);
      setSelectedClusterId((current) => {
        const stored = typeof window === "undefined" ? null : localStorage.getItem(MOBILE_CLUSTER_STORAGE_KEY);
        const requested = new URLSearchParams(window.location.search).get("cluster_owner");
        const requestedOwner = requested === "owned"
          ? next.find((cluster) => cluster.access === "owner")?.owner_user_id
          : next.find((cluster) => cluster.owner_user_id === requested && cluster.access === "owner")?.owner_user_id;
        if (requestedOwner) {
          localStorage.setItem(MOBILE_CLUSTER_STORAGE_KEY, requestedOwner);
          return requestedOwner;
        }
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
    } finally {
      setClustersResolved(true);
    }
  }, [api, userEmail, userId]);

  useEffect(() => { void loadClusters(); }, [loadClusters]);

  // Only the newest load may write its answers: a reload after a write and a
  // cluster switch can overlap.
  const loadGeneration = useRef(0);
  // Answers with the cluster's projects as just loaded, or `null` when the
  // load failed or a newer one took over.
  const loadCluster = useCallback(async (): Promise<ClusterProject[] | null> => {
    const generation = ++loadGeneration.current;
    const current = () => generation === loadGeneration.current;
    const clusterId = selectedClusterId;
    try {
      const projectPath = sharedView && selectedClusterId
        ? `/cluster/contexts/${encodeURIComponent(selectedClusterId)}/projects?limit=200`
        : "/cluster/projects?limit=200";
      const policyPath = sharedView && selectedClusterId
        ? `/cluster/contexts/${encodeURIComponent(selectedClusterId)}/policy/default`
        : "/cluster/policy/default";
      const [projectPage, policy, launchProfiles] = await Promise.all([
        api<Page<ClusterProject>>(projectPath),
        api<ClusterPolicy>(policyPath),
        api<LaunchProfile[]>("/cluster/launch-profiles"),
      ]);
      if (!current()) return null;
      setClusterProjects(projectPage.items);
      setProjectsLoad("loaded");
      setProjectsLoadedFor(clusterId);
      // A deletion nobody reported done is over once its cluster's list no
      // longer has the project, whatever loaded that list. Another cluster's
      // list never had it and says nothing.
      for (const [projectId, listedIn] of unconfirmedDeletions.current) {
        if (projectPage.items.some((project) => project.id === projectId)) {
          unconfirmedDeletions.current.set(projectId, clusterId);
        } else if (listedIn === clusterId) {
          unconfirmedDeletions.current.delete(projectId);
          // The load may come back after the page is gone; see `mounted`.
          if (mounted.current) pageNow.current?.closeGoneProject(projectId);
        }
      }
      setClusterPolicy(policy);
      setPolicyLoad("loaded");
      setProfiles(launchProfiles.filter((profile) => !isRetiredLaunchProfileKey(profile.key)));
      if (sharedView) {
        setAudit([]);
        setClusterMembers([]);
        setUsage(null);
        setAuditLoad("loaded");
        setMembersLoad("loaded");
        setUsageLoad("loaded");
      } else {
        const [auditPage, memberRows, usageReport] = await Promise.all([
          api<Page<AuditEvent>>("/cluster/audit?limit=25"),
          api<unknown>("/cluster/members"),
          api<ClusterUsageReport>("/cluster/usage?limit=200"),
        ]);
        if (!current()) return null;
        setAudit(auditPage.items || []);
        setAuditLoad("loaded");
        setClusterMembers(Array.isArray(memberRows) ? memberRows as ClusterMembership[] : []);
        setMembersLoad("loaded");
        setUsage(usageReport?.projects ? usageReport : null);
        setUsageLoad("loaded");
      }
      setPageError(null);
      return projectPage.items;
    } catch (cause) {
      if (!current()) return null;
      const failed = (state: LoadState): LoadState => state === "loading" ? "failed" : state;
      setProjectsLoad(failed);
      setPolicyLoad(failed);
      setAuditLoad(failed);
      setMembersLoad(failed);
      setUsageLoad(failed);
      setPageError(message(cause));
      return null;
    }
  }, [api, selectedClusterId, sharedView]);

  // A different cluster is a different set of answers: say "Loading…" until
  // they arrive rather than an empty state that is not yet true. Nothing is
  // loaded before the clusters are known, so the first answers are already
  // the selected cluster's.
  const loadedClusterKey = useRef<string | null>(null);
  useEffect(() => {
    if (!clustersResolved) return;
    const key = `${selectedClusterId}:${sharedView}`;
    if (loadedClusterKey.current !== key) {
      loadedClusterKey.current = key;
      setProjectsLoad("loading");
      setPolicyLoad("loading");
      setAuditLoad("loading");
      setMembersLoad("loading");
      setUsageLoad("loading");
    }
    void loadCluster();
  }, [clustersResolved, loadCluster, selectedClusterId, sharedView]);

  // The server refuses a project outside this cluster, and that refusal is the
  // user-visible answer — surface it rather than letting the rejection escape
  // into an unhandled promise where it would look like nothing happened.
  const loadProject = useCallback(async (projectId: string, { reload = false } = {}) => {
    latestProjectRequest.current = projectId;
    if (!reload) {
      setProjectError(null);
      setProjectNotice(null);
      setSelected((current) => current?.project.id === projectId ? current : null);
      setDetailLoading(true);
    }
    try {
      const path = sharedView && selectedClusterId
        ? `/cluster/contexts/${encodeURIComponent(selectedClusterId)}/projects/${encodeURIComponent(projectId)}`
        : `/cluster/projects/${encodeURIComponent(projectId)}`;
      const [detail, projectUsage] = await Promise.all([
        api<ClusterProjectDetail>(path),
        api<UsageWindows>(`/projects/${encodeURIComponent(projectId)}/usage`),
      ]);
      if (latestProjectRequest.current !== projectId) return;
      setSelected(detail);
      setSelectedUsage(projectUsage);
    } catch (cause) {
      if (latestProjectRequest.current !== projectId) return;
      setSelected(null);
      setSelectedUsage(null);
      setProjectError(message(cause));
    } finally {
      if (latestProjectRequest.current === projectId) setDetailLoading(false);
    }
  }, [api, selectedClusterId, sharedView, setProjectNotice]);

  const writeUrl = useCallback((nextView: ClusterView, nextSelected: string | null) => {
    window.history.replaceState(null, "", clusterViewUrl(window.location, nextView, nextSelected));
  }, []);

  const selectProject = useCallback((projectId: string) => {
    setSelectedId(projectId);
    writeUrl("projects", projectId);
    void loadProject(projectId);
  }, [loadProject, writeUrl]);

  // `?view=projects&selected=<id>` opens that project once, after the list of
  // the cluster it belongs to has loaded.
  useEffect(() => {
    const pending = pendingSelection.current;
    if (!pending || !clustersResolved || projectsLoad !== "loaded" || projectsLoadedFor !== selectedClusterId) return;
    pendingSelection.current = null;
    selectProject(pending);
  }, [clustersResolved, projectsLoad, projectsLoadedFor, selectedClusterId, selectProject]);

  // Closes the selected project's panel. The address is not touched.
  const clearSelection = () => {
    latestProjectRequest.current = null;
    setSelectedId(null);
    setSelected(null);
    setSelectedUsage(null);
    setProjectError(null);
    setProjectNotice(null);
    setDetailLoading(false);
  };

  const clearProject = () => {
    clearSelection();
    writeUrl("projects", null);
  };

  // A project that is gone keeps no panel; another selection stays. The
  // address names the selected project only while the Projects view shows,
  // so it is written only then.
  const closeGoneProject = (projectId: string) => {
    if (selectedId !== projectId) return;
    clearSelection();
    if (view === "projects") writeUrl("projects", null);
  };

  useEffect(() => {
    pageNow.current = { selectedClusterId, closeGoneProject, loadCluster };
  });

  // The end of a deletion can arrive after its dialog is gone: the dialog
  // was closed during "Deleting…", and another project, another view or
  // another cluster may show by then.
  const handleProjectDeleted = useCallback((projectId: string) => {
    if (!mounted.current) return;
    unconfirmedDeletions.current.delete(projectId);
    pageNow.current?.closeGoneProject(projectId);
    listMachines();
    void pageNow.current?.loadCluster();
  }, [listMachines]);
  // The dialog stopped learning how an accepted deletion went. It may have
  // finished since, and this list is loaded over HTTP only when asked: load
  // it now, and again when the server reports the project gone. The delete
  // request itself is never sent again.
  const handleDeletionUnconfirmed = useCallback((projectId: string) => {
    const now = pageNow.current;
    if (!mounted.current || !now) return;
    unconfirmedDeletions.current.set(projectId, now.selectedClusterId);
    void now.loadCluster();
  }, []);
  // The server says once, on the socket, that a deletion is done, and the
  // store then forgets the project.
  useEffect(() => useStore.subscribe((state, previous) => {
    if (state.projectForgets === previous.projectForgets) return;
    const forgotten = [...unconfirmedDeletions.current.keys()].some((projectId) =>
      (state.projectForgets[projectId] ?? 0) > (previous.projectForgets[projectId] ?? 0));
    if (forgotten) void pageNow.current?.loadCluster();
  }), []);
  // A page that was not connected then missed it, and asks when it is
  // connected again.
  useEffect(() => {
    if (connected && unconfirmedDeletions.current.size > 0) void pageNow.current?.loadCluster();
  }, [connected]);

  const selectView = (next: ClusterView) => {
    setRequestedView(next);
    writeUrl(next, next === "projects" ? selectedId : null);
  };

  const openInProjects = (projectId: string) => {
    setRequestedView("projects");
    selectProject(projectId);
  };

  const mutate: Mutate = useCallback(async (path, init, { area, success, after }) => {
    const setError = area === "page" ? setPageError : setProjectError;
    const setNotice = area === "page" ? setPageNotice : setProjectNotice;
    setNotice(null);
    try {
      await api(path, init);
      setError(null);
      if (after) await after();
      await loadCluster();
      setNotice(success);
      return true;
    } catch (cause) {
      setError(message(cause));
      return false;
    }
  }, [api, loadCluster, setPageNotice, setProjectNotice]);

  const projectWrite = (path: string, init: RequestInit, success: string) => {
    const projectId = selected?.project.id;
    return mutate(path, init, {
      area: "project",
      success,
      after: projectId ? () => loadProject(projectId, { reload: true }) : undefined,
    });
  };

  const openReboot = ({ id, hostname, behind }: { id: string; hostname: string; behind: boolean }) => requestConfirm({
    ariaLabel: rebootActionLabelFor(behind, hostname),
    title: `${rebootActionLabelFor(behind, hostname)}?`,
    body: (
      <>
        <p>
          {behind
            ? "This machine is behind, so the reboot updates it first."
            : "It updates to the latest version if one is available."}
        </p>
        <p>{rebootEffectNote(hostname)}</p>
      </>
    ),
    confirmLabel: rebootLabelFor(behind),
    tone: "primary",
    onConfirm: () => rebootDaemon(id),
  });

  const openRebootAll = () => {
    const targets = rebootableMachines;
    requestConfirm({
      ariaLabel: "Reboot all daemons",
      title: "Reboot all daemons?",
      body: (
        <>
          <p>
            {targets.length === 1
              ? "This requests a reboot on the one machine in your cluster."
              : `This requests a reboot on all ${targets.length} machines in your cluster.`}
            {behindDaemonCount > 0
              ? ` ${behindDaemonCount} ${behindDaemonCount === 1 ? "daemon is" : "daemons are"} behind and will update first.`
              : " Each daemon updates first if a newer version is available."}
            {" Every project running on them restarts. Terminal agents normally keep running during the restart, because their hosts are separate processes."}
          </p>
          <p className="break-words text-xs">{targets.map(({ machine }) => machine.hostname).join(", ")}</p>
        </>
      ),
      confirmLabel: "Reboot all daemons",
      tone: "primary",
      onConfirm: () => rebootDaemons(targets.map(({ machine }) => machine.machineId)),
    });
  };

  const stoppedOpen = (machineId: string) =>
    stoppedToggled[machineId] ?? (registrationTarget?.machineId === machineId);

  const switchCluster = (clusterId: string) => {
    const next = clusters.find((cluster) => cluster.owner_user_id === clusterId);
    const nextView = clusterViewOffered(view, next?.access !== "member") ? view : "machines";
    latestProjectRequest.current = null;
    pendingSelection.current = null;
    setSelectedId(null);
    setSelected(null);
    setSelectedUsage(null);
    setProjectError(null);
    setRequestedView(nextView);
    setSelectedClusterId(clusterId);
    localStorage.setItem(MOBILE_CLUSTER_STORAGE_KEY, clusterId);
    writeUrl(nextView, null);
  };

  const views = sharedView ? MEMBER_VIEWS : CLUSTER_VIEWS;
  const counts: Partial<Record<ClusterView, number | undefined>> = {
    machines: visibleMachines.length,
    projects: projectsLoad === "loaded" ? clusterProjects.length : undefined,
    sharing: membersLoad === "loaded" ? activeClusterMembers.length : undefined,
  };

  return (
    <main className="min-h-screen bg-gray-50 text-gray-900 dark:bg-gray-950 dark:text-gray-100">
      <div className={overviewPage}>
        <div className="space-y-4">
          <div className="flex flex-wrap items-start gap-3">
            <div className="flex min-w-0 items-start gap-3">
              <Link href="/" className={overviewButton}>
                <ArrowLeft className="h-3.5 w-3.5" aria-hidden="true" /> Back
              </Link>
              <div className="min-w-0">
                <h1 className="text-lg font-semibold leading-8">{sharedView ? "Shared Cluster" : "My Cluster"}</h1>
                <p className={`break-words ${overviewMuted}`}>
                  {sharedView ? `Owned by ${selectedCluster?.owner_email}` : "Your machines and the projects hosted on them"}
                </p>
              </div>
            </div>
            <div className="ml-auto flex flex-wrap items-center gap-2">
              {clusters.length > 1 && (
                <select
                  aria-label="Selected cluster"
                  value={selectedClusterId}
                  onChange={(event) => switchCluster(event.target.value)}
                  className="h-8 max-w-56 rounded border border-gray-300 bg-white px-2 text-xs pointer-coarse:h-10 dark:border-gray-600 dark:bg-gray-800"
                >
                  {clusters.map((cluster) => (
                    <option key={cluster.owner_user_id} value={cluster.owner_user_id}>
                      {cluster.access === "owner" ? "My cluster" : cluster.owner_email}
                    </option>
                  ))}
                </select>
              )}
              <button
                type="button"
                aria-label="Refresh machines"
                onClick={() => { listMachines(); void loadClusters(); void loadCluster(); }}
                className={overviewButton}
              >
                <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" /> Refresh
              </button>
              <button
                type="button"
                onClick={() => setCreateOpen(true)}
                disabled={sharedView && !visibleMachines.some((machine) => machine.sharedProvisioningAvailable)}
                className={overviewPrimaryButton}
              >
                <Plus className="h-3.5 w-3.5" aria-hidden="true" /> New project
              </button>
            </div>
          </div>

          {sharedView && (
            <div role="note" className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
              <strong className="font-semibold">Trusted compute boundary</strong> {TRUST_WARNING}
            </div>
          )}

          {pageError && (
            <div role="alert" className="flex items-start justify-between gap-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
              <span className="min-w-0 break-words">{pageError}</span>
              <button type="button" onClick={() => setPageError(null)} className={`${overviewQuietButton} -my-0.5`}>Dismiss</button>
            </div>
          )}
          {pageNotice && <OverviewNotice status>{pageNotice}</OverviewNotice>}

          <ClusterViewTabs views={views} selected={view} counts={counts} onSelect={selectView} />
        </div>

        <div role="tabpanel" id={`cluster-view-${view}`} aria-labelledby={`cluster-tab-${view}`} className="flex flex-col gap-8">
          {view === "machines" && (
            <MachinesView
              machines={visibleMachines}
              sharedView={sharedView}
              latestVersion={latestVersion}
              connected={connected}
              pendingInstances={pendingInstances}
              registrationTarget={registrationTarget}
              stoppedOpen={stoppedOpen}
              onToggleStopped={(machineId) => setStoppedToggled((current) => ({ ...current, [machineId]: !stoppedOpen(machineId) }))}
              canRebootAll={!sharedView && rebootableMachines.length > 0}
              onReboot={openReboot}
              onRebootAll={openRebootAll}
              openStateSupported={openState.supported}
              projectMachines={machines}
              onOpen={(machineId, projectId) => openProject(machineId, projectId)}
              onClose={(machineId, projectId) => closeProject(machineId, projectId)}
              setMachineDeepseekConfig={setMachineDeepseekConfig}
              requestConfirm={requestConfirm}
            />
          )}
          {view === "projects" && (
            <ProjectsView
              sharedView={sharedView}
              projects={clusterProjects}
              load={projectsLoad}
              userId={userId}
              openState={openState}
              selectedId={selectedId}
              onSelect={selectProject}
              onClear={clearProject}
              onDeleted={handleProjectDeleted}
              onDeletionUnconfirmed={handleDeletionUnconfirmed}
              detail={selected}
              detailUsage={selectedUsage}
              detailLoading={detailLoading}
              projectError={projectError}
              projectNotice={projectNotice}
              usagePeriod={usagePeriod}
              onUsagePeriod={setUsagePeriod}
              profiles={profiles}
              clusterPolicy={clusterPolicy}
              write={projectWrite}
              requestConfirm={requestConfirm}
              request={api}
            />
          )}
          {view === "sharing" && (
            <SharingView
              members={activeClusterMembers}
              load={membersLoad}
              machines={visibleMachines}
              profiles={memberProfiles}
              mutate={mutate}
              requestConfirm={requestConfirm}
            />
          )}
          {view === "usage" && (
            <UsageView
              usage={usage}
              load={usageLoad}
              period={usagePeriod}
              onPeriod={setUsagePeriod}
              clusterProjects={clusterProjects}
              userEmail={userEmail}
              onOpenProject={openInProjects}
            />
          )}
          {view === "policies" && (
            <PoliciesView
              clusterPolicy={clusterPolicy}
              load={policyLoad}
              profiles={profiles}
              mutate={mutate}
              requestConfirm={requestConfirm}
              request={api}
            />
          )}
          {view === "activity" && (
            <ActivityView
              events={audit}
              load={auditLoad}
              projects={clusterProjects}
              members={clusterMembers}
              userId={userId}
              onOpenProject={openInProjects}
            />
          )}
        </div>
      </div>

      {confirmDialog}
      <CreateInstanceModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        clusterOwnerUserId={selectedClusterId || undefined}
      />
    </main>
  );
}
