import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { handleServerMessage, storeDebugLog, useStore, paneWatermarksToRecord, type Message, type CliClient, type TeamRecord, type SessionCacheEntry } from './store';

describe('useStore', () => {
  beforeEach(() => {
    // connect() bails when no token is in localStorage, so the WS-touching
    // tests below rely on having one pre-seeded.
    localStorage.setItem('apas_token', 'test-token');
    // Reset store state before each test
    useStore.setState({
      connected: false,
      sessionId: null,
      ws: null,
      cliClients: [],
      sessions: [],
      workingPanesBySession: new Map(),
      messages: [],
      machines: [],
      projectGoals: {},
      projectFlags: {},
      projectPolicies: {},
      toasts: [],
      showToast: useStore.getInitialState().showToast,
      teamRecords: [],
      teamRecordsBySession: new Map(),
    });
  });

  describe('storeDebugLog', () => {
    afterEach(() => {
      vi.unstubAllEnvs();
    });

    it('emits diagnostics outside production', () => {
      vi.stubEnv('NODE_ENV', 'development');
      const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {});

      storeDebugLog('store diagnostic', { connected: true });

      expect(consoleSpy).toHaveBeenCalledWith('store diagnostic', { connected: true });
      consoleSpy.mockRestore();
    });

    it('suppresses diagnostics in production', () => {
      vi.stubEnv('NODE_ENV', 'production');
      const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {});

      storeDebugLog('store diagnostic');

      expect(consoleSpy).not.toHaveBeenCalled();
      consoleSpy.mockRestore();
    });
  });

  describe('initial state', () => {
    it('should have correct initial values', () => {
      const state = useStore.getState();
      expect(state.connected).toBe(false);
      expect(state.sessionId).toBeNull();
      expect(state.ws).toBeNull();
      expect(state.cliClients).toEqual([]);
      expect(state.messages).toEqual([]);
    });
  });

  describe('session attachment confirmations', () => {
    it('does not let a background session disable the current project', () => {
      useStore.setState({ sessionId: 'current-session', isAttached: true });

      handleServerMessage({
        type: 'session_attached',
        session_id: 'cached-background-session',
        has_active_cli: false,
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().isAttached).toBe(true);

      handleServerMessage({
        type: 'session_attached',
        session_id: 'current-session',
        has_active_cli: false,
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().isAttached).toBe(false);
    });

    it('tracks working, idle, and offline independently for every session', () => {
      useStore.setState({
        sessionId: 'current-session',
        sessions: [
          { id: 'current-session', status: 'connected', isActive: true, isWorking: false },
          { id: 'background-session', status: 'connected', isActive: true, isWorking: false },
        ],
      });

      handleServerMessage({
        type: 'pane_status',
        session_id: 'background-session',
        pane_id: 7,
        pane_type: 'interactive',
        status: 'Working…',
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions.find((session) => session.id === 'background-session')?.isWorking).toBe(true);
      expect(useStore.getState().paneStatuses['7']).toBeUndefined();

      handleServerMessage({
        type: 'pane_status',
        session_id: 'background-session',
        pane_id: 7,
        pane_type: 'interactive',
        status: null,
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions.find((session) => session.id === 'background-session')?.isWorking).toBe(false);

      handleServerMessage({
        type: 'session_attached',
        session_id: 'background-session',
        has_active_cli: false,
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions.find((session) => session.id === 'background-session')).toMatchObject({
        isActive: false,
        isWorking: false,
      });
    });

    it('uses an explicit terminal completion as a redundant working-state clear', () => {
      useStore.setState({
        sessionId: 'current-session',
        sessions: [
          { id: 'current-session', status: 'connected', isActive: true, isWorking: true },
        ],
        workingPanesBySession: new Map([['current-session', new Set([7])]]),
        paneStatuses: { '7': 'Working...' },
      });

      handleServerMessage({
        type: 'stream_message',
        session_id: 'current-session',
        pane_id: 7,
        message: {
          type: 'assistant',
          message: { content: [] },
          extra: { terminal_turn_complete: false },
        },
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions[0]?.isWorking).toBe(true);

      handleServerMessage({
        type: 'stream_message',
        session_id: 'current-session',
        pane_id: 7,
        message: {
          type: 'assistant',
          message: { content: [] },
          extra: { terminal_turn_complete: true },
        },
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().sessions[0]?.isWorking).toBe(false);
      expect(useStore.getState().workingPanesBySession.has('current-session')).toBe(false);
      expect(useStore.getState().paneStatuses['7']).toBeNull();
    });
  });

  describe('cluster policy snapshots', () => {
    it('stores the versioned policy and noncompliant running pane ids', () => {
      handleServerMessage({
        type: 'project_policy_changed',
        session_id: 'policy-session',
        policy: {
          team_available: false,
          allowed_launch_profiles: ['agent:codex:official:default'],
          version: 12,
          project_suspended: false,
        },
        noncompliant_pane_ids: [4, 8],
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().projectPolicies['policy-session']).toEqual({
        teamAvailable: false,
        allowedLaunchProfiles: ['agent:codex:official:default'],
        version: 12,
        projectSuspended: false,
        noncompliantPaneIds: [4, 8],
      });
    });
  });

  describe('project access changes', () => {
    it('refreshes transferred roles and removes revoked project state', () => {
      const listSessions = vi.fn();
      useStore.setState({
        listSessions,
        sessionId: 'session-a',
        isAttached: true,
        messages: [{
          id: 'secret',
          role: 'assistant',
          content: 'project content',
          timestamp: new Date(),
        }],
        paneConfigs: [{ pane_id: 1 } as never],
        sessions: [
          {
            id: 'session-a',
            projectId: 'project-a',
            status: 'active',
            shareRole: 'owner',
          },
          {
            id: 'session-b',
            projectId: 'project-b',
            status: 'active',
            shareRole: 'owner',
          },
        ],
      });

      handleServerMessage({
        type: 'project_access_changed',
        project_id: 'project-a',
        change: 'transferred',
        role: 'user',
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions[0]).toMatchObject({
        shareRole: 'user',
        isShared: true,
      });

      handleServerMessage({
        type: 'project_access_changed',
        project_id: 'project-a',
        change: 'revoked',
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().sessions.map((session) => session.id)).toEqual(['session-b']);
      expect(useStore.getState().sessionId).toBeNull();
      expect(useStore.getState().isAttached).toBe(false);
      expect(useStore.getState().messages).toEqual([]);
      expect(useStore.getState().paneConfigs).toEqual([]);
      expect(listSessions).toHaveBeenCalledTimes(2);
    });
  });

  describe('addMessage', () => {
    it('should add a message to the messages array', () => {
      const message: Message = {
        id: '1',
        role: 'user',
        content: 'Hello',
        timestamp: new Date(),
      };

      useStore.getState().addMessage(message);

      const state = useStore.getState();
      expect(state.messages).toHaveLength(1);
      expect(state.messages[0]).toEqual(message);
    });

    it('should preserve existing messages when adding new ones', () => {
      const message1: Message = {
        id: '1',
        role: 'user',
        content: 'First',
        timestamp: new Date(),
      };
      const message2: Message = {
        id: '2',
        role: 'assistant',
        content: 'Second',
        timestamp: new Date(),
      };

      useStore.getState().addMessage(message1);
      useStore.getState().addMessage(message2);

      const state = useStore.getState();
      expect(state.messages).toHaveLength(2);
      expect(state.messages[0].content).toBe('First');
      expect(state.messages[1].content).toBe('Second');
    });
  });

  describe('clearMessages', () => {
    it('should clear all messages', () => {
      const message: Message = {
        id: '1',
        role: 'user',
        content: 'Hello',
        timestamp: new Date(),
      };

      useStore.getState().addMessage(message);
      expect(useStore.getState().messages).toHaveLength(1);

      useStore.getState().clearMessages();
      expect(useStore.getState().messages).toHaveLength(0);
    });
  });

  describe('connect', () => {
    it('should create a WebSocket connection', async () => {
      useStore.getState().connect();

      // Wait for async WebSocket connection
      await new Promise(resolve => setTimeout(resolve, 10));

      const state = useStore.getState();
      expect(state.ws).not.toBeNull();
    });
  });

  describe('disconnect', () => {
    it('should close WebSocket and reset state', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      useStore.getState().disconnect();

      const state = useStore.getState();
      expect(state.connected).toBe(false);
      expect(state.ws).toBeNull();
      expect(state.sessionId).toBeNull();
      expect(state.cliClients).toEqual([]);
    });
  });

  describe('startSession', () => {
    it('should clear messages when starting a new session', async () => {
      const message: Message = {
        id: '1',
        role: 'user',
        content: 'Hello',
        timestamp: new Date(),
      };
      useStore.getState().addMessage(message);

      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      useStore.getState().startSession();

      expect(useStore.getState().messages).toHaveLength(0);
    });

    it('should not send message if not connected', () => {
      const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      useStore.getState().startSession();

      expect(consoleSpy).toHaveBeenCalledWith('WebSocket not connected');
      consoleSpy.mockRestore();
    });
  });

  describe('attachSession', () => {
    it('should clear messages when attaching to session', async () => {
      const message: Message = {
        id: '1',
        role: 'user',
        content: 'Hello',
        timestamp: new Date(),
      };
      useStore.getState().addMessage(message);

      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      useStore.getState().attachSession('test-session-id');

      expect(useStore.getState().messages).toHaveLength(0);
    });
  });

  describe('loadSessionActivity', () => {
    it('requests a bounded all-pane timeline without resetting the attachment', () => {
      const send = vi.fn();
      const ws = {
        readyState: WebSocket.OPEN,
        send,
        close: vi.fn(),
      } as unknown as WebSocket;
      const existingMessage: Message = {
        id: 'existing-message',
        role: 'assistant',
        content: 'Already loaded',
        timestamp: new Date(),
      };

      useStore.setState({
        ws,
        sessionId: 'session-a',
        isAttached: true,
        messages: [existingMessage],
      });

      useStore.getState().loadSessionActivity('session-a');

      expect(send).toHaveBeenCalledWith(JSON.stringify({
        type: 'get_session_messages',
        session_id: 'session-a',
        limit: 30,
      }));
      expect(useStore.getState().sessionId).toBe('session-a');
      expect(useStore.getState().isAttached).toBe(true);
      expect(useStore.getState().messages).toEqual([existingMessage]);
    });
  });

  describe('sendMessage', () => {
    it('should add user message to messages array', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      useStore.getState().sendMessage('Hello there');

      const messages = useStore.getState().messages;
      expect(messages).toHaveLength(1);
      expect(messages[0].role).toBe('user');
      expect(messages[0].content).toBe('Hello there');
    });

    it('should not send if WebSocket is not connected', () => {
      const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      useStore.getState().sendMessage('Hello');

      expect(consoleSpy).toHaveBeenCalledWith('WebSocket not connected');
      consoleSpy.mockRestore();
    });
  });

  describe('approve and reject', () => {
    it('should send approve message', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      const ws = useStore.getState().ws;
      useStore.getState().approve('tool-call-123');

      expect(ws?.send).toHaveBeenCalled();
    });

    it('should send reject message', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      const ws = useStore.getState().ws;
      useStore.getState().reject('tool-call-123');

      expect(ws?.send).toHaveBeenCalled();
    });
  });

  describe('team todo approval and add', () => {
    const initialStore = useStore.getInitialState();

    function makeWs(readyState: number = WebSocket.OPEN) {
      const send = vi.fn();
      return {
        readyState,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    function makeOpenWs() {
      return makeWs();
    }

    beforeEach(() => {
      useStore.setState({
        sessionId: 'session-team-todo',
        approveTodo: initialStore.approveTodo,
        rejectTodo: initialStore.rejectTodo,
        addTodo: initialStore.addTodo,
        startTeam: initialStore.startTeam,
        showToast: initialStore.showToast,
      });
    });

    it('approveTodo sends a todo_approval approve request for the active session', () => {
      const ws = makeOpenWs();
      useStore.setState({ ws });

      useStore.getState().approveTodo('TODO-001');

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'todo_approval',
        session_id: 'session-team-todo',
        todo_id: 'TODO-001',
        action: 'approve',
      }));
    });

    it('rejectTodo sends a todo_approval reject request for the active session', () => {
      const ws = makeOpenWs();
      useStore.setState({ ws });

      useStore.getState().rejectTodo('TODO-002');

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'todo_approval',
        session_id: 'session-team-todo',
        todo_id: 'TODO-002',
        action: 'reject',
      }));
    });

    it('addTodo rejects an empty trimmed title with an error toast', () => {
      const ws = makeOpenWs();
      const showToast = vi.fn();
      useStore.setState({ ws, showToast });

      useStore.getState().addTodo('   ', 'ignored body');

      expect(showToast).toHaveBeenCalledWith("TODO title can't be empty", 'error');
      expect(ws.send).not.toHaveBeenCalled();
    });

    it('addTodo trims the title and sends body for a valid request', () => {
      const ws = makeOpenWs();
      useStore.setState({ ws });

      useStore.getState().addTodo('  Ship the workflow  ', 'acceptance\ncriteria');

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'add_todo',
        session_id: 'session-team-todo',
        title: 'Ship the workflow',
        body: 'acceptance\ncriteria',
      }));
    });

    it('startTeam sends exact retained role specs and omits null models', () => {
      const ws = makeOpenWs();
      useStore.setState({
        ws,
        sessionId: 'session-team',
        projectPolicies: {
          'session-team': {
            teamAvailable: true,
            allowedLaunchProfiles: [
              'agent:claude:official:claude-sonnet-4',
              'agent:codex:official:default',
              'agent:opencode:official:default',
              'agent:cursor-agent:official:default',
            ],
            version: 1,
            projectSuspended: false,
            noncompliantPaneIds: [],
          },
        },
      });

      useStore.getState().startTeam({
        manager: { provider: 'claude', model: 'claude-sonnet-4' },
        techLead: { provider: 'codex', model: null },
        reviewer: { provider: 'opencode', model: null },
        developer: { provider: 'cursor-agent', model: null },
      });

      expect(ws.send).toHaveBeenCalledOnce();
      const payload = JSON.parse(ws.send.mock.calls[0][0] as string);
      expect(payload).toEqual({
        type: 'start_team',
        session_id: 'session-team',
        manager: { provider: 'claude', model: 'claude-sonnet-4' },
        tech_lead: { provider: 'codex' },
        reviewer: { provider: 'opencode' },
        developer: { provider: 'cursor-agent' },
      });
      expect(payload.tech_lead).not.toHaveProperty('model');
    });

    it('startTeam rejects a retired role before sending', () => {
      const ws = makeOpenWs();
      const showToast = vi.fn();
      useStore.setState({ ws, showToast, sessionId: 'session-team' });

      useStore.getState().startTeam({
        manager: { provider: 'claude', model: null },
        techLead: { provider: 'claude', model: null },
        reviewer: { provider: 'minimax', model: 'MiniMax-M2.7' },
        developer: { provider: 'claude', model: null },
      });

      expect(ws.send).not.toHaveBeenCalled();
      expect(showToast).toHaveBeenCalledWith(
        expect.stringContaining('no longer supported'),
        'error',
      );
    });

    it('startTeam shows an error toast and does not send when disconnected', () => {
      const ws = makeWs(WebSocket.CLOSED);
      const showToast = vi.fn();
      useStore.setState({ ws, showToast });

      useStore.getState().startTeam({
        manager: { provider: 'claude', model: null },
        techLead: { provider: 'claude', model: null },
        reviewer: { provider: 'claude', model: null },
        developer: { provider: 'claude', model: null },
      });

      expect(ws.send).not.toHaveBeenCalled();
      expect(showToast).toHaveBeenCalledWith(expect.stringContaining('cannot start team'), 'error');
    });
  });

  describe('pane reboot', () => {
    function makeWs(readyState: number = WebSocket.OPEN) {
      const send = vi.fn();
      return {
        readyState,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    it('rebootPane sends a reboot_pane request for the active session and target pane', () => {
      const ws = makeWs();
      useStore.setState({
        sessionId: 'session-pane-reboot',
        ws,
        paneConfigs: [{
          pane_id: 42,
          provider: 'codex',
          mode: 'interactive',
          session_id: 'session-pane-reboot',
          is_paused: false,
        }],
        projectPolicies: {
          'session-pane-reboot': {
            teamAvailable: true,
            allowedLaunchProfiles: ['agent:codex:official:default'],
            version: 1,
            projectSuspended: false,
            noncompliantPaneIds: [],
          },
        },
      });

      useStore.getState().rebootPane(42);

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'reboot_pane',
        session_id: 'session-pane-reboot',
        pane_id: 42,
      }));
    });

    it('rebootCli sends only the full CLI reboot request over an open websocket', () => {
      const ws = makeWs();
      useStore.setState({
        sessionId: 'session-cli-reboot',
        ws,
        paneConfigs: [],
        projectPolicies: {
          'session-cli-reboot': {
            teamAvailable: true,
            allowedLaunchProfiles: [],
            version: 1,
            projectSuspended: false,
            noncompliantPaneIds: [],
          },
        },
      });

      useStore.getState().rebootCli();

      expect(ws.send).toHaveBeenCalledOnce();
      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'reboot_cli',
        session_id: 'session-cli-reboot',
      }));
    });

    it('rebootCli does not send when the websocket is closed', () => {
      const ws = makeWs(WebSocket.CLOSED);
      const showToast = vi.fn();
      useStore.setState({
        sessionId: 'session-cli-reboot',
        ws,
        paneConfigs: [],
        projectPolicies: {
          'session-cli-reboot': {
            teamAvailable: true,
            allowedLaunchProfiles: [],
            version: 1,
            projectSuspended: false,
            noncompliantPaneIds: [],
          },
        },
        showToast,
      });

      useStore.getState().rebootCli();

      expect(ws.send).not.toHaveBeenCalled();
      expect(showToast).toHaveBeenCalledWith(
        'Not connected — reboot the CLI manually on the project host',
        'error',
      );
    });
  });

  describe('server error visibility', () => {
    it('shows server errors as a toast even when the per-pane view hides global messages', () => {
      const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      handleServerMessage({
        type: 'error',
        message: 'This project CLI is too old to reboot from the web. Reboot the CLI manually.',
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().toasts.at(-1)).toMatchObject({
        kind: 'error',
        message: 'This project CLI is too old to reboot from the web. Reboot the CLI manually.',
      });
      expect(useStore.getState().messages.at(-1)).toMatchObject({
        role: 'system',
        content: 'This project CLI is too old to reboot from the web. Reboot the CLI manually.',
      });
      consoleSpy.mockRestore();
    });
  });

  describe('session download', () => {
    function makeWs(readyState: number = WebSocket.OPEN) {
      const send = vi.fn();
      return {
        readyState,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    it('downloadSession sends a download_session request for the active session', () => {
      const ws = makeWs();
      useStore.setState({ sessionId: 'session-download', ws });

      useStore.getState().downloadSession();

      expect(ws.send).toHaveBeenCalledOnce();
      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'download_session',
        session_id: 'session-download',
      }));
    });

    it('downloadSession does not send without an active session id', () => {
      const ws = makeWs();
      useStore.setState({ sessionId: null, ws });

      useStore.getState().downloadSession();

      expect(ws.send).not.toHaveBeenCalled();
    });

    it('downloadSession does not send when the websocket is closed', () => {
      const ws = makeWs(WebSocket.CLOSED);
      useStore.setState({ sessionId: 'session-download', ws });

      useStore.getState().downloadSession();

      expect(ws.send).not.toHaveBeenCalled();
    });
  });

  describe('project_goal_changed', () => {
    it('caches project goal content by session id from websocket messages', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      const ws = useStore.getState().ws as unknown as {
        onmessage?: (event: MessageEvent) => void;
      };
      ws.onmessage?.(new MessageEvent('message', {
        data: JSON.stringify({
          type: 'project_goal_changed',
          session_id: 'session-project-goal',
          content: 'line one\n\nline two\n',
        }),
      }));

      expect(useStore.getState().projectGoals['session-project-goal']).toBe(
        'line one\n\nline two\n',
      );
    });
  });

  describe('team_record', () => {
    function teamRecord(body: string): TeamRecord {
      return {
        ts: '2026-06-18T12:00:00Z',
        pane_id: 178,
        kind: 'status',
        tags: ['task:TODO-127'],
        body,
      };
    }

    async function connectForTeamRecords(activeSession: string) {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));
      useStore.setState({
        sessionId: activeSession,
        teamRecords: [],
        teamRecordsBySession: new Map(),
      });
      return useStore.getState().ws as unknown as {
        onmessage?: (event: MessageEvent) => void;
        send: ReturnType<typeof vi.fn>;
      };
    }

    function dispatchTeamRecord(
      ws: { onmessage?: (event: MessageEvent) => void },
      sessionId: string,
      record: TeamRecord,
    ) {
      ws.onmessage?.(new MessageEvent('message', {
        data: JSON.stringify({
          type: 'team_record',
          session_id: sessionId,
          record,
        }),
      }));
    }

    it('stores scratchpad records by session and switches the active view', async () => {
      const ws = await connectForTeamRecords('session-a');

      dispatchTeamRecord(ws, 'session-a', teamRecord('record from session A'));
      dispatchTeamRecord(ws, 'session-b', teamRecord('record from session B'));

      expect(useStore.getState().teamRecords.map((r) => r.body)).toEqual([
        'record from session A',
      ]);
      expect(useStore.getState().teamRecordsBySession.get('session-b')?.map((r) => r.body)).toEqual([
        'record from session B',
      ]);

      useStore.getState().attachSession('session-b');
      expect(useStore.getState().teamRecords.map((r) => r.body)).toEqual([
        'record from session B',
      ]);

      useStore.getState().attachSession('session-a');
      expect(useStore.getState().teamRecords.map((r) => r.body)).toEqual([
        'record from session A',
      ]);
    });
  });

  describe('project flags', () => {
    function makeOpenWs() {
      const send = vi.fn();
      return {
        readyState: WebSocket.OPEN,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    it('updateProjectFlags sends both booleans and optimistically caches the active session', () => {
      const ws = makeOpenWs();
      useStore.setState({
        ws,
        sessionId: 'session-flags-a',
        projectFlags: {
          'session-flags-b': {
            autoApproveTodos: false,
            autoMergePrs: true,
            teamEnabled: false,
            disallowedTabTypes: [],
          },
        },
      });

      useStore.getState().updateProjectFlags({
        autoApproveTodos: true,
        autoMergePrs: false,
        teamEnabled: true,
        disallowedTabTypes: [],
      });

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
        type: 'update_project_operations',
        session_id: 'session-flags-a',
        auto_approve_todos: true,
        auto_merge_prs: false,
      }));
      expect(useStore.getState().projectFlags).toEqual({
        'session-flags-a': {
          autoApproveTodos: true,
          autoMergePrs: false,
          teamEnabled: true,
          disallowedTabTypes: [],
        },
        'session-flags-b': {
          autoApproveTodos: false,
          autoMergePrs: true,
          teamEnabled: false,
          disallowedTabTypes: [],
        },
      });
    });

    it('caches project_flags_changed messages by session id without leaking between projects', async () => {
      useStore.getState().connect();
      await new Promise(resolve => setTimeout(resolve, 10));

      const ws = useStore.getState().ws as unknown as {
        onmessage?: (event: MessageEvent) => void;
      };
      ws.onmessage?.(new MessageEvent('message', {
        data: JSON.stringify({
          type: 'project_flags_changed',
          session_id: 'session-flags-a',
          auto_approve_todos: true,
          auto_merge_prs: false,
        }),
      }));
      ws.onmessage?.(new MessageEvent('message', {
        data: JSON.stringify({
          type: 'project_flags_changed',
          session_id: 'session-flags-b',
          auto_approve_todos: false,
          auto_merge_prs: true,
        }),
      }));

      expect(useStore.getState().projectFlags).toMatchObject({
        'session-flags-a': { autoApproveTodos: true, autoMergePrs: false },
        'session-flags-b': { autoApproveTodos: false, autoMergePrs: true },
      });
    });
  });
});

