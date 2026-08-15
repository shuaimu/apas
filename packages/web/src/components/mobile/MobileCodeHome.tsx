"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, ChevronRight, Plus, RotateCcw, WifiOff, X } from "lucide-react";
import type { SessionInfo } from "@/lib/store";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";

// "machines" is a third selection rather than a screen: the home already
// switches lists, and a handful of machine rows does not warrant navigation.
type HomeView = "all" | "idle" | "machines";
type SessionFilter = "all" | "idle";

interface MobileSessionSummary {
  id: string;
  project_id?: string | null;
  project_name?: string | null;
  hostname?: string | null;
  working_dir?: string | null;
  status: string;
  is_active?: boolean;
  is_working?: boolean;
  latest_update_at?: string | null;
  last_user_input_at?: string | null;
  latest_summary?: string | null;
  attention_count?: number;
  is_shared?: boolean;
  owner_email?: string | null;
}

interface MobileMachineProject {
  project_id: string;
  is_running?: boolean;
}

interface MobileMachineSummary {
  machine: {
    machine_id: string;
    hostname: string;
    os?: string | null;
    arch?: string | null;
    last_seen?: string | null;
  };
  projects?: MobileMachineProject[];
}

interface MobileBootstrapResponse {
  sessions: MobileSessionSummary[];
  /// Already on the wire and previously discarded here — the machines list
  /// costs no extra request.
  machines?: MobileMachineSummary[];
}

const FILTERS: { key: HomeView; label: string }[] = [
  { key: "all", label: "All projects" },
  { key: "idle", label: "Idle projects" },
  { key: "machines", label: "Machines" },
];

/// A machine is connected when its daemon has reported recently. The daemon
/// heartbeats every 10s; a minute of silence is a disconnect rather than a
/// slow tick.
const MACHINE_STALE_MS = 60_000;

function machineConnected(machine: MobileMachineSummary): boolean {
  const lastSeen = machine.machine.last_seen;
  if (!lastSeen) return false;
  const at = Date.parse(lastSeen);
  return Number.isFinite(at) && Date.now() - at < MACHINE_STALE_MS;
}

function projectName(session: SessionInfo): string {
  const value = session.gitRemote?.split("/").pop()
    ?? session.workingDir?.replace(/\/$/, "").split("/").pop();
  return value || `Project ${(session.projectId || session.id).slice(0, 8)}`;
}

function adaptSession(session: SessionInfo): MobileSessionSummary {
  return {
    id: session.id,
    project_id: session.projectId,
    project_name: projectName(session),
    hostname: session.hostname,
    working_dir: session.workingDir,
    status: session.status,
    is_active: session.isActive,
    is_working: session.isWorking,
    latest_update_at: session.createdAt,
    attention_count: 0,
    is_shared: session.isShared,
    owner_email: session.ownerEmail,
  };
}

function matches(session: MobileSessionSummary, filter: SessionFilter): boolean {
  return filter === "all" || statusLabel(session) === "Idle";
}

function timestamp(value?: string | null): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isNaN(parsed) ? Number.NEGATIVE_INFINITY : parsed;
}

function compareSessionRecency(left: MobileSessionSummary, right: MobileSessionSummary): number {
  const leftHasUserInput = Boolean(left.last_user_input_at);
  const rightHasUserInput = Boolean(right.last_user_input_at);
  if (leftHasUserInput !== rightHasUserInput) return rightHasUserInput ? 1 : -1;
  return timestamp(right.last_user_input_at) - timestamp(left.last_user_input_at)
    || timestamp(right.latest_update_at) - timestamp(left.latest_update_at)
    || left.id.localeCompare(right.id);
}

function statusLabel(session: MobileSessionSummary): string {
  if (!session.is_active) return "Offline";
  return session.is_working ? "Working" : "Idle";
}

function sessionTarget(session: MobileSessionSummary): string {
  return session.hostname || session.working_dir || "Unknown target";
}

function formatUpdatedAt(value?: string | null): string {
  if (!value) return "No recent activity";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? "No recent activity" : parsed.toLocaleString();
}

export interface MobileCodeHomeProps {
  active: boolean;
  connected: boolean;
  legacySessions: SessionInfo[];
  token: string | null;
  onAccount: () => void;
  onManageMachines: () => void;
  onOpenSession: (sessionId: string, projectName: string) => void;
  /// Reboot the daemon on a machine. Targeted by machine id: a daemon is
  /// per-machine, so no project on it identifies the right one.
  onRebootDaemon: (machineId: string, hostname: string) => void;
}

