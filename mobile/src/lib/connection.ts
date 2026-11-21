// @author wzxgqtl 2026-11-22
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

/** 探测单个地址的超时。局域网通常几毫秒，公网经反向代理也就几十毫秒。 */
const PROBE_TIMEOUT_MS = 1500;
/**
 * 心跳间隔。反向代理对空闲连接有超时（Cloudflare 约 100 秒），
 * 30 秒留足余量。服务端收到 ping 会回 pong，不需要额外处理。
 */
const HEARTBEAT_MS = 30_000;

export class ServerConnection {
  private socket: WebSocket | null = null;
  private readonly listeners = new Set<EnvelopeListener>();
  private readonly statusListeners = new Set<StatusListener>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private retries = 0;
  private disposed = false;
  /** 当前实际连通的地址。产物下载也要用它，否则内网切公网后图片会失效。 */
  private endpoint: string | null = null;

  status: ConnectionStatus = 'connecting';

  constructor(readonly device: PairedDevice) {}

  get baseUrl(): string {
    return this.endpoint ?? this.device.serverUrl;
  }

  /**
   * 当前走的是哪条路径。配对时电脑把内网地址放在首位，
   * 因此"不等于首个候选"就说明走的是公网。
   */
  get network(): 'lan' | 'public' | null {
    if (!this.endpoint) return null;
    return this.endpoint === this.device.serverUrl ? 'lan' : 'public';
  }

  connect() {
    if (this.disposed || this.socket) return;
    this.setStatus('connecting');
    void this.establish();
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
    this.stopHeartbeat();
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.socket = null;
    this.listeners.clear();
    this.statusListeners.clear();
  }

  /** 候选地址：局域网优先，公网兜底。去重后按顺序探测。 */
  private candidates(): string[] {
    const list = [this.device.serverUrl, ...(this.device.candidates ?? [])];
    return [...new Set(list.filter(Boolean))];
  }

  private async establish() {
    const best = await pickEndpoint(this.candidates());
    if (this.disposed || this.socket) return;
    this.endpoint = best;
    this.openSocket(best);
  }

  private openSocket(baseUrl: string) {
    const socket = new WebSocket(websocketUrl(baseUrl));
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
      this.stopHeartbeat();
      this.setStatus('offline');
      this.scheduleReconnect();
    };
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
      this.startHeartbeat();
      this.setStatus('connected');
    }
    for (const listener of this.listeners) listener(envelope);
  }

  private startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      if (!this.connected || !this.socket) return;
      this.socket.send(JSON.stringify(createEnvelope('ping', {})));
    }, HEARTBEAT_MS);
  }

  private stopHeartbeat() {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;
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

/**
 * 并发探测候选地址，取最先响应的那个。
 *
 * 用 `/health` 而不是直接开 WebSocket：健康检查无鉴权、开销极小，
 * 而并发开多个 WS 会在握手失败时留下难以清理的半开连接。
 * 全部失败时返回第一个候选，让后续的重连逻辑继续尝试。
 */
async function pickEndpoint(candidates: string[]): Promise<string> {
  if (candidates.length <= 1) return candidates[0];

  const probes = candidates.map(async (base) => {
    const started = Date.now();
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
    try {
      const response = await fetch(`${base}/health`, { signal: controller.signal });
      if (!response.ok) return null;
      return { base, elapsed: Date.now() - started };
    } catch {
      return null;
    } finally {
      clearTimeout(timer);
    }
  });

  const results = (await Promise.all(probes)).filter(
    (item): item is { base: string; elapsed: number } => item !== null,
  );
  if (results.length === 0) return candidates[0];
  results.sort((left, right) => left.elapsed - right.elapsed);
  return results[0].base;
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
