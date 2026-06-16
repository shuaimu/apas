import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useStore, type Message, type CliClient } from './store';

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
      messages: [],
      machines: [],
      projectGoals: {},
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

    function makeOpenWs() {
      const send = vi.fn();
      return {
        readyState: WebSocket.OPEN,
        send,
        close: vi.fn(),
      } as unknown as WebSocket & { send: typeof send };
    }

    beforeEach(() => {
      useStore.setState({
        sessionId: 'session-team-todo',
        approveTodo: initialStore.approveTodo,
        rejectTodo: initialStore.rejectTodo,
        addTodo: initialStore.addTodo,
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
  });
});
