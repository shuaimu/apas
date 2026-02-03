import { create } from "zustand";

// UUID generator with fallback for environments without crypto.randomUUID
function generateId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  // Fallback UUID v4 generator
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}

export interface Message {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: Date;
  outputType?: OutputType;
}

export interface CliClient {
  id: string;
  name?: string;
  status: "online" | "offline" | "busy";
  lastSeen?: string;
  activeSession?: string;
}

export interface SessionInfo {
  id: string;
  cliClientId?: string;
  workingDir?: string;
  hostname?: string;
  status: string;
  createdAt?: string;
  isShared?: boolean;
  ownerEmail?: string;
  isActive?: boolean;
}

export interface UsageLimitWindow {
  utilization: number; // 0.0 to 1.0+
  resetsAt?: string; // ISO 8601 timestamp
}

export interface UsageLimits {
  fiveHour?: UsageLimitWindow;
  sevenDay?: UsageLimitWindow;
  fetchedAt?: string;
}

export interface CliUsageLimits {
  cliClientId: string;
  limits: UsageLimits;
}

export type OutputType =
  | { type: "text" }
  | { type: "code"; language?: string }
  | { type: "tool_use"; tool: string; input: unknown }
  | { type: "tool_result"; tool: string; success: boolean }
  | { type: "approval_request"; toolCallId: string; tool: string; description: string }
  | { type: "system" }
  | { type: "error" };

export type PaneType = "deadloop" | "interactive";

interface AppState {
  // Auth state
  token: string | null;
  userId: string | null;
  isAuthenticated: boolean;

  // Connection state
  connected: boolean;
  sessionId: string | null;
  cliClientId: string | null; // Current CLI client ID for per-project settings
  ws: WebSocket | null;
  refreshInterval: NodeJS.Timeout | null;
  isAttached: boolean; // Whether we're attached to an active session
  reconnectAttempts: number;
  reconnectTimeout: NodeJS.Timeout | null;
  visibilityHandler: (() => void) | null;

  // CLI clients
  cliClients: CliClient[];

  // Persisted sessions
  sessions: SessionInfo[];

  // Messages (single pane mode)
  messages: Message[];
  hasMoreMessages: boolean; // Whether there are older messages to load
  isLoadingMore: boolean; // Prevent multiple simultaneous loads

  // Dual pane mode
  isDualPane: boolean;
  deadloopMessages: Message[];
  interactiveMessages: Message[];

  // Deadloop control
  isDeadloopPaused: boolean;

  // Pane status (for status bar)
  interactiveStatus: string | null;
  deadloopStatus: string | null;

  // Usage limits per CLI client
  usageLimits: Map<string, UsageLimits>;

  // Auth actions
  login: (token: string, userId: string) => void;
  logout: () => void;

  // Actions
  connect: () => void;
  disconnect: () => void;
  sendMessage: (text: string) => void;
  addMessage: (message: Message) => void;
  approve: (toolCallId: string) => void;
  reject: (toolCallId: string) => void;
  clearMessages: () => void;
  startSession: (cliClientId?: string) => void;
  attachSession: (sessionId: string) => void;
  refreshCliClients: () => void;
  listSessions: () => void;
  loadSessionMessages: (sessionId: string) => void;
  loadMoreMessages: () => void; // Load older messages
  prependMessages: (messages: Message[], hasMore: boolean) => void; // Prepend older messages
  sendMessageToPane: (text: string, pane: PaneType) => { success: boolean; error?: string }; // Send to specific pane
  addMessageToPane: (message: Message, pane: PaneType) => void; // Add message to specific pane
  startAutoRefresh: () => void;
  stopAutoRefresh: () => void;
  pauseDeadloop: () => void;
  resumeDeadloop: () => void;
  rebootCli: () => void;
  downloadSession: () => void;
}

const WS_URL = process.env.NEXT_PUBLIC_WS_URL || "ws://apas.mpaxos.com:8080";

