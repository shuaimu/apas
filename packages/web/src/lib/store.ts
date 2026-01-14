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
}

export type OutputType =
  | { type: "text" }
  | { type: "code"; language?: string }
  | { type: "tool_use"; tool: string; input: unknown }
  | { type: "tool_result"; tool: string; success: boolean }
  | { type: "approval_request"; toolCallId: string; tool: string; description: string }
  | { type: "system" }
  | { type: "error" };

interface AppState {
  // Connection state
  connected: boolean;
  sessionId: string | null;
  ws: WebSocket | null;
  refreshInterval: NodeJS.Timeout | null;
  isAttached: boolean; // Whether we're attached to an active session

  // CLI clients
  cliClients: CliClient[];

  // Persisted sessions
  sessions: SessionInfo[];

  // Messages
  messages: Message[];

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
  startAutoRefresh: () => void;
  stopAutoRefresh: () => void;
}

const WS_URL = process.env.NEXT_PUBLIC_WS_URL || "ws://130.245.173.105:8081";

export const useStore = create<AppState>((set, get) => ({
  connected: false,
  sessionId: null,
  ws: null,
  refreshInterval: null,
  isAttached: false,
  cliClients: [],
  sessions: [],
  messages: [],

  connect: () => {
    const ws = new WebSocket(`${WS_URL}/ws/web`);

    ws.onopen = () => {
      console.log("WebSocket connected");
      // Dev mode: no authentication needed, server auto-authenticates
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        handleServerMessage(data, set, get);
      } catch (e) {
        console.error("Failed to parse message:", e);
      }
    };

    ws.onclose = () => {
      console.log("WebSocket disconnected");
      set({ connected: false, ws: null, cliClients: [] });
    };

    ws.onerror = (error) => {
      console.error("WebSocket error:", error);
    };

    set({ ws });
  },

  disconnect: () => {
    const { ws } = get();
    get().stopAutoRefresh();
    if (ws) {
      ws.close();
    }
    set({ connected: false, ws: null, sessionId: null, cliClients: [], isAttached: false });
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
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    // Clear previous messages and mark as attached
    set({ messages: [], isAttached: true });

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
    set({ sessionId, messages: [], isAttached: false });
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

  startAutoRefresh: () => {
    const { refreshInterval } = get();
    if (refreshInterval) return; // Already running

    const interval = setInterval(() => {
      const { connected, sessionId, isAttached, cliClients } = get();
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
}));

function handleServerMessage(
  data: Record<string, unknown>,
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState
) {
  switch (data.type) {
    case "authenticated":
      set({ connected: true });
      console.log("Authenticated as user:", data.user_id);
      // Request CLI clients and sessions list after authentication
      get().refreshCliClients();
      get().listSessions();
      // Start auto-refresh for real-time updates
      get().startAutoRefresh();
      break;

    case "authentication_failed":
      console.error("Authentication failed:", data.reason);
      set({ connected: false });
      break;

    case "cli_clients": {
      const clients = (data.clients as Array<Record<string, unknown>>) || [];
      set({
        cliClients: clients.map((c) => ({
          id: c.id as string,
          name: c.name as string | undefined,
          status: (c.status as "online" | "offline" | "busy") || "offline",
          lastSeen: c.last_seen as string | undefined,
          activeSession: c.active_session as string | undefined,
        })),
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

    case "output": {
      const outputType = parseOutputType(data.output_type as Record<string, unknown> | undefined);
      const message: Message = {
        id: generateId(),
        role: "assistant",
        content: data.content as string,
        timestamp: new Date(),
        outputType,
      };
      set((state) => ({ messages: [...state.messages, message] }));
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
        })),
      });
      break;
    }

    case "session_messages": {
      const messages = (data.messages as Array<Record<string, unknown>>) || [];
      set({
        sessionId: data.session_id as string,
        messages: messages.map((m) => ({
          id: m.id as string,
          role: m.role as "user" | "assistant" | "system",
          content: m.content as string,
          timestamp: new Date(m.created_at as string || Date.now()),
          outputType: { type: m.message_type as "text" | "system" || "text" } as OutputType,
        })),
      });
      break;
    }

    case "user_input": {
      // User input from CLI (displayed as user message)
      const userMessage: Message = {
        id: generateId(),
        role: "user",
        content: data.text as string,
        timestamp: new Date(),
        outputType: { type: "text" },
      };
      set((state) => ({ messages: [...state.messages, userMessage] }));
      break;
    }

    case "stream_message": {
      // Real-time Claude output from attached session
      const msg = data.message as Record<string, unknown>;
      if (!msg) break;

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
              set((state) => ({ messages: [...state.messages, assistantMessage] }));
            } else if (block.type === "tool_use") {
              const toolMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: `Using ${block.name}: ${JSON.stringify(block.input)}`,
                timestamp: new Date(),
                outputType: { type: "tool_use", tool: block.name as string, input: block.input },
              };
              set((state) => ({ messages: [...state.messages, toolMessage] }));
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
        set((state) => ({ messages: [...state.messages, resultMessage] }));
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