describe('OutputType parsing', () => {
  it('should handle different output types in messages', () => {
    const textMessage: Message = {
      id: '1',
      role: 'assistant',
      content: 'Hello',
      timestamp: new Date(),
      outputType: { type: 'text' },
    };

    const codeMessage: Message = {
      id: '2',
      role: 'assistant',
      content: 'const x = 1;',
      timestamp: new Date(),
      outputType: { type: 'code', language: 'typescript' },
    };

    const errorMessage: Message = {
      id: '3',
      role: 'system',
      content: 'Error occurred',
      timestamp: new Date(),
      outputType: { type: 'error' },
    };

    useStore.getState().addMessage(textMessage);
    useStore.getState().addMessage(codeMessage);
    useStore.getState().addMessage(errorMessage);

    const messages = useStore.getState().messages;
    expect(messages).toHaveLength(3);
    expect(messages[0].outputType?.type).toBe('text');
    expect(messages[1].outputType?.type).toBe('code');
    expect(messages[2].outputType?.type).toBe('error');
  });
});

describe('retired provider web guards', () => {
  it('blocks every pane relaunch and input action before sending a websocket message', () => {
    const send = vi.fn();
    const ws = {
      readyState: WebSocket.OPEN,
      send,
      close: vi.fn(),
    } as unknown as WebSocket;
    const showToast = vi.fn();
    useStore.setState({
      ws,
      showToast,
      isAttached: true,
      sessionId: 'retired-session',
      paneConfigs: [{
        pane_id: 91,
        provider: 'claude',
        model: 'MiniMax-M2.7',
        mode: 'deadloop',
        session_id: 'retired-session',
        is_paused: true,
      }],
      projectPolicies: {
        'retired-session': {
          teamAvailable: true,
          allowedLaunchProfiles: [
            'agent:claude:minimax:minimax-m2.7',
            'agent:claude:official:default',
          ],
          version: 3,
          projectSuspended: false,
          noncompliantPaneIds: [91],
        },
      },
    });

    expect(useStore.getState().addPane('glm', 'interactive', undefined, undefined, 'glm-5.1'))
      .toEqual(expect.objectContaining({ success: false }));
    expect(useStore.getState().sendMessageToPane('hello', 91).success).toBe(false);
    useStore.getState().resumePane(91);
    useStore.getState().rebootPane(91);
    useStore.getState().updatePaneModel(91, null, 'codex');
    useStore.getState().startBot(91);

    expect(send).not.toHaveBeenCalled();
    expect(showToast).toHaveBeenCalledTimes(4);
  });
});

