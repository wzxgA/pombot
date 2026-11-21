// @author wzxgqtl 2026-11-21
// 与电脑之间的唯一长连接。
//
// 聊天、同步、编辑锁共用一条 WebSocket：服务端按设备 id 登记出站通道，
// 开两条连接会导致广播只落到其中一条，锁状态两边不一致。
// 因此这里做成按设备复用的单例，页面只订阅、不各自建连。

import type { Envelope } from '../generated/contract';
import type { PairedDevice } from './identity';
import { createEnvelope, parseEnvelope, websocketUrl } from './protocol';

export type ConnectionStatus = 'connecting' | 'connected' | 'offline';

type EnvelopeListener = (envelope: Envelope) => void;
type StatusListener = (status: ConnectionStatus) => void;

export class ServerConnection {
  private socket: WebSocket | null = null;
  private readonly listeners = new Set<EnvelopeListener>();
  private readonly statusListeners = new Set<StatusListener>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private retries = 0;
  private disposed = false;

  status: ConnectionStatus = 'connecting';

  constructor(readonly device: PairedDevice) {}

  connect() {
    if (this.disposed || this.socket) return;
    this.setStatus('connecting');
    const socket = new WebSocket(websocketUrl(this.device.serverUrl));
    this.socket = socket;

    socket.onopen = () => {
      socket.send(
        JSON.stringify(
          createEnvelope('auth', {
            device_id: this.device.deviceId,
            token: this.device.token,
          }),
        ),
      );
    };
    socket.onmessage = (event) => this.handleData(socket, event.data);
    socket.onerror = () => {
      if (this.socket === socket) this.setStatus('offline');
    };
    socket.onclose = () => {
      if (this.socket !== socket || this.disposed) return;
      this.socket = null;
      this.setStatus('offline');
      this.scheduleReconnect();
    };
  }

  get connected(): boolean {
    return this.status === 'connected';
  }

  send(type: Envelope['type'], payload: unknown): boolean {
    if (!this.connected || !this.socket) return false;
    this.socket.send(JSON.stringify(createEnvelope(type, payload)));
    return true;
  }

  subscribe(listener: EnvelopeListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  onStatus(listener: StatusListener): () => void {
    this.statusListeners.add(listener);
    listener(this.status);
    return () => this.statusListeners.delete(listener);
  }

  dispose() {
    this.disposed = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.socket = null;
    this.listeners.clear();
    this.statusListeners.clear();
  }

  private handleData(socket: WebSocket, data: unknown) {
    if (this.socket !== socket) return;
    let envelope: Envelope;
    try {
      envelope = parseEnvelope(data);
    } catch {
      return;
    }
    if (envelope.type === 'auth.ok') {
      this.retries = 0;
      this.setStatus('connected');
    }
    for (const listener of this.listeners) listener(envelope);
  }

  private setStatus(status: ConnectionStatus) {
    this.status = status;
    for (const listener of this.statusListeners) listener(status);
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

let current: ServerConnection | null = null;

export function getServerConnection(device: PairedDevice): ServerConnection {
  if (!current || current.device.deviceId !== device.deviceId) {
    current?.dispose();
    current = new ServerConnection(device);
  }
  return current;
}

export function resetServerConnection() {
  current?.dispose();
  current = null;
}