export function MobileCodeHome({
  active,
  connected,
  legacySessions,
  token,
  onAccount,
  onManageMachines,
  onOpenSession,
  onRebootDaemon,
}: MobileCodeHomeProps) {
  const [remoteSessions, setRemoteSessions] = useState<MobileSessionSummary[] | null>(null);
  const [filter, setFilter] = useState<HomeView>("all");
  const [machines, setMachines] = useState<MobileMachineSummary[]>([]);
  const [machineRebootTarget, setMachineRebootTarget] =
    useState<{ id: string; hostname: string } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [newTaskOpen, setNewTaskOpen] = useState(false);

  const refresh = useCallback(async (signal?: AbortSignal) => {
    if (!active || !token) return;
    try {
      const response = await fetch(`${API_URL}/mobile/v1/bootstrap`, {
        headers: { Authorization: `Bearer ${token}` },
        signal,
      });
      if (!response.ok) throw new Error(`Request failed (${response.status})`);
      const bootstrap = await response.json() as MobileBootstrapResponse;
      setRemoteSessions(Array.isArray(bootstrap.sessions) ? bootstrap.sessions : []);
      setMachines(Array.isArray(bootstrap.machines) ? bootstrap.machines : []);
      setLoadError(null);
    } catch (error) {
      if (signal?.aborted) return;
      setLoadError(error instanceof Error ? error.message : "Could not refresh coding sessions");
    }
  }, [active, token]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh, connected]);

  const sessions = useMemo(() => {
    const legacyById = new Map(legacySessions.map((session) => [session.id, session]));
    if (!remoteSessions) return legacySessions.map(adaptSession);
    return remoteSessions.map((session) => {
      const live = legacyById.get(session.id);
      if (!live) return session;
      return {
        ...session,
        status: live.status || session.status,
        is_active: live.isActive ?? session.is_active,
        // Once the WebSocket inventory has this session it is the live
        // authority. Do not preserve a stale REST `true` merely because an
        // older/mixed-version inventory omitted isWorking.
        is_working: Boolean(live.isWorking),
        hostname: live.hostname ?? session.hostname,
        working_dir: live.workingDir ?? session.working_dir,
      };
    });
  }, [legacySessions, remoteSessions]);

  const filteredSessions = useMemo(
    () => sessions
      // The machines view lists no sessions; keep the session filter total
      // over the two values it was written for.
      .filter((session) => filter !== "machines" && matches(session, filter))
      .sort(compareSessionRecency),
    [filter, sessions],
  );
  const activeSessions = useMemo(
    () => sessions.filter((session) => session.is_active).sort(compareSessionRecency),
    [sessions],
  );

  return (
    <section
      aria-label="Mobile coding sessions"
      className="flex h-full min-h-0 w-full flex-col overflow-hidden bg-[#f7f7fa] text-[#18181b] dark:bg-[#111115] dark:text-[#f7f7fa]"
    >
      {!connected && (
        <div className="flex items-center gap-2 border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs font-medium text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <WifiOff className="h-4 w-4" /> Offline · session actions are unavailable
        </div>
      )}

      <div className="flex items-start justify-between gap-3 px-4 pt-4">
        <div className="min-w-0 flex-1">
          <h1 className="text-[1.35rem] font-extrabold tracking-tight">Coding sessions</h1>
          <p className="mt-0.5 text-sm text-[#686873] dark:text-[#aaaab6]">Active work and recent outcomes</p>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <button
            type="button"
            onClick={onAccount}
            className="min-h-9 px-1 text-sm font-bold text-[#6d5efc] hover:text-[#5547dc]"
          >
            Account
          </button>
          <button
            type="button"
            onClick={() => setNewTaskOpen(true)}
            className="inline-flex min-h-10 items-center gap-1.5 rounded-xl bg-[#6d5efc] px-3.5 text-sm font-bold text-white shadow-sm hover:bg-[#5547dc]"
          >
            <Plus className="h-4 w-4" /> New task
          </button>
        </div>
      </div>

      <div className="no-scrollbar flex shrink-0 gap-2 overflow-x-auto px-4 pt-3.5 pb-1">
        {FILTERS.map((item) => {
          const selected = filter === item.key;
          return (
            <button
              key={item.key}
              type="button"
              aria-pressed={selected}
              onClick={() => setFilter(item.key)}
              className={`shrink-0 rounded-full px-3.5 py-2 text-sm font-semibold ${
                selected
                  ? "bg-[#6d5efc] text-white"
                  : "bg-[#efeff5] text-[#18181b] hover:bg-[#e4e4ec] dark:bg-[#25252d] dark:text-[#f7f7fa] dark:hover:bg-[#303039]"
              }`}
            >
              {item.label}
            </button>
          );
        })}
      </div>

      {loadError && sessions.length > 0 && (
        <div className="mx-4 mt-2 flex items-center gap-2 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <AlertTriangle className="h-4 w-4 shrink-0" /> Showing the last session list; refresh will retry automatically.
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {filter === "machines" ? (
          machines.length > 0 ? (
            <div className="space-y-2.5">
              {machines.map((entry) => {
                const hostname = entry.machine.hostname || "Unknown machine";
                const connected = machineConnected(entry);
                const running = (entry.projects ?? []).filter((project) => project.is_running).length;
                return (
                  <div
                    key={entry.machine.machine_id}
                    className="rounded-2xl border border-[#dedee7] bg-white p-3.5 shadow-sm dark:border-[#383842] dark:bg-[#1b1b21]"
                  >
                    <div className="flex items-center justify-between gap-2.5">
                      <span className="min-w-0 flex-1 truncate text-base font-bold">{hostname}</span>
                      <span className={`shrink-0 rounded-full px-2.5 py-1 text-[0.7rem] font-bold ${
                        connected
                          ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300"
                          : "bg-red-100 text-red-700 dark:bg-red-950/60 dark:text-red-300"
                      }`}>
                        {connected ? "Connected" : "Offline"}
                      </span>
                    </div>
                    <p className="mt-2 truncate text-sm text-[#686873] dark:text-[#aaaab6]">
                      {[entry.machine.os, entry.machine.arch].filter(Boolean).join("/") || "Unknown platform"}
                      {" · "}
                      {running === 1 ? "1 project running" : `${running} projects running`}
                    </p>
                    <div className="mt-2.5 flex items-center justify-end">
                      <button
                        type="button"
                        aria-label={`Reboot daemon on ${hostname}`}
                        onClick={() => setMachineRebootTarget({ id: entry.machine.machine_id, hostname })}
                        className="inline-flex items-center gap-1.5 rounded-xl border border-[#dedee7] px-3 py-2 text-sm font-semibold active:opacity-60 dark:border-[#383842]"
                      >
                        <RotateCcw className="h-4 w-4" /> Reboot daemon
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="flex h-full min-h-52 flex-col items-center justify-center px-5 text-center">
              <div className="mb-3 rounded-2xl bg-[#efeff5] p-3 dark:bg-[#25252d]">
                <AlertTriangle className="h-6 w-6 text-[#686873] dark:text-[#aaaab6]" />
              </div>
              <h2 className="text-lg font-extrabold">No machines yet</h2>
              <p className="mt-1.5 max-w-sm text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
                Run `apas daemon` on a machine and it will appear here.
              </p>
            </div>
          )
        ) : filteredSessions.length > 0 ? (
          <div className="space-y-2.5">
            {filteredSessions.map((session) => {
              const name = session.project_name || "Coding session";
              const attention = session.attention_count ?? 0;
              return (
                <button
                  key={session.id}
                  type="button"
                  aria-label={`Open ${name}`}
                  onClick={() => onOpenSession(session.id, name)}
                  className="w-full rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left shadow-sm transition hover:border-[#bdbdc9] active:opacity-75 dark:border-[#383842] dark:bg-[#1b1b21] dark:hover:border-[#50505c]"
                >
                  <div className="flex items-center justify-between gap-2.5">
                    <span className="min-w-0 flex-1 truncate text-base font-bold">{name}</span>
                    <span className={`shrink-0 rounded-full px-2.5 py-1 text-[0.7rem] font-bold ${
                      session.is_working
                        ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300"
                        : session.is_active
                          ? "bg-[#efeff5] text-[#686873] dark:bg-[#25252d] dark:text-[#aaaab6]"
                          : "bg-red-100 text-red-700 dark:bg-red-950/60 dark:text-red-300"
                    }`}>
                      {statusLabel(session)}
                    </span>
                  </div>
                  <p className="mt-2 truncate text-sm text-[#686873] dark:text-[#aaaab6]">{sessionTarget(session)}</p>
                  {session.latest_summary && <p className="mt-2 line-clamp-2 text-sm leading-5">{session.latest_summary}</p>}
                  <div className="mt-2.5 flex items-center justify-between gap-2.5">
                    <span className="min-w-0 flex-1 truncate text-xs text-[#686873] dark:text-[#aaaab6]">{formatUpdatedAt(session.latest_update_at)}</span>
                    {attention > 0 && (
                      <span className="shrink-0 rounded-full bg-amber-100 px-2.5 py-1 text-[0.7rem] font-bold text-amber-800 dark:bg-amber-950/60 dark:text-amber-200">
                        {attention} attention
                      </span>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <div className="flex h-full min-h-52 flex-col items-center justify-center px-5 text-center">
            <div className="mb-3 rounded-2xl bg-[#efeff5] p-3 dark:bg-[#25252d]">
              <AlertTriangle className="h-6 w-6 text-[#686873] dark:text-[#aaaab6]" />
            </div>
            <h2 className="text-lg font-extrabold">{sessions.length ? "No idle projects" : "No coding sessions yet"}</h2>
            <p className="mt-1.5 max-w-sm text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
              {sessions.length ? "All connected projects are currently working." : "Start APAS in a project, then follow its coding activity here."}
            </p>
            <button
              type="button"
              onClick={() => setNewTaskOpen(true)}
              className="mt-4 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#5547dc]"
            >
              Start a task
            </button>
          </div>
        )}
      </div>

      {machineRebootTarget && (
        <div className="fixed inset-0 z-[90] flex items-end bg-black/45" onClick={() => setMachineRebootTarget(null)}>
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="mobile-machine-reboot-title"
            className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]"
            onClick={(event) => event.stopPropagation()}
          >
            <h2 id="mobile-machine-reboot-title" className="text-lg font-extrabold">
              Reboot the daemon on {machineRebootTarget.hostname}?
            </h2>
            <p className="mt-1.5 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
              It updates to the latest version if one is available. Projects, panes, and agents on
              this machine keep running — the daemon does not own them.
            </p>
            <div className="mt-4 flex gap-2.5">
              <button
                type="button"
                onClick={() => setMachineRebootTarget(null)}
                className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  onRebootDaemon(machineRebootTarget.id, machineRebootTarget.hostname);
                  setMachineRebootTarget(null);
                }}
                className="flex-1 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#5547dc]"
              >
                Reboot daemon
              </button>
            </div>
          </div>
        </div>
      )}

      {newTaskOpen && (
        <div className="fixed inset-0 z-[90] flex items-end bg-black/45" onClick={() => setNewTaskOpen(false)}>
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="mobile-new-task-title"
            className="max-h-[82dvh] w-full overflow-y-auto rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <h2 id="mobile-new-task-title" className="text-xl font-extrabold">Start coding work</h2>
                <p className="mt-1 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
                  Choose a running project, then use its + control to create the coding pane.
                </p>
              </div>
              <button type="button" aria-label="Close new task" onClick={() => setNewTaskOpen(false)} className="rounded-lg p-2 hover:bg-[#efeff5] dark:hover:bg-[#25252d]">
                <X className="h-5 w-5" />
              </button>
            </div>
            <div className="mt-4 space-y-2">
              {activeSessions.map((session) => {
                const name = session.project_name || "Coding session";
                return (
                  <button
                    key={session.id}
                    type="button"
                    onClick={() => onOpenSession(session.id, name)}
                    className="flex w-full items-center justify-between gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left dark:border-[#383842] dark:bg-[#1b1b21]"
                  >
                    <span className="min-w-0">
                      <span className="block truncate font-bold">{name}</span>
                      <span className="mt-0.5 block truncate text-xs text-[#686873] dark:text-[#aaaab6]">{sessionTarget(session)}</span>
                    </span>
                    <ChevronRight className="h-5 w-5 shrink-0 text-[#6d5efc]" />
                  </button>
                );
              })}
              {activeSessions.length === 0 && (
                <div className="rounded-2xl border border-[#dedee7] bg-white p-4 text-sm text-[#686873] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#aaaab6]">
                  No running projects are available. Start one from Machines first.
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={onManageMachines}
              className="mt-4 w-full rounded-xl border border-[#dedee7] bg-white px-4 py-3 text-sm font-bold dark:border-[#383842] dark:bg-[#1b1b21]"
            >
              Manage machines and projects
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
