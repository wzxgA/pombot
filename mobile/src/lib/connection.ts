// @author wzxgqtl 2026-11-22
// 与电脑之间的唯一长连接。
//
// 聊天、同步、编辑锁共用一条 WebSocket：服务端按设备 id 登记出站通道，
// 开两条连接会导致广播只落到其中一条，锁状态两边不一致。
// 因此这里做成按设备复用的单例，页面只订阅、不各自建连。
//
// 移动网络下有三件事必须处理，否则会出现"明明有网却连不上"：
// 1. 切网后旧 TCP 变成半开连接，onclose 常常几分钟才触发 → 需要心跳看门狗
// 2. 握手成功但认证没回来，socket 会一直挂着不报错 → 需要认证超时
// 3. 一直连同一个地址失败时要轮换 → 否则永远试不到能用的那个

import { AppState } from 'react-native';

import type { Envelope } from '../generated/contract';
import type { PairedDevice } from './identity';
import type { ConnectionMode } from './prefs';
import { createEnvelope, parseEnvelope, websocketUrl } from './protocol';

export type ConnectionStatus = 'connecting' | 'connected' | 'offline';

type EnvelopeListener = (envelope: Envelope) => void;
type StatusListener = (status: ConnectionStatus) => void;

/** 探测单个地址的超时。局域网通常几毫秒，公网经反向代理也就几十毫秒。 */
const PROBE_TIMEOUT_MS = 1500;
/** 心跳间隔。反向代理对空闲连接有超时（Cloudflare 约 100 秒）。 */
const HEARTBEAT_MS = 30_000;
/** 多久没收到任何消息就认定连接已死。取心跳间隔的 2.5 倍。 */
const SILENCE_LIMIT_MS = 75_000;
/** 握手成功后多久没等到 auth.ok 就重来。 */
const AUTH_TIMEOUT_MS = 8_000;

export class ServerConnection {
  private socket: WebSocket | null = null;
  private readonly listeners = new Set<EnvelopeListener>();
  private readonly statusListeners = new Set<StatusListener>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private authTimer: ReturnType<typeof setTimeout> | null = null;
  private retries = 0;
  private disposed = false;
  /** 当前实际连通的地址。产物下载也要用它，否则内网切公网后图片会失效。 */
  private endpoint: string | null = null;
  /** 每次强制重连加一，用于轮换候选地址的顺序。 */
  private rotation = 0;
  /** 最后一次收到的入站消息时间，心跳看门狗据此判断连接是否已死。 */
  private lastInboundAt = 0;
  /** 最近一次探测结果，界面上直接显示它，省得用户只能看到"请检查网络"。 */
  diagnostic = '';
  /** 前台/后台监听只注册一次。 */
  private appStateBound = false;

  status: ConnectionStatus = 'connecting';
  /** 连接优先级：自动 / 只用局域网 / 只用公网。 */
  mode: ConnectionMode = 'auto';

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
    if (!this.appStateBound) {
      this.appStateBound = true;
      AppState.addEventListener('change', (next) => {
        if (next === 'active') this.onAppForeground();
      });
    }
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

  /** 回到前台时检查一次。手机切网或休眠后，旧连接通常已经不可用但不会报错。 */
  onAppForeground() {
    if (this.disposed) return;
    if (this.status !== 'connected') {
      this.forceReconnect('回到前台，重新连接');
    }
  }

  /** 切换连接模式，并立刻按新模式重连。 */
  setMode(mode: ConnectionMode) {
    if (this.mode === mode) return;
    this.mode = mode;
    this.rotation = 0;
    this.retries = 0;
    if (this.socket) {
      this.forceReconnect('已切换连接模式');
    } else {
      this.connect();
    }
  }

  dispose() {
    this.disposed = true;
    this.stopHeartbeat();
    this.clearAuthTimer();
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.socket = null;
    this.listeners.clear();
    this.statusListeners.clear();
  }

  /** 候选地址：按当前模式筛选，再按重连次数轮换起点。 */
  private candidates(): string[] {
    const lan = this.device.serverUrl;
    const others = [...new Set(this.device.candidates ?? [])].filter((url) => url !== lan);

    let pool: string[];
    switch (this.mode) {
      case 'lan':
        pool = [lan];
        break;
      case 'public':
        // 强制走公网时不回退局域网：回退会让用户以为"切了却没生效"。
        pool = others;
        break;
      default:
        pool = [lan, ...others];
    }

    if (pool.length <= 1) return pool;
    const shift = this.rotation % pool.length;
    return [...pool.slice(shift), ...pool.slice(0, shift)];
  }