export const useStore = create<AppState>((set, get) => ({
  // Auth state - initialize from localStorage if available
  token: typeof window !== 'undefined' ? localStorage.getItem("apas_token") : null,
  userId: typeof window !== 'undefined' ? localStorage.getItem("apas_user_id") : null,
  isAuthenticated: false,

  connected: false,
  // Restore sessionId from localStorage to persist across page refreshes
  sessionId: typeof window !== 'undefined' ? localStorage.getItem("apas_session_id") : null,
  // Restore cliClientId from localStorage for per-project settings
  cliClientId: typeof window !== 'undefined' ? localStorage.getItem("apas_cli_client_id") : null,
  ws: null,
  refreshInterval: null,
  isAttached: false,
  reconnectAttempts: 0,
  reconnectTimeout: null,
  visibilityHandler: null,
  cliClients: [],
  sessions: [],
  messages: [],
  hasMoreMessages: false,
  isLoadingMore: false,
  isDualPane: false,
  deadloopMessages: [],
  interactiveMessages: [],
  isDeadloopPaused: false,
  interactiveStatus: null,
  deadloopStatus: null,
  usageLimits: new Map(),

  login: (token: string, userId: string) => {
    localStorage.setItem("apas_token", token);
    localStorage.setItem("apas_user_id", userId);
    set({ token, userId, isAuthenticated: true });
  },

  logout: () => {
    localStorage.removeItem("apas_token");
    localStorage.removeItem("apas_user_id");
    localStorage.removeItem("apas_session_id");
    const { ws, reconnectTimeout, visibilityHandler } = get();

    // Clear reconnect timeout
    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
    }

    // Remove visibility handler
    if (visibilityHandler && typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', visibilityHandler);
    }

    if (ws) {
      ws.close(1000, "User logged out");
    }
    set({
      token: null,
      userId: null,
      isAuthenticated: false,
      connected: false,
      ws: null,
      sessionId: null,
      cliClients: [],
      sessions: [],
      reconnectAttempts: 0,
      reconnectTimeout: null,
      visibilityHandler: null,
    });
  },

  connect: () => {
    const token = typeof window !== 'undefined' ? localStorage.getItem("apas_token") : null;
    if (!token) {
      console.log("No token found, cannot connect");
      return;
    }

    // Clear any existing reconnect timeout
    const { reconnectTimeout, visibilityHandler } = get();
    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
      set({ reconnectTimeout: null });
    }

    const ws = new WebSocket(`${WS_URL}/ws/web`);

    ws.onopen = () => {
      console.log("WebSocket connected, sending authentication...");
      // Reset reconnect attempts on successful connection
      set({ reconnectAttempts: 0 });
      // Send token for authentication
      ws.send(JSON.stringify({ type: "authenticate", token }));
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        handleServerMessage(data, set, get);
      } catch (e) {
        console.error("Failed to parse message:", e);
      }
    };

    ws.onclose = (event) => {
      console.log("WebSocket disconnected", event.code, event.reason);
      set({ connected: false, ws: null, cliClients: [], isAttached: false });

      // Auto-reconnect with exponential backoff (unless intentionally disconnected)
      // Code 1000 = normal close (intentional), 1001 = going away
      if (event.code !== 1000) {
        const { reconnectAttempts } = get();
        const maxAttempts = 10;
        if (reconnectAttempts < maxAttempts) {
          // Exponential backoff: 1s, 2s, 4s, 8s, 16s, max 30s
          const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000);
          console.log(`Scheduling reconnect attempt ${reconnectAttempts + 1} in ${delay}ms`);
          const timeout = setTimeout(() => {
            console.log(`Reconnect attempt ${reconnectAttempts + 1}`);
            set({ reconnectAttempts: reconnectAttempts + 1 });
            get().connect();
          }, delay);
          set({ reconnectTimeout: timeout });
        } else {
          console.log("Max reconnect attempts reached");
        }
      }
    };

    ws.onerror = (error) => {
      console.error("WebSocket error:", error);
    };

    // Add visibility change listener for mobile (only once)
    if (!visibilityHandler && typeof document !== 'undefined') {
      const handler = () => {
        if (document.visibilityState === 'visible') {
          const { ws, connected, sessionId, isAttached } = get();
          console.log("App became visible, checking connection...", { connected, isAttached, sessionId });
          // If not connected or WebSocket is not open, reconnect
          if (!connected || !ws || ws.readyState !== WebSocket.OPEN) {
            console.log("Connection lost while in background, reconnecting...");
            // Reset reconnect attempts for immediate reconnect
            set({ reconnectAttempts: 0 });
            get().connect();
          } else {
            // Connection appears healthy, refresh data
            console.log("Connection appears healthy, refreshing data...");
            get().refreshCliClients();
            get().listSessions();

            // If we have a session but lost attachment, re-attach
            // This handles the case where connection stayed open but server-side state was lost
            if (sessionId && !isAttached) {
              console.log("Session exists but not attached, re-attaching...");
              get().attachSession(sessionId);
            }
          }
        }
      };
      document.addEventListener('visibilitychange', handler);
      set({ visibilityHandler: handler });
    }

    set({ ws });
  },

  disconnect: () => {
    const { ws, reconnectTimeout, visibilityHandler } = get();
    get().stopAutoRefresh();

    // Clear reconnect timeout
    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
    }

    // Remove visibility handler
    if (visibilityHandler && typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', visibilityHandler);
    }

    if (ws) {
      ws.close(1000, "User disconnected"); // 1000 = normal close, prevents auto-reconnect
    }
    set({
      connected: false,
      ws: null,
      sessionId: null,
      cliClients: [],
      isAttached: false,
      reconnectAttempts: 0,
      reconnectTimeout: null,
      visibilityHandler: null,
    });
  },

  startSession: (cliClientId?: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    // Clear previous messages
    set({ messages: [], sessionId: null });

    // Request new session
    ws.send(JSON.stringify({
      type: "start_session",
      cli_client_id: cliClientId || null
    }));
  },

  attachSession: (sessionId: string) => {
    const { ws, sessionId: currentSessionId, isDualPane } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    // Save to localStorage to persist across page refreshes
    localStorage.setItem("apas_session_id", sessionId);

    // Only reset state when switching to a different session
    const isSameSession = currentSessionId === sessionId;

    // Check if session has an active CLI client
    const { cliClients, sessions } = get();
    const hasActiveClient = cliClients.some(c => c.activeSession === sessionId);

    // Get cliClientId from session info or active client
    let newCliClientId: string | null = null;
    const sessionInfo = sessions.find(s => s.id === sessionId);
    if (sessionInfo?.cliClientId) {
      newCliClientId = sessionInfo.cliClientId;
    } else {
      // Try to get from active CLI client
      const activeClient = cliClients.find(c => c.activeSession === sessionId);
      if (activeClient) {
        newCliClientId = activeClient.id;
      }
    }

    // Save cliClientId to localStorage for per-project settings
    if (newCliClientId) {
      localStorage.setItem("apas_cli_client_id", newCliClientId);
    }

    if (!isSameSession) {
      set({
        sessionId,
        cliClientId: newCliClientId,
        messages: [],
        deadloopMessages: [],
        interactiveMessages: [],
        isDualPane: false,
        isAttached: hasActiveClient, // Only attached if session has active CLI
        isDeadloopPaused: false, // Reset pause state - server will send correct state
        interactiveStatus: null, // Reset status bar
        deadloopStatus: null, // Reset status bar
      });
    } else {
      // Re-attaching to same session - update attached state based on active client
      set({ isAttached: hasActiveClient, cliClientId: newCliClientId });
    }

    // Attach to existing session
    ws.send(JSON.stringify({
      type: "attach_session",
      session_id: sessionId
    }));
  },

  refreshCliClients: () => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({ type: "list_cli_clients" }));
  },

  listSessions: () => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({ type: "list_sessions" }));
  },

  loadSessionMessages: (sessionId: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    // Save to localStorage to persist across page refreshes
    localStorage.setItem("apas_session_id", sessionId);
    // Reset all message state including dual-pane arrays
    set({
      sessionId,
      messages: [],
      deadloopMessages: [],
      interactiveMessages: [],
      isDualPane: false,
      isAttached: false
    });
    ws.send(JSON.stringify({ type: "get_session_messages", session_id: sessionId }));
  },

  sendMessage: (text: string) => {
    const { ws, sessionId } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    // Add user message to UI
    const userMessage: Message = {
      id: generateId(),
      role: "user",
      content: text,
      timestamp: new Date(),
      outputType: { type: "text" },
    };
    set((state) => ({ messages: [...state.messages, userMessage] }));

    // Start session if not started
    if (!sessionId) {
      ws.send(JSON.stringify({ type: "start_session" }));
    }

    // Send input
    ws.send(JSON.stringify({ type: "input", text }));
  },

  addMessage: (message: Message) => {
    set((state) => ({ messages: [...state.messages, message] }));
  },

  approve: (toolCallId: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "approve", tool_call_id: toolCallId }));
    }
  },

  reject: (toolCallId: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reject", tool_call_id: toolCallId }));
    }
  },

  clearMessages: () => {
    set({ messages: [] });
  },

  loadMoreMessages: () => {
    const { ws, sessionId, messages, deadloopMessages, interactiveMessages, isDualPane, isLoadingMore, hasMoreMessages } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    if (!sessionId || isLoadingMore || !hasMoreMessages) {
      return;
    }

    // Find the oldest message across all arrays
    let oldestMessage: Message | undefined;
    const allMessages = isDualPane
      ? [...messages, ...deadloopMessages, ...interactiveMessages]
      : messages;

    if (allMessages.length === 0) {
      return;
    }

    // Sort by timestamp to find the oldest
    oldestMessage = allMessages.reduce((oldest, msg) =>
      msg.timestamp < oldest.timestamp ? msg : oldest
    );

    set({ isLoadingMore: true });

    ws.send(JSON.stringify({
      type: "get_session_messages",
      session_id: sessionId,
      limit: 50,
      before_id: oldestMessage.id
    }));
  },

  prependMessages: (newMessages: Message[], hasMore: boolean) => {
    set((state) => ({
      messages: [...newMessages, ...state.messages],
      hasMoreMessages: hasMore,
      isLoadingMore: false
    }));
  },

  sendMessageToPane: (text: string, pane: PaneType): { success: boolean; error?: string } => {
    const { ws, sessionId, isAttached } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return { success: false, error: "Not connected to server" };
    }

    if (!isAttached) {
      console.error("Session is not active");
      return { success: false, error: "Session is not active. Start the CLI to send messages." };
    }

    // Don't add message locally - the server will broadcast it back via user_input
    // This prevents duplicate display

    // Send to server with pane type
    ws.send(JSON.stringify({
      type: "input",
      text,
      pane_type: pane
    }));
    return { success: true };
  },

  addMessageToPane: (message: Message, pane: PaneType) => {
    if (pane === "deadloop") {
      set((state) => ({ deadloopMessages: [...state.deadloopMessages, message] }));
    } else {
      set((state) => ({ interactiveMessages: [...state.interactiveMessages, message] }));
    }
  },

  startAutoRefresh: () => {
    const { refreshInterval } = get();
    if (refreshInterval) return; // Already running

    const interval = setInterval(() => {
      const { ws, connected, sessionId, isAttached, cliClients } = get();

      // Check for zombie connection - WebSocket might think it's open but actually dead
      if (ws && ws.readyState !== WebSocket.OPEN) {
        console.log("WebSocket not in OPEN state, triggering reconnect...");
        set({ connected: false, ws: null, isAttached: false, reconnectAttempts: 0 });
        get().connect();
        return;
      }

      if (!connected) return;

      // Refresh CLI clients and sessions list
      get().refreshCliClients();
      get().listSessions();

      // If we're viewing a session but not attached, check if it became active
      if (sessionId && !isAttached) {
        const activeClient = cliClients.find(c => c.activeSession === sessionId);
        if (activeClient) {
          // Session is now active, attach to it for real-time updates
          get().attachSession(sessionId);
        }
      }
    }, 3000); // Refresh every 3 seconds

    set({ refreshInterval: interval });
  },

  stopAutoRefresh: () => {
    const { refreshInterval } = get();
    if (refreshInterval) {
      clearInterval(refreshInterval);
      set({ refreshInterval: null });
    }
  },

  pauseDeadloop: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "pause_deadloop" }));
    }
  },

  resumeDeadloop: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resume_deadloop" }));
    }
  },

  rebootCli: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reboot_cli" }));
    }
  },

  downloadSession: () => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN && sessionId) {
      ws.send(JSON.stringify({ type: "download_session", session_id: sessionId }));
    }
  },
}));