describe('DeepSeek machine config', () => {
  function makeOpenWs() {
    return {
      readyState: WebSocket.OPEN,
      send: vi.fn(),
      close: vi.fn(),
    } as unknown as WebSocket;
  }

  it('optimistically updates machine config and sends the DeepSeek backend payload', () => {
    const ws = makeOpenWs();
    useStore.setState({
      ws,
      machines: [{
        machine: {
          machineId: 'machine-1',
          hostname: 'devbox',
          os: 'linux',
          arch: 'x86_64',
        },
        projects: [],
      }],
    });

    useStore.getState().setMachineDeepseekConfig('machine-1', ' sk-deepseek ', false);

    const machine = useStore.getState().machines[0].machine;
    expect(machine.deepseekBackend).toEqual({
      apiBaseUrl: 'https://api.deepseek.com/anthropic',
      apiKey: 'sk-deepseek',
      apiKeyConfigured: true,
    });
    expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
      type: 'set_machine_deepseek_config',
      machine_id: 'machine-1',
      api_base_url: 'https://api.deepseek.com/anthropic',
      api_key: 'sk-deepseek',
      clear_api_key: false,
    }));
  });

  it('normalizes DeepSeek machine backend and preserves a masked existing key', async () => {
    useStore.getState().connect();
    await new Promise(resolve => setTimeout(resolve, 10));
    const ws = useStore.getState().ws as unknown as { onmessage?: (event: MessageEvent) => void };

    useStore.setState({
      machines: [{
        machine: {
          machineId: 'machine-1',
          hostname: 'devbox',
          os: 'linux',
          arch: 'x86_64',
          deepseekBackend: {
            apiBaseUrl: 'https://api.deepseek.com/anthropic',
            apiKey: 'sk-existing',
            apiKeyConfigured: true,
          },
        },
        projects: [],
      }],
    });

    ws.onmessage?.(new MessageEvent('message', {
      data: JSON.stringify({
        type: 'machines',
        machines: [{
          machine: {
            machine_id: 'machine-1',
            hostname: 'devbox',
            os: 'linux',
            arch: 'x86_64',
            deepseek_backend: {
              api_base_url: 'https://api.deepseek.com/anthropic',
              api_key_configured: true,
            },
            minimax_backend: {
              api_key: 'must-not-survive',
              api_key_configured: true,
            },
            glm_backend: {
              api_key: 'must-not-survive',
              api_key_configured: true,
            },
          },
          projects: [{
            project_id: 'project-1',
            path: '/repo',
            is_running: true,
          }],
        }],
      }),
    }));

    const entry = useStore.getState().machines[0];
    expect(entry.machine.deepseekBackend).toEqual({
      apiBaseUrl: 'https://api.deepseek.com/anthropic',
      apiKey: 'sk-existing',
      apiKeyConfigured: true,
    });
    expect(entry.machine).not.toHaveProperty('minimaxBackend');
    expect(entry.machine).not.toHaveProperty('glmBackend');
  });

  it('ignores retired usage messages from a mixed-version server', async () => {
    useStore.getState().connect();
    await new Promise(resolve => setTimeout(resolve, 10));
    const ws = useStore.getState().ws as unknown as { onmessage?: (event: MessageEvent) => void };
    useStore.setState({ usageLimits: new Map() });

    ws.onmessage?.(new MessageEvent('message', {
      data: JSON.stringify({
        type: 'usage_limits',
        cli_client_id: 'legacy-cli',
        provider: 'glm',
        limits: { seven_day: { utilization: 0.75 } },
      }),
    }));

    expect(useStore.getState().usageLimits.has('legacy-cli')).toBe(false);
  });
});