  private async establish() {
    const candidates = this.candidates();
    if (candidates.length === 0) {
      // 强制公网但这次配对没带公网地址：说清原因，别让用户以为手机坏了。
      this.diagnostic = '这次配对没有包含公网地址，请重新扫码配对';
      this.setStatus('offline');
      this.scheduleReconnect();
      return;
    }

    const { best, report } = await probeAll(candidates);
    this.diagnostic = report.join('；');
    if (this.disposed || this.socket) return;
    this.endpoint = best;
    this.openSocket(best);
  }

  private openSocket(baseUrl: string) {
    const socket = new WebSocket(websocketUrl(baseUrl));
    this.socket = socket;
    this.lastInboundAt = Date.now();

    // 握手成功不等于能用：认证没回来时连接会安静地挂着，必须有超时。
    this.authTimer = setTimeout(() => {
      if (this.socket === socket && this.status !== 'connected') {
        this.forceReconnect('认证超时');
      }
    }, AUTH_TIMEOUT_MS);

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
      if (this.socket === socket) this.forceReconnect('连接出错');
    };
    socket.onclose = () => {
      if (this.socket !== socket || this.disposed) return;
      this.forceReconnect('连接被关闭');
    };
  }

  private handleData(socket: WebSocket, data: unknown) {
    if (this.socket !== socket) return;
    this.lastInboundAt = Date.now();
    let envelope: Envelope;
    try {
      envelope = parseEnvelope(data);
    } catch {
      return;
    }
    if (envelope.type === 'auth.ok') {
      this.retries = 0;
      this.rotation = 0;
      this.clearAuthTimer();
      this.startHeartbeat();
      this.setStatus('connected');
    }
    for (const listener of this.listeners) listener(envelope);
  }

  private startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      if (!this.connected || !this.socket) return;
      // 看门狗：切网后 TCP 会变成半开，发送仍然"成功"但对方收不到，
      // 只能靠"多久没收到东西"来判断连接是否已死。
      if (Date.now() - this.lastInboundAt > SILENCE_LIMIT_MS) {
        this.forceReconnect('长时间没有收到电脑的回应');
        return;
      }
      this.socket.send(JSON.stringify(createEnvelope('ping', {})));
    }, HEARTBEAT_MS);
  }

  private stopHeartbeat() {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;
  }

  private clearAuthTimer() {
    if (this.authTimer) clearTimeout(this.authTimer);
    this.authTimer = null;
  }

  private setStatus(status: ConnectionStatus) {
    this.status = status;
    for (const listener of this.statusListeners) listener(status);
  }

  /** 主动丢弃当前连接并进入重连。用于超时、切网、看门狗触发。 */
  private forceReconnect(reason: string) {
    if (this.disposed) return;
    this.diagnostic = `${reason}（${this.diagnostic || '无探测结果'}）`;
    const socket = this.socket;
    this.socket = null;
    this.stopHeartbeat();
    this.clearAuthTimer();
    try {
      socket?.close();
    } catch {
      // 半开连接关闭可能抛异常，忽略即可，两边都已不可用。
    }
    this.rotation += 1;
    this.setStatus('offline');
    this.scheduleReconnect();
  }

  private scheduleReconnect() {
    if (this.reconnectTimer || this.disposed) return;
    const delay = Math.min(10_000, 1_000 * 2 ** this.retries);
    this.retries += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.socket) return;
      this.connect();
    }, delay);
  }
}

/**
 * 并发探测候选地址，取最先响应的那个。
 *
 * 用 `/health` 而不是直接开 WebSocket：健康检查无鉴权、开销极小，
 * 而并发开多个 WS 会在握手失败时留下难以清理的半开连接。
 * 全部失败时返回第一个候选，并保留每一条的失败原因供界面显示。
 */
async function probeAll(
  candidates: string[],
): Promise<{ best: string; report: string[] }> {
  if (candidates.length === 0) return { best: '', report: ['没有可用地址'] };

  // 单个候选也要真实探测：强制模式下用户需要知道这条路到底通不通，
  // "仅一个地址"这种敷衍的结论毫无用处。
  const probes = candidates.map(async (base) => {
    const started = Date.now();
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
    try {
      const response = await fetch(`${base}/health`, { signal: controller.signal });
      if (!response.ok) {
        return { base, ok: false, note: `HTTP ${response.status}` };
      }
      return { base, ok: true, note: `${Date.now() - started}ms` };
    } catch {
      return { base, ok: false, note: '不可达' };
    } finally {
      clearTimeout(timer);
    }
  });

  const results = await Promise.all(probes);
  const usable = results
    .filter((item) => item.ok)
    .sort((left, right) => Number.parseInt(left.note, 10) - Number.parseInt(right.note, 10));

  const report = results.map(
    (item) => `${shorten(item.base)} ${item.ok ? '✓' : '✗'} ${item.note}`,
  );
  return { best: usable[0]?.base ?? candidates[0], report };
}

function shorten(url: string): string {
  return url.replace(/^https?:\/\//, '');
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
