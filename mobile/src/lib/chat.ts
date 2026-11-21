// @author wzxgqtl 2026-11-21
// 聊天状态。连接本身由 connection.ts 统一持有，这里只负责消息列表的增量维护。

import type {
  ChatDeltaPayload as ChatDelta,
  ChatDonePayload as ChatDone,
  ChatHistoryResultPayload as ChatHistoryResult,
  Envelope,
} from '../generated/contract';
import type { ServerConnection } from './connection';

export type { ConnectionStatus } from './connection';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  createdAt: number;
  /**
   * 仍在流式接收中。
   * 渲染层据此决定用纯文本还是 Markdown：边收边解析会让每帧都重排，
   * 长输出时肉眼可见地卡；等 chat.done 之后一次性渲染更稳。
   */
  streaming?: boolean;
}

interface ChatCallbacks {
  onMessages: (messages: ChatMessage[]) => void;
  onSending: (sending: boolean) => void;
  onError: (message: string) => void;
}

export class ChatStore {
  private messages: ChatMessage[] = [];
  private readonly unsubscribe: () => void;
  private readonly unsubscribeStatus: () => void;
  private historyLoaded = false;

  constructor(
    private readonly connection: ServerConnection,
    private readonly callbacks: ChatCallbacks,
  ) {
    this.unsubscribe = connection.subscribe((envelope) => this.handle(envelope));
    this.unsubscribeStatus = connection.onStatus((status) => {
      if (status === 'connected' && !this.historyLoaded) this.loadHistory();
      if (status === 'offline') this.callbacks.onSending(false);
    });
  }

  loadHistory() {
    if (!this.connection.send('chat.history', { session_id: 'default', limit: 100 })) return;
    this.historyLoaded = true;
  }

  sendMessage(content: string) {
    const text = content.trim();
    if (!text) return;
    if (!this.connection.connected) throw new Error('电脑尚未连接');
    this.messages = [
      ...this.messages,
      { id: `local-${Date.now()}`, role: 'user', content: text, createdAt: Date.now() },
    ];
    this.callbacks.onMessages(this.messages);
    this.callbacks.onSending(true);
    this.connection.send('chat.send', { session_id: 'default', content: text });
  }

  dispose() {
    this.unsubscribe();
    this.unsubscribeStatus();
  }

  private handle(envelope: Envelope) {
    switch (envelope.type) {
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
        if (existing) existing.content += delta.delta;
        else
          this.messages.push({
            id: delta.message_id,
            role: 'assistant',
            content: delta.delta,
            createdAt: envelope.ts,
            streaming: true,
          });
        this.callbacks.onMessages([...this.messages]);
        break;
      }
      case 'chat.done': {
        const done = envelope.payload as unknown as ChatDone;
        const existing = this.messages.find((message) => message.id === done.message_id);
        if (existing) {
          existing.content = done.content;
          // 收完才切到 Markdown 渲染。
          existing.streaming = false;
        } else
          this.messages.push({
            id: done.message_id,
            role: 'assistant',
            content: done.content,
            createdAt: envelope.ts,
            streaming: false,
          });
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
      default:
        break;
    }
  }
}
