import { create } from "zustand";
import {
  deriveAttention,
  type CodeEvent,
  type MobileBootstrapResponse,
  type MobileFeatureFlags,
  type MobileLaunchTarget,
  type MobileSessionSummary,
  type PaneConfig,
  type PaneWorkSummary,
  type ServerToWeb,
} from "@apas/protocol";

import { paneStatusIsWorking } from "./paneStatus";

export type ConnectionPhase = "offline" | "connecting" | "authenticating" | "synchronizing" | "ready";
export const PANE_WORK_SUMMARY_CAPABILITY = "pane_work_summary_v1";

export type PaneWorkSummaryAvailability = NonNullable<Extract<ServerToWeb, {
  type: "pane_work_summaries";
}>["availability"]>;

export interface PaneWorkSummaryCache {
  summaries: PaneWorkSummary[];
  availability: PaneWorkSummaryAvailability;
  loading: boolean;
  error: string | null;
  updatedAt: string | null;
}

export interface VisibleSummaryPane {
  sessionId: string;
  paneId: number;
}

interface MobileState {
  hydrated: boolean;
  signedIn: boolean;
  userEmail: string | null;
  connection: ConnectionPhase;
  lastUpdatedAt: string | null;
  sessions: MobileSessionSummary[];
  launchTargets: MobileLaunchTarget[];
  features: MobileFeatureFlags;
  activeSessionId: string | null;
  eventsBySession: Record<string, CodeEvent[]>;
  panesBySession: Record<string, PaneConfig[]>;
  paneStatusesBySession: Record<string, Record<string, string>>;
  negotiatedCapabilities: string[];
  paneWorkSummaries: Record<string, PaneWorkSummaryCache>;
  visibleSummaryPane: VisibleSummaryPane | null;
  terminalReady: boolean;
  terminalPaneId: number | null;
  serverMutationsAllowed: boolean;
  error: string | null;
  setHydrated: (signedIn: boolean) => void;
  setConnection: (connection: ConnectionPhase) => void;
  setServerMutationsAllowed: (allowed: boolean) => void;
  setNegotiatedCapabilities: (capabilities: string[]) => void;
  applyBootstrap: (bootstrap: MobileBootstrapResponse) => void;
  setCachedSessions: (sessions: MobileSessionSummary[], updatedAt: string | null) => void;
  setActiveSession: (sessionId: string | null) => void;
  markSessionUserInput: (sessionId: string, createdAt?: string) => void;
  setEvents: (sessionId: string, events: CodeEvent[]) => void;
  setPanes: (sessionId: string, panes: PaneConfig[]) => void;
  setPaneStatus: (sessionId: string, paneId: number, status: string | null | undefined) => void;
  setSessionActive: (sessionId: string, active: boolean) => void;
  beginPaneWorkSummaryRequest: (sessionId: string, paneId: number) => void;
  setPaneWorkSummaryError: (sessionId: string, paneId: number, error: string | null) => void;
  replacePaneWorkSummaries: (
    sessionId: string,
    paneId: number,
    summaries: PaneWorkSummary[],
    availability?: PaneWorkSummaryAvailability,
    updatedAt?: string,
  ) => void;
  hydratePaneWorkSummaries: (
    sessionId: string,
    paneId: number,
    summaries: PaneWorkSummary[],
    availability: PaneWorkSummaryAvailability,
    updatedAt: string,
  ) => void;
  upsertPaneWorkSummary: (
    sessionId: string,
    paneId: number,
    summary: PaneWorkSummary,
    availability?: PaneWorkSummaryAvailability,
    updatedAt?: string,
  ) => void;
  setVisibleSummaryPane: (pane: VisibleSummaryPane | null) => void;
  setTerminal: (paneId: number | null, ready: boolean) => void;
  setError: (error: string | null) => void;
  reset: () => void;
}

const initialState = {
  hydrated: false,
  signedIn: false,
  userEmail: null,
  connection: "offline" as const,
  lastUpdatedAt: null,
  sessions: [] as MobileSessionSummary[],
  launchTargets: [] as MobileLaunchTarget[],
  features: {} as MobileFeatureFlags,
  activeSessionId: null,
  eventsBySession: {} as Record<string, CodeEvent[]>,
  panesBySession: {} as Record<string, PaneConfig[]>,
  paneStatusesBySession: {} as Record<string, Record<string, string>>,
  negotiatedCapabilities: [] as string[],
  paneWorkSummaries: {} as Record<string, PaneWorkSummaryCache>,
  visibleSummaryPane: null as VisibleSummaryPane | null,
  terminalReady: false,
  terminalPaneId: null,
  serverMutationsAllowed: false,
  error: null,
};