describe('pane pause state sync', () => {
  async function connectAndDispatch(payload: Record<string, unknown>) {
    useStore.getState().connect();
    await new Promise(resolve => setTimeout(resolve, 10));
    useStore.setState({ isAuthenticated: true, sessionId: 'session-pause' });
    const ws = useStore.getState().ws as unknown as { onmessage?: (event: MessageEvent) => void };
    ws.onmessage?.(new MessageEvent('message', {
      data: JSON.stringify(payload),
    }));
  }

  beforeEach(() => {
    useStore.setState({
      connected: false,
      isAuthenticated: false,
      sessionId: 'session-pause',
      ws: null,
      pausedPanes: [],
      paneConfigs: [],
      paneModes: {},
      isDeadloopPaused: false,
    });
  });

  it('seeds pausedPanes from PaneList is_paused flags', async () => {
    await connectAndDispatch({
      type: 'pane_list',
      session_id: 'session-pause',
      panes: [
        {
          pane_id: 1,
          provider: 'claude',
          mode: 'deadloop',
          session_id: 'pane-1',
          is_paused: true,
        },
        {
          pane_id: 42,
          provider: 'codex',
          mode: 'deadloop',
          session_id: 'pane-42',
          is_paused: false,
        },
        {
          pane_id: 99,
          provider: 'claude',
          mode: 'interactive',
          session_id: 'pane-99',
          is_paused: true,
        },
      ],
    });

    const state = useStore.getState();
    expect(state.pausedPanes).toEqual([1, 99]);
    expect(state.isDeadloopPaused).toBe(true);
    expect(state.paneConfigs.map((pane) => pane.provider)).toEqual(['claude', 'codex', 'claude']);
  });

  it('adds and removes pane_paused events without duplicate ids', async () => {
    useStore.setState({ pausedPanes: [5], isDeadloopPaused: false });

    await connectAndDispatch({ type: 'pane_paused', pane_id: 7, is_paused: true });
    await connectAndDispatch({ type: 'pane_paused', pane_id: 7, is_paused: true });
    expect(useStore.getState().pausedPanes).toEqual([5, 7]);

    await connectAndDispatch({ type: 'pane_paused', pane_id: 7, is_paused: false });
    expect(useStore.getState().pausedPanes).toEqual([5]);

    await connectAndDispatch({ type: 'pane_paused', pane_id: 1, is_paused: true });
    expect(useStore.getState().pausedPanes).toEqual([5, 1]);
    expect(useStore.getState().isDeadloopPaused).toBe(true);

    await connectAndDispatch({ type: 'pane_paused', pane_id: 1, is_paused: false });
    expect(useStore.getState().pausedPanes).toEqual([5]);
    expect(useStore.getState().isDeadloopPaused).toBe(false);
  });
});