// Helper function to route messages to correct array based on pane type
function addMessageWithPaneRouting(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
  message: Message,
  paneType: string | undefined
) {
  let { isDualPane } = get();

  // Auto-detect dual pane mode when we receive a pane_type
  if (paneType && !isDualPane) {
    set({ isDualPane: true });
    isDualPane = true;
  }

  if (isDualPane && paneType) {
    // Dual pane mode - route to specific array
    if (paneType === "deadloop") {
      set((state) => ({ deadloopMessages: [...state.deadloopMessages, message] }));
    } else if (paneType === "interactive") {
      set((state) => ({ interactiveMessages: [...state.interactiveMessages, message] }));
    } else {
      // Unknown pane type, add to main messages
      set((state) => ({ messages: [...state.messages, message] }));
    }
  } else {
    // Single pane mode - add to main messages
    set((state) => ({ messages: [...state.messages, message] }));
  }
}

function handleServerMessage(
  data: Record<string, unknown>,
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState
) {
  switch (data.type) {
    case "authenticated":
      set({
        connected: true,
        isAuthenticated: true,
        userId: data.user_id as string,
      });
      console.log("Authenticated as user:", data.user_id);
      // Request CLI clients and sessions list after authentication
      get().refreshCliClients();
      get().listSessions();
      // Start auto-refresh for real-time updates
      get().startAutoRefresh();
      // Restore previously viewed session if available
      const savedSessionId = localStorage.getItem("apas_session_id");
      if (savedSessionId) {
        console.log("Restoring session:", savedSessionId);
        // Use setTimeout to ensure sessions list is loaded first
        setTimeout(() => {
          get().attachSession(savedSessionId);
        }, 500);
      }
      break;

    case "authentication_failed":
      console.error("Authentication failed:", data.reason);
      // Clear invalid token
      localStorage.removeItem("apas_token");
      localStorage.removeItem("apas_user_id");
      set({
        connected: false,
        isAuthenticated: false,
        token: null,
        userId: null,
      });
      break;

    case "cli_clients": {
      const clients = (data.clients as Array<Record<string, unknown>>) || [];
      const parsedClients = clients.map((c) => ({
        id: c.id as string,
        name: c.name as string | undefined,
        status: (c.status as "online" | "offline" | "busy") || "offline",
        lastSeen: c.last_seen as string | undefined,
        activeSession: c.active_session as string | undefined,
      }));

      // Update isAttached based on whether current session has an active client
      const { sessionId } = get();
      const hasActiveClient = sessionId
        ? parsedClients.some(c => c.activeSession === sessionId)
        : false;

      set({
        cliClients: parsedClients,
        isAttached: hasActiveClient,
      });
      break;
    }

    case "session_started":
      set({ sessionId: data.session_id as string });
      console.log("Session started:", data.session_id);
      break;

    case "session_status":
      console.log("Session status:", data.status);
      break;

    case "pane_status": {
      const paneType = data.pane_type as string | undefined;
      const status = data.status as string | null;
      if (paneType === "interactive") {
        set({ interactiveStatus: status });
      } else if (paneType === "deadloop") {
        set({ deadloopStatus: status });
      }
      break;
    }

    case "output": {
      const outputType = parseOutputType(data.output_type as Record<string, unknown> | undefined);
      const message: Message = {
        id: generateId(),
        role: "assistant",
        content: data.content as string,
        timestamp: new Date(),
        outputType,
      };
      const paneType = data.pane_type as string | undefined;
      addMessageWithPaneRouting(set, get, message, paneType);
      break;
    }

    case "error":
      console.error("Server error:", data.message);
      const errorMessage: Message = {
        id: generateId(),
        role: "system",
        content: data.message as string,
        timestamp: new Date(),
        outputType: { type: "error" },
      };
      set((state) => ({ messages: [...state.messages, errorMessage] }));
      break;

    case "sessions": {
      const sessions = (data.sessions as Array<Record<string, unknown>>) || [];
      set({
        sessions: sessions.map((s) => ({
          id: s.id as string,
          cliClientId: s.cli_client_id as string | undefined,
          workingDir: s.working_dir as string | undefined,
          hostname: s.hostname as string | undefined,
          status: s.status as string,
          createdAt: s.created_at as string | undefined,
          isShared: s.is_shared as boolean | undefined,
          ownerEmail: s.owner_email as string | undefined,
          isActive: s.is_active as boolean | undefined,
        })),
      });
      break;
    }

    case "session_messages": {
      const incomingSessionId = data.session_id as string;
      const messages = (data.messages as Array<Record<string, unknown>>) || [];
      const hasMore = data.has_more as boolean || false;

      const { sessionId: currentSessionId, isLoadingMore } = get();

      // Check if any messages have pane_type - if so, enable dual pane
      const hasPaneType = messages.some((m) => m.pane_type);
      if (hasPaneType) {
        set({ isDualPane: true });
      }

      const parsedMessages: Message[] = messages.map((m) => {
        const messageType = m.message_type as string || "text";
        const content = m.content as string;
        let outputType: OutputType;
        let displayContent = content;

        // Reconstruct outputType based on message_type
        if (messageType === "tool_use") {
          try {
            const toolData = JSON.parse(content);
            outputType = {
              type: "tool_use",
              tool: toolData.name as string,
              input: toolData.input,
            };
            displayContent = `Using ${toolData.name}: ${JSON.stringify(toolData.input)}`;
          } catch {
            outputType = { type: "text" };
          }
        } else if (messageType === "tool_result") {
          try {
            const resultData = JSON.parse(content);
            outputType = {
              type: "tool_result",
              tool: resultData.tool_use_id as string,
              success: !resultData.is_error,
            };
            displayContent = resultData.content as string || content;
          } catch {
            outputType = { type: "text" };
          }
        } else if (messageType === "result" || messageType === "system") {
          outputType = { type: "system" };
        } else {
          outputType = { type: "text" };
        }

        return {
          id: m.id as string,
          role: m.role as "user" | "assistant" | "system",
          content: displayContent,
          timestamp: new Date(m.created_at as string || Date.now()),
          outputType,
        };
      });

      // Route messages to correct panes
      const { isDualPane } = get();
      const deadloopMsgs: Message[] = [];
      const interactiveMsgs: Message[] = [];
      const mainMsgs: Message[] = [];

      messages.forEach((m, i) => {
        const paneType = m.pane_type as string | undefined;
        const msg = parsedMessages[i];
        if (paneType === "deadloop") {
          deadloopMsgs.push(msg);
        } else if (paneType === "interactive") {
          interactiveMsgs.push(msg);
        } else {
          mainMsgs.push(msg);
        }
      });

      if (isLoadingMore) {
        // Prepend older messages
        if (isDualPane || hasPaneType) {
          set((state) => ({
            messages: [...mainMsgs, ...state.messages],
            deadloopMessages: [...deadloopMsgs, ...state.deadloopMessages],
            interactiveMessages: [...interactiveMsgs, ...state.interactiveMessages],
            hasMoreMessages: hasMore,
            isLoadingMore: false,
          }));
        } else {
          get().prependMessages(parsedMessages, hasMore);
        }
      } else if (isDualPane || hasPaneType) {
        // Initial load - dual pane mode
        // Preserve any real-time messages that arrived before this response (race condition fix)
        const {
          interactiveMessages: existingInteractive,
          deadloopMessages: existingDeadloop,
          messages: existingMain
        } = get();
        set({
          sessionId: data.session_id as string,
          messages: [...mainMsgs, ...existingMain],
          deadloopMessages: [...deadloopMsgs, ...existingDeadloop],
          interactiveMessages: [...interactiveMsgs, ...existingInteractive],
          hasMoreMessages: hasMore,
          isDualPane: true,
        });
      } else {
        // Initial load - single pane mode
        set({
          sessionId: data.session_id as string,
          messages: parsedMessages,
          hasMoreMessages: hasMore,
        });
      }
      break;
    }

    case "user_input": {
      // User input from CLI (displayed as user message)
      // Only show if it's for the currently viewed session
      const msgSessionId = data.session_id as string | undefined;
      const { sessionId: currentSessionId } = get();
      if (msgSessionId && currentSessionId && msgSessionId !== currentSessionId) {
        break; // Ignore messages from other sessions
      }

      const userMessage: Message = {
        id: generateId(),
        role: "user",
        content: data.text as string,
        timestamp: new Date(),
        outputType: { type: "text" },
      };
      const paneType = data.pane_type as string | undefined;
      addMessageWithPaneRouting(set, get, userMessage, paneType);
      break;
    }

    case "stream_message": {
      // Real-time Claude output from attached session
      // Only show if it's for the currently viewed session
      const msgSessionId = data.session_id as string | undefined;
      const { sessionId: currentSessionId } = get();
      if (msgSessionId && currentSessionId && msgSessionId !== currentSessionId) {
        break; // Ignore messages from other sessions
      }

      const msg = data.message as Record<string, unknown>;
      if (!msg) break;

      const paneType = data.pane_type as string | undefined;
      const msgType = msg.type as string;
      if (msgType === "assistant") {
        const message = msg.message as Record<string, unknown>;
        const content = message?.content as Array<Record<string, unknown>>;
        if (content) {
          for (const block of content) {
            if (block.type === "text") {
              const assistantMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: block.text as string,
                timestamp: new Date(),
                outputType: { type: "text" },
              };
              addMessageWithPaneRouting(set, get, assistantMessage, paneType);
            } else if (block.type === "tool_use") {
              const toolMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: `Using ${block.name}: ${JSON.stringify(block.input)}`,
                timestamp: new Date(),
                outputType: { type: "tool_use", tool: block.name as string, input: block.input },
              };
              addMessageWithPaneRouting(set, get, toolMessage, paneType);
            }
          }
        }
      } else if (msgType === "result") {
        const resultMessage: Message = {
          id: generateId(),
          role: "system",
          content: `${msg.subtype} - Cost: $${(msg.total_cost_usd as number || 0).toFixed(4)}, Duration: ${msg.duration_ms}ms`,
          timestamp: new Date(),
          outputType: { type: "system" },
        };
        addMessageWithPaneRouting(set, get, resultMessage, paneType);
      }
      break;
    }

    case "deadloop_status": {
      const isPaused = data.is_paused as boolean;
      console.log("Deadloop status update:", isPaused ? "paused" : "running");
      set({ isDeadloopPaused: isPaused });
      break;
    }

    case "session_download": {
      // Handle session download - create a downloadable file
      const sessionId = data.session_id as string;
      const messages = data.messages as Array<Record<string, unknown>> || [];
      const workingDir = data.working_dir as string | undefined;
      const hostname = data.hostname as string | undefined;
      const createdAt = data.created_at as string | undefined;

      const downloadData = {
        session_id: sessionId,
        working_dir: workingDir,
        hostname: hostname,
        created_at: createdAt,
        exported_at: new Date().toISOString(),
        message_count: messages.length,
        messages: messages,
      };

      // Create and trigger download
      const blob = new Blob([JSON.stringify(downloadData, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `apas-session-${sessionId.slice(0, 8)}-${new Date().toISOString().slice(0, 10)}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);

      console.log(`Downloaded session ${sessionId} with ${messages.length} messages`);
      break;
    }

    case "usage_limits": {
      const cliClientId = data.cli_client_id as string;
      const limits = data.limits as Record<string, unknown>;
      if (cliClientId && limits) {
        const parsedLimits: UsageLimits = {
          fiveHour: limits.five_hour ? {
            utilization: (limits.five_hour as Record<string, unknown>).utilization as number,
            resetsAt: (limits.five_hour as Record<string, unknown>).resets_at as string | undefined,
          } : undefined,
          sevenDay: limits.seven_day ? {
            utilization: (limits.seven_day as Record<string, unknown>).utilization as number,
            resetsAt: (limits.seven_day as Record<string, unknown>).resets_at as string | undefined,
          } : undefined,
          fetchedAt: limits.fetched_at as string | undefined,
        };
        set((state) => {
          const newMap = new Map(state.usageLimits);
          newMap.set(cliClientId, parsedLimits);
          return { usageLimits: newMap };
        });
        console.log("Usage limits updated for CLI:", cliClientId, parsedLimits);
      }
      break;
    }

    default:
      console.log("Unknown message type:", data.type);
  }
}

function parseOutputType(data: Record<string, unknown> | undefined): OutputType {
  if (!data) return { type: "text" };

  switch (data.type || Object.keys(data)[0]) {
    case "text":
      return { type: "text" };
    case "code":
      return { type: "code", language: data.language as string | undefined };
    case "tool_use":
      return { type: "tool_use", tool: data.tool as string, input: data.input };
    case "tool_result":
      return { type: "tool_result", tool: data.tool as string, success: data.success as boolean };
    case "approval_request":
      return {
        type: "approval_request",
        toolCallId: data.tool_call_id as string,
        tool: data.tool as string,
        description: data.description as string,
      };
    case "system":
      return { type: "system" };
    case "error":
      return { type: "error" };
    default:
      return { type: "text" };
  }
}