export const useMobileStore = create<MobileState>((set) => ({
  ...initialState,
  setHydrated: (signedIn) => set({ hydrated: true, signedIn }),
  setConnection: (connection) => set({ connection }),
  setServerMutationsAllowed: (serverMutationsAllowed) => set({ serverMutationsAllowed }),
  setNegotiatedCapabilities: (negotiatedCapabilities) => set({ negotiatedCapabilities: [...new Set(negotiatedCapabilities)] }),
  applyBootstrap: (bootstrap) =>
    set((state) => {
      const allowed = new Set(bootstrap.sessions.map((session) => session.id));
      return {
        signedIn: true,
        userEmail: bootstrap.user_email,
        sessions: bootstrap.sessions,
        launchTargets: bootstrap.launch_targets,
        features: bootstrap.features,
        panesBySession: Object.fromEntries(
          Object.entries(state.panesBySession).filter(([sessionId]) => allowed.has(sessionId)),
        ),
        paneStatusesBySession: Object.fromEntries(
          Object.entries(state.paneStatusesBySession).filter(([sessionId]) => allowed.has(sessionId)),
        ),
        paneWorkSummaries: Object.fromEntries(
          Object.entries(state.paneWorkSummaries).filter(([key]) => allowed.has(summarySessionId(key))),
        ),
        visibleSummaryPane: state.visibleSummaryPane && allowed.has(state.visibleSummaryPane.sessionId)
          ? state.visibleSummaryPane
          : null,
        lastUpdatedAt: new Date().toISOString(),
        error: null,
      };
    }),
  setCachedSessions: (sessions, lastUpdatedAt) => set({ sessions, lastUpdatedAt }),
  setActiveSession: (activeSessionId) => set({ activeSessionId }),
  markSessionUserInput: (sessionId, createdAt = new Date().toISOString()) =>
    set((state) => ({
      sessions: state.sessions.map((session) => session.id === sessionId
        ? {
            ...session,
            last_user_input_at: createdAt,
            latest_update_at: createdAt,
          }
        : session),
    })),
  setEvents: (sessionId, events) =>
    set((state) => {
      const latest = events.at(-1);
      const latestUserInput = events.reduce<CodeEvent | undefined>(
        (current, event) => event.kind === "instruction"
          && (!current || event.ordering_key > current.ordering_key)
          ? event
          : current,
        undefined,
      );
      return {
        eventsBySession: { ...state.eventsBySession, [sessionId]: events },
        sessions: state.sessions.map((session) => session.id === sessionId
          ? {
              ...session,
              latest_summary: latest?.summary ?? session.latest_summary,
              latest_update_at: latest?.created_at ?? session.latest_update_at,
              last_user_input_at: latestUserInput?.created_at ?? session.last_user_input_at,
              attention_count: deriveAttention(events).length,
            }
          : session),
      };
    }),
  setPanes: (sessionId, panes) =>
    set((state) => ({ panesBySession: { ...state.panesBySession, [sessionId]: panes } })),
  setPaneStatus: (sessionId, paneId, status) =>
    set((state) => {
      const sessionStatuses = { ...(state.paneStatusesBySession[sessionId] ?? {}) };
      if (status) sessionStatuses[String(paneId)] = status;
      else delete sessionStatuses[String(paneId)];
      return {
        sessions: state.sessions.map((session) => session.id === sessionId
          ? { ...session, is_working: Object.values(sessionStatuses).some(paneStatusIsWorking) }
          : session),
        paneStatusesBySession: {
          ...state.paneStatusesBySession,
          [sessionId]: sessionStatuses,
        },
      };
    }),
  setSessionActive: (sessionId, active) =>
    set((state) => {
      const paneStatusesBySession = { ...state.paneStatusesBySession };
      if (!active) delete paneStatusesBySession[sessionId];
      return {
        sessions: state.sessions.map((session) => session.id === sessionId
          ? { ...session, is_active: active, is_working: active && Boolean(session.is_working) }
          : session),
        paneStatusesBySession,
      };
    }),
  beginPaneWorkSummaryRequest: (sessionId, paneId) =>
    set((state) => {
      const key = paneWorkSummaryKey(sessionId, paneId);
      return {
        paneWorkSummaries: {
          ...state.paneWorkSummaries,
          [key]: {
            ...emptyPaneWorkSummaryCache(),
            ...state.paneWorkSummaries[key],
            loading: true,
            error: null,
          },
        },
      };
    }),
  setPaneWorkSummaryError: (sessionId, paneId, error) =>
    set((state) => {
      const key = paneWorkSummaryKey(sessionId, paneId);
      return {
        paneWorkSummaries: {
          ...state.paneWorkSummaries,
          [key]: {
            ...emptyPaneWorkSummaryCache(),
            ...state.paneWorkSummaries[key],
            loading: false,
            error,
          },
        },
      };
    }),
  replacePaneWorkSummaries: (sessionId, paneId, summaries, availability = "unknown", updatedAt = new Date().toISOString()) =>
    set((state) => ({
      paneWorkSummaries: {
        ...state.paneWorkSummaries,
        [paneWorkSummaryKey(sessionId, paneId)]: {
          summaries: normalizePaneWorkSummaries(sessionId, paneId, summaries),
          availability,
          loading: false,
          error: null,
          updatedAt,
        },
      },
    })),
  hydratePaneWorkSummaries: (sessionId, paneId, summaries, availability, updatedAt) =>
    set((state) => {
      const key = paneWorkSummaryKey(sessionId, paneId);
      const current = state.paneWorkSummaries[key];
      if (current?.updatedAt && current.updatedAt > updatedAt) return state;
      return {
        paneWorkSummaries: {
          ...state.paneWorkSummaries,
          [key]: {
            summaries: normalizePaneWorkSummaries(sessionId, paneId, summaries),
            availability,
            loading: current?.loading ?? false,
            error: current?.error ?? null,
            updatedAt,
          },
        },
      };
    }),
  upsertPaneWorkSummary: (sessionId, paneId, summary, availability, updatedAt = new Date().toISOString()) =>
    set((state) => {
      if (summary.session_id !== sessionId || summary.pane_id !== paneId) return state;
      const key = paneWorkSummaryKey(sessionId, paneId);
      const current = state.paneWorkSummaries[key] ?? emptyPaneWorkSummaryCache();
      return {
        paneWorkSummaries: {
          ...state.paneWorkSummaries,
          [key]: {
            summaries: normalizePaneWorkSummaries(sessionId, paneId, [
              ...current.summaries.filter((item) => item.window_start !== summary.window_start),
              summary,
            ]),
            availability: availability ?? current.availability,
            loading: false,
            error: null,
            updatedAt,
          },
        },
      };
    }),
  setVisibleSummaryPane: (visibleSummaryPane) => set({ visibleSummaryPane }),
  setTerminal: (terminalPaneId, terminalReady) => set({ terminalPaneId, terminalReady }),
  setError: (error) => set({ error }),
  reset: () => set({ ...initialState, hydrated: true }),
}));

export function mutationsAllowed(): boolean {
  const state = useMobileStore.getState();
  return state.connection === "ready" && state.serverMutationsAllowed;
}

export function paneWorkSummaryKey(sessionId: string, paneId: number): string {
  return `${sessionId}:${paneId}`;
}

export function paneWorkSummariesSupported(): boolean {
  return useMobileStore.getState().negotiatedCapabilities.includes(PANE_WORK_SUMMARY_CAPABILITY);
}

function summarySessionId(key: string): string {
  return key.slice(0, key.lastIndexOf(":"));
}

function emptyPaneWorkSummaryCache(): PaneWorkSummaryCache {
  return {
    summaries: [],
    availability: "unknown",
    loading: false,
    error: null,
    updatedAt: null,
  };
}

function normalizePaneWorkSummaries(
  sessionId: string,
  paneId: number,
  summaries: PaneWorkSummary[],
): PaneWorkSummary[] {
  const byWindow = new Map<string, PaneWorkSummary>();
  for (const summary of summaries) {
    if (summary.session_id !== sessionId || summary.pane_id !== paneId) continue;
    byWindow.set(summary.window_start, summary);
  }
  return [...byWindow.values()].sort((left, right) => right.window_start.localeCompare(left.window_start));
}
