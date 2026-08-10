import { create } from "zustand";
import { deriveAttention, type CodeEvent, type MobileBootstrapResponse, type MobileFeatureFlags, type MobileLaunchTarget, type MobileSessionSummary, type PaneConfig } from "@apas/protocol";

export type ConnectionPhase = "offline" | "connecting" | "authenticating" | "synchronizing" | "ready";

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
  terminalReady: boolean;
  terminalPaneId: number | null;
  serverMutationsAllowed: boolean;
  error: string | null;
  setHydrated: (signedIn: boolean) => void;
  setConnection: (connection: ConnectionPhase) => void;
  setServerMutationsAllowed: (allowed: boolean) => void;
  applyBootstrap: (bootstrap: MobileBootstrapResponse) => void;
  setCachedSessions: (sessions: MobileSessionSummary[], updatedAt: string | null) => void;
  setActiveSession: (sessionId: string | null) => void;
  markSessionUserInput: (sessionId: string, createdAt?: string) => void;
  setEvents: (sessionId: string, events: CodeEvent[]) => void;
  setPanes: (sessionId: string, panes: PaneConfig[]) => void;
  setPaneStatus: (sessionId: string, paneId: number, status: string | null | undefined) => void;
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
        paneStatusesBySession: {
          ...state.paneStatusesBySession,
          [sessionId]: sessionStatuses,
        },
      };
    }),
  setTerminal: (terminalPaneId, terminalReady) => set({ terminalPaneId, terminalReady }),
  setError: (error) => set({ error }),
  reset: () => set({ ...initialState, hydrated: true }),
}));

export function mutationsAllowed(): boolean {
  const state = useMobileStore.getState();
  return state.connection === "ready" && state.serverMutationsAllowed;
}
