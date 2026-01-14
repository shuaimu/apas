import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useStore, type Message, type CliClient } from './store';

describe('useStore', () => {
  beforeEach(() => {
    // Reset store state before each test
    useStore.setState({
      connected: false,
      sessionId: null,
      ws: null,
      cliClients: [],
      messages: [],
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
