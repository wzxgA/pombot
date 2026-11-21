import { randomUUID } from 'expo-crypto';

import type {
  ChatDeltaPayload as ChatDelta,
  ChatDonePayload as ChatDone,
  ChatHistoryResultPayload as ChatHistoryResult,
} from '../generated/contract';
import type { PairedDevice } from './identity';
import { createEnvelope, parseEnvelope, websocketUrl } from './protocol';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  createdAt: number;
}

export type ConnectionStatus = 'connecting' | 'connected' | 'offline';

interface ChatCallbacks {
  onStatus: (status: ConnectionStatus) => void;
  onMessages: (messages: ChatMessage[]) => void;
  onSending: (sending: boolean) => void;
  onError: (message: string) => void;
}

export class ChatConnection {
  private socket: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private retries = 0;
  private disposed = false;
  private connected = false;
  private messages: ChatMessage[] = [];

  constructor(
    private readonly device: PairedDevice,
    private readonly callbacks: ChatCallbacks,
  ) {}

  connect() {
    if (this.disposed) return;
    this.callbacks.onStatus('connecting');
    const socket = new WebSocket(websocketUrl(this.device.serverUrl));
    this.socket = socket;
    socket.onopen = () => {
      socket.send(JSON.stringify(createEnvelope('auth', {
        device_id: this.device.deviceId,
        token: this.device.token,
      })));
    };
    socket.onmessage = (event) => this.handleMessage(socket, event.data);
    socket.onerror = () => {
      if (this.socket === socket) this.callbacks.onStatus('offline');
    };
    socket.onclose = () => {
      if (this.socket !== socket || this.disposed) return;
      this.connected = false;
      this.callbacks.onStatus('offline');
      this.callbacks.onSending(false);
      this.scheduleReconnect();
    };
  }

  sendMessage(content: string) {
    if (!this.connected || !this.socket) throw new Error('电脑尚未连接');
    const text = content.trim();
    if (!text) return;
    const local: ChatMessage = {
      id: randomUUID(),
      role: 'user',
      content: text,
      createdAt: Date.now(),
    };
    this.messages = [...this.messages, local];
    this.callbacks.onMessages(this.messages);
    this.callbacks.onSending(true);
    this.socket.send(JSON.stringify(createEnvelope('chat.send', {
      session_id: 'default',
      content: text,
    })));
  }

  dispose() {
    this.disposed = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.socket = null;
  }

  private handleMessage(socket: WebSocket, data: unknown) {
    if (socket !== this.socket) return;
    try {
      const envelope = parseEnvelope(data);
      switch (envelope.type) {
        case 'auth.ok':
          this.connected = true;
          this.retries = 0;
          this.callbacks.onStatus('connected');
          socket.send(JSON.stringify(createEnvelope('chat.history', {
            session_id: 'default',
            limit: 100,
          })));
          break;
        case 'chat.history.result': {
          const payload = envelope.payload as unknown as ChatHistoryResult;
          this.messages = payload.messages.map((message) => ({
            id: message.id,
            role: message.role,
            content: message.content,
            createdAt: message.created_at,
          }));
          this.callbacks.onMessages(this.messages);
          break;
        }
        case 'chat.delta': {
          const delta = envelope.payload as unknown as ChatDelta;
          const existing = this.messages.find((message) => message.id === delta.message_id);
          if (existing) {
            existing.content += delta.delta;
          } else {
            this.messages.push({
              id: delta.message_id,
              role: 'assistant',
              content: delta.delta,
              createdAt: envelope.ts,
            });
          }
          this.callbacks.onMessages([...this.messages]);
          break;
        }
        case 'chat.done': {
          const done = envelope.payload as unknown as ChatDone;
          const existing = this.messages.find((message) => message.id === done.message_id);
          if (existing) existing.content = done.content;
          else this.messages.push({ id: done.message_id, role: 'assistant', content: done.content, createdAt: envelope.ts });
          this.callbacks.onMessages([...this.messages]);
          this.callbacks.onSending(false);
          break;
        }
        case 'error': {
          const payload = envelope.payload as { message?: string };
          this.callbacks.onError(payload.message ?? '发生未知错误');
          this.callbacks.onSending(false);
          break;
        }
      }
    } catch (error) {
      this.callbacks.onError(error instanceof Error ? error.message : '协议消息无效');
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer || this.disposed) return;
    const delay = Math.min(10_000, 1_000 * 2 ** this.retries);
    this.retries += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }
}