describe('suggested worker accept/dismiss', () => {
  const initialStore = useStore.getInitialState();

  function makeOpenWs() {
    return {
      readyState: WebSocket.OPEN,
      send: vi.fn(),
      close: vi.fn(),
    } as unknown as WebSocket;
  }

  beforeEach(() => {
    useStore.setState({
      sessionId: 'session-suggestions',
      ws: null,
      isAttached: true,
      acceptSuggestion: initialStore.acceptSuggestion,
      dismissSuggestion: initialStore.dismissSuggestion,
      addPane: initialStore.addPane,
      showToast: initialStore.showToast,
    });
  });

  it('acceptSuggestion spawns a managed pane with role metadata and dismisses it', () => {
    const addPane = vi.fn(() => ({ success: true }));
    const showToast = vi.fn();
    const ws = makeOpenWs();
    useStore.setState({ addPane, showToast, ws });

    useStore.getState().acceptSuggestion({
      id: 'SUG-001',
      label: 'Frontend Worker',
      role: 'developer',
      goal: 'Build the dashboard',
      backstory: 'React specialist',
      needs_worktree: true,
    });

    expect(addPane).toHaveBeenCalledWith(
      'claude',
      'interactive',
      'Frontend Worker',
      undefined,
      undefined,
      true,
      {
        role: 'developer',
        goal: 'Build the dashboard',
        backstory: 'React specialist',
      },
      true,
    );
    expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
      type: 'dismiss_suggestion',
      session_id: 'session-suggestions',
      suggestion_id: 'SUG-001',
    }));
    expect(showToast).toHaveBeenCalledWith(
      'Accepted Frontend Worker — added to the team',
      'info',
    );
  });

  it('dismissSuggestion sends the active session and suggestion id', () => {
    const ws = makeOpenWs();
    useStore.setState({ ws, sessionId: 'session-suggestions' });

    useStore.getState().dismissSuggestion('SUG-002');

    expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
      type: 'dismiss_suggestion',
      session_id: 'session-suggestions',
      suggestion_id: 'SUG-002',
    }));
  });

  describe('per-pane catchup watermarks (reload staleness fix)', () => {
    function makeOpenWs() {
      const send = vi.fn();
      return {
        readyState: WebSocket.OPEN,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    function emptyCacheEntry(): SessionCacheEntry {
      return {
        messages: [],
        paneMessages: {},
        paneHasMore: {},
        paneConfigs: [],
        paneModes: {},
        hasMoreMessages: false,
        isDualPane: true,
        answeredQuestions: new Map(),
        cachedAt: 0,
      };
    }

    it('paneWatermarksToRecord keys by pane id, skips the legacy -1 key, and drops empties', () => {
      expect(
        paneWatermarksToRecord(
          new Map([
            [205, 'ts-205'],
            [841, 'ts-841'],
            [-1, 'ts-legacy'],
          ]),
        ),
      ).toEqual({ '205': 'ts-205', '841': 'ts-841' });

      expect(paneWatermarksToRecord(undefined)).toBeUndefined();
      expect(paneWatermarksToRecord(new Map())).toBeUndefined();
      // A map that holds only the synthetic legacy key persists nothing.
      expect(paneWatermarksToRecord(new Map([[-1, 'ts']]))).toBeUndefined();
    });

    it('attaching a cached session fires a PER-PANE watermark catchup (not the diluted single cutoff)', () => {
      const ws = makeOpenWs();
      const target = 'session-appdev';
      // Simulate the post-reload state: a persisted snapshot for the
      // target session plus its per-pane watermarks reseeded from IDB.
      // Pane 841 is long-idle (old ts); pane 205 ("App Dev") is active.
      useStore.setState({
        ws,
        isAuthenticated: true,
        sessionId: 'session-current', // different -> isSameSession false
        cliClients: [],
        sessions: [],
        sessionCache: new Map([[target, emptyCacheEntry()]]),
        paneLastCreatedAt: new Map([
          [
            target,
            new Map([
              [205, '2026-07-04T18:40:00Z'],
              [841, '2026-06-27T03:32:00Z'],
            ]),
          ],
        ]),
      });

      useStore.getState().attachSession(target);

      // The catchup must ask per-pane so pane 205's fresh tail is fetched
      // independently of pane 841's stale watermark — never the
      // after_created_at single cutoff (which the dead pane would drag to
      // June 27 and the 500-row cap could then truncate before 205).
      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify({
          type: 'get_session_messages',
          session_id: target,
          pane_watermarks: {
            '205': '2026-07-04T18:40:00Z',
            '841': '2026-06-27T03:32:00Z',
          },
        }),
      );
      const sentAfterCutoff = ws.send.mock.calls.some((c: unknown[]) =>
        typeof c[0] === 'string' && c[0].includes('after_created_at'),
      );
      expect(sentAfterCutoff).toBe(false);
    });
  });
});
