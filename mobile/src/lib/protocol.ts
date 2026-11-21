import { randomUUID } from 'expo-crypto';

import type { Envelope } from '../generated/contract';

export function createEnvelope(type: Envelope['type'], payload: unknown): Envelope {
  return {
    v: 1,
    id: randomUUID(),
    type,
    ts: Date.now(),
    nonce: randomUUID(),
    payload,
  } as Envelope;
}

export function parseEnvelope(data: unknown): Envelope {
  if (typeof data !== 'string') throw new Error('收到非文本协议消息');
  const envelope = JSON.parse(data) as Envelope;
  if (envelope.v !== 1 || !envelope.id || !envelope.type || !envelope.nonce) {
    throw new Error('收到无效的协议消息');
  }
  return envelope;
}

export function websocketUrl(serverUrl: string, path = '/ws'): string {
  return `${serverUrl.replace(/^http/, 'ws').replace(/\/$/, '')}${path}`;
}
