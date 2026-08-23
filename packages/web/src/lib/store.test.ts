import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { handleServerMessage, launchProfileKey, storeDebugLog, useStore, paneWatermarksToRecord, type Message, type CliClient, type TeamRecord, type SessionCacheEntry } from './store';

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
      cliLifecycleInventories: {},
      cliLifecycleOperations: {},
      cliLifecycleLatestBySession: {},
      sessions: [],
      workingPanesBySession: new Map(),
      paneStatuses: {},
      messages: [],
      machines: [],
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

    it('records a working-to-idle transition without refreshing it on idle replay', () => {
      vi.useFakeTimers();
      try {
        vi.setSystemTime(new Date('2026-08-20T12:00:00Z'));
        useStore.setState({
          sessionId: 'current-session',
          sessions: [{
            id: 'current-session',
            status: 'connected',
            isActive: true,
            isWorking: true,
            panes: [{
              pane_id: 7,
              kind: 'terminal',
              provider: 'claude',
              is_working: true,
            }],
          }],
          workingPanesBySession: new Map([['current-session', new Set([7])]]),
        });

        handleServerMessage({
          type: 'pane_status',
          session_id: 'current-session',
          pane_id: 7,
          pane_type: 'interactive',
          status: null,
        }, useStore.setState, useStore.getState);
        expect(useStore.getState().sessions[0]?.panes?.[0]?.idle_since)
          .toBe('2026-08-20T12:00:00.000Z');

        vi.setSystemTime(new Date('2026-08-20T13:00:00Z'));
        handleServerMessage({
          type: 'pane_status',
          session_id: 'current-session',
          pane_id: 7,
          pane_type: 'interactive',
          status: null,
        }, useStore.setState, useStore.getState);
        expect(useStore.getState().sessions[0]?.panes?.[0]?.idle_since)
          .toBe('2026-08-20T12:00:00.000Z');

        handleServerMessage({
          type: 'pane_status',
          session_id: 'current-session',
          pane_id: 7,
          pane_type: 'interactive',
          status: 'Working…',
        }, useStore.setState, useStore.getState);
        expect(useStore.getState().sessions[0]?.panes?.[0]?.idle_since).toBeUndefined();
      } finally {
        vi.useRealTimers();
      }
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

    it('clears stale foreground pane pills from an authoritative idle snapshot', () => {
      useStore.setState({
        sessionId: 'current-session',
        sessions: [
          { id: 'current-session', status: 'connected', isActive: true, isWorking: true },
        ],
        workingPanesBySession: new Map([['current-session', new Set([7, 8])]]),
        paneStatuses: { '7': 'Working...', '8': 'Still working...' },
      });

      handleServerMessage({
        type: 'sessions',
        sessions: [{
          id: 'current-session',
          status: 'connected',
          is_active: true,
          is_working: true,
          panes: [
            { pane_id: 7, kind: 'terminal', provider: 'codex', is_working: false },
            { pane_id: 8, kind: 'terminal', provider: 'codex', is_working: true },
          ],
        }],
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().paneStatuses).toEqual({ '8': 'Still working...' });

      handleServerMessage({
        type: 'sessions',
        sessions: [{
          id: 'current-session',
          status: 'connected',
          is_active: true,
          is_working: false,
          panes: [
            { pane_id: 7, kind: 'terminal', provider: 'codex', is_working: false },
            { pane_id: 8, kind: 'terminal', provider: 'codex', is_working: false },
          ],
        }],
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().paneStatuses).toEqual({});
    });

    it('clears stale foreground pane pills before reattaching the same session', () => {
      const send = vi.fn();
      useStore.setState({
        ws: { readyState: WebSocket.OPEN, send } as unknown as WebSocket,
        sessionId: 'current-session',
        sessions: [
          { id: 'current-session', status: 'connected', isActive: true, isWorking: false },
        ],
        paneStatuses: { '7': 'Working...' },
        interactiveStatus: 'Working...',
        deadloopStatus: 'Working...',
      });

      useStore.getState().attachSession('current-session');

      expect(useStore.getState().paneStatuses).toEqual({});
      expect(useStore.getState().interactiveStatus).toBeNull();
      expect(useStore.getState().deadloopStatus).toBeNull();
      expect(send).toHaveBeenCalledWith(JSON.stringify({
        type: 'attach_session',
        session_id: 'current-session',
      }));
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

    it('does not announce noncompliant panes on entering a project', () => {
      // This fired on every entry, named pane numbers, asserted they could not
      // be relaunched — which is no longer true — and offered nothing the
      // person reading it could act on.
      const showToast = vi.fn();
      useStore.setState({ showToast });

      handleServerMessage({
        type: 'project_policy_changed',
        session_id: 'policy-session',
        policy: {
          team_available: true,
          allowed_launch_profiles: ['agent:codex:official:default'],
          version: 12,
          project_suspended: false,
        },
        noncompliant_pane_ids: [4, 8],
      }, useStore.setState, useStore.getState);

      expect(showToast).not.toHaveBeenCalled();
    });

    it('still announces a suspended project', () => {
      const showToast = vi.fn();
      useStore.setState({ showToast });

      handleServerMessage({
        type: 'project_policy_changed',
        session_id: 'policy-session',
        policy: {
          team_available: true,
          allowed_launch_profiles: [],
          version: 13,
          project_suspended: true,
        },
        noncompliant_pane_ids: [4],
      }, useStore.setState, useStore.getState);

      expect(showToast).toHaveBeenCalledWith(
        'This project is suspended by a cluster administrator',
        'error',
      );
    });

    it('relaunches an existing pane whose profile is no longer allowed', () => {
      const send = vi.fn();
      const showToast = vi.fn();
      useStore.setState({
        ws: { readyState: WebSocket.OPEN, send, close: vi.fn() } as unknown as WebSocket,
        sessionId: 'policy-session',
        showToast,
        paneConfigs: [{
          pane_id: 42,
          provider: 'claude',
          kind: 'terminal',
          mode: 'interactive',
          session_id: 'policy-session',
          is_paused: false,
        }] as never,
        projectPolicies: {
          'policy-session': {
            teamAvailable: true,
            // Allows nothing pane 42 could be.
            allowedLaunchProfiles: ['agent:codex:official:default'],
            version: 12,
            projectSuspended: false,
            noncompliantPaneIds: [42],
          },
        },
      });

      useStore.getState().resumePane(42);
      useStore.getState().rebootPane(42);
      useStore.getState().startBot(42);

      expect(showToast).not.toHaveBeenCalled();
      expect(send).toHaveBeenCalledTimes(3);

      // Creating that same combination is still refused.
      expect(useStore.getState().addPane('claude', 'interactive'))
        .toEqual(expect.objectContaining({ success: false }));
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


    it('uses correlated reboot with the advertised preservation inventory', () => {
      const ws = makeWs();
      useStore.setState({
        sessionId: 'session-lifecycle-reboot',
        ws,
        paneConfigs: [],
        projectPolicies: {
          'session-lifecycle-reboot': {
            teamAvailable: true,
            allowedLaunchProfiles: [],
            version: 1,
            projectSuspended: false,
            noncompliantPaneIds: [],
          },
        },
        cliLifecycleInventories: {
          'session-lifecycle-reboot': {
            persistent_terminal_hosting: true,
            panes: [{ pane_id: 1, mode: 'live_adoptable' }],
          },
        },
      });

      const requestId = useStore.getState().rebootCli();
      expect(JSON.parse(ws.send.mock.calls[0][0] as string)).toEqual({
        type: 'cli_lifecycle_request',
        session_id: 'session-lifecycle-reboot',
        request_id: requestId,
        operation: 'reboot_cli',
      });
    });


    it('tracks server success and authorization failure by request ID', () => {
      const showToast = vi.fn();
      useStore.setState({ showToast });
      handleServerMessage({
        type: 'cli_lifecycle_status',
        session_id: 'session-a',
        request_id: 'request-success',
        operation: 'reboot_cli',
        phase: 'succeeded',
        message: 'Transport restored',
      }, useStore.setState, useStore.getState);
      handleServerMessage({
        type: 'cli_lifecycle_status',
        session_id: 'session-b',
        request_id: 'request-failed',
        operation: 'reboot_cli',
        phase: 'failed',
        message: 'Project access is no longer authorized',
      }, useStore.setState, useStore.getState);

      expect(useStore.getState().cliLifecycleOperations['request-success'].phase).toBe('succeeded');
      expect(useStore.getState().cliLifecycleOperations['request-failed'].phase).toBe('failed');
      expect(showToast).toHaveBeenCalledWith('Project access is no longer authorized', 'error');
    });

    it('times out an unfinished lifecycle operation with recovery guidance', () => {
      vi.useFakeTimers();
      try {
        const ws = makeWs();
        const showToast = vi.fn();
        useStore.setState({
          sessionId: 'session-timeout',
          ws,
          showToast,
          paneConfigs: [],
          projectPolicies: {
            'session-timeout': {
              teamAvailable: true,
              allowedLaunchProfiles: [],
              version: 1,
              projectSuspended: false,
              noncompliantPaneIds: [],
            },
          },
          cliLifecycleInventories: {
            'session-timeout': {
              persistent_terminal_hosting: false,
              panes: [],
            },
          },
        });
        const requestId = useStore.getState().rebootCli();

        vi.advanceTimersByTime(185_000);

        expect(useStore.getState().cliLifecycleOperations[requestId!]).toMatchObject({
          phase: 'timed_out',
          message: expect.stringContaining('project host'),
        });
      } finally {
        vi.useRealTimers();
      }
    });

    it('keeps pending progress across project navigation and removes it on access revocation', () => {
      const ws = makeWs();
      useStore.setState({
        sessionId: 'session-a',
        ws,
        sessions: [
          { id: 'session-a', projectId: 'project-a', status: 'connected' },
          { id: 'session-b', projectId: 'project-b', status: 'connected' },
        ],
        cliLifecycleOperations: {
          'request-a': {
            sessionId: 'session-a',
            requestId: 'request-a',
            operation: 'reboot_cli',
            phase: 'preparing',
            startedAt: 1,
            updatedAt: 2,
          },
        },
        cliLifecycleLatestBySession: { 'session-a': 'request-a' },
      });

      useStore.getState().attachSession('session-b');
      expect(useStore.getState().cliLifecycleOperations['request-a'].phase).toBe('preparing');

      handleServerMessage({
        type: 'project_access_changed',
        project_id: 'project-a',
        change: 'revoked',
      }, useStore.setState, useStore.getState);
      expect(useStore.getState().cliLifecycleOperations['request-a']).toBeUndefined();
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

describe('DeepSeek launch profiles', () => {
  it('keeps direct-provider default and canonical variants aligned with Rust policy keys', () => {
    expect(launchProfileKey('agent', 'deepseek', null))
      .toBe('agent:claude:deepseek:deepseek-v4-pro');
    expect(launchProfileKey('agent', 'claude', 'DeepSeek-V4-Flash'))
      .toBe('agent:claude:deepseek:deepseek-v4-flash');
    expect(launchProfileKey('agent', 'claude', 'deepseek-chat'))
      .toBe('agent:claude:deepseek:deepseek-chat');
  });

  it('derives terminal DeepSeek profiles from the claude frontend', () => {
    expect(launchProfileKey('terminal', 'claude', 'deepseek-v4-pro'))
      .toBe('terminal:claude:deepseek:deepseek-v4-pro');
    expect(launchProfileKey('terminal', 'claude', 'DeepSeek-V4-Flash'))
      .toBe('terminal:claude:deepseek:deepseek-v4-flash');
    expect(launchProfileKey('terminal', 'claude', undefined))
      .toBe('terminal:claude:official:default');
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

  it('preserves explicit usage-limited availability and reset metadata', () => {
    useStore.setState({ usageLimits: new Map() });

    handleServerMessage({
      type: 'usage_limits',
      cli_client_id: 'cli-1',
      provider: 'claude',
      limits: {
        seven_day: { utilization: 1, resets_at: '2026-08-23T13:00:00Z' },
        usage_limited: { window: 'weekly', resets_at: '2026-08-23T13:00:00Z' },
      },
    }, useStore.setState, useStore.getState);

    expect(useStore.getState().usageLimits.get('cli-1')?.claude?.usageLimited).toEqual({
      window: 'weekly',
      resetsAt: '2026-08-23T13:00:00Z',
    });
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
