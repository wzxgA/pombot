import type {
  PairChallengePayload as PairChallenge,
  PairResultPayload as PairResult,
} from '../generated/contract';
import {
  createIdentity,
  type PairedDevice,
  type PairTicket,
  savePairedDevice,
} from './identity';
import { createEnvelope, parseEnvelope, websocketUrl } from './protocol';

export interface PairSession {
  socket: WebSocket;
  pairId: string;
  shortWord: string;
  device: Omit<PairedDevice, 'deviceId' | 'token'> & { deviceId: string; token: string };
}

export async function beginPairing(ticket: PairTicket, endpoint = ticket.serverUrl): Promise<PairSession> {
  const identity = await createIdentity(ticket);
  const socket = new WebSocket(websocketUrl(endpoint, '/ws/pair'));
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error('连接配对服务超时，请确认手机与电脑在同一 WiFi'));
    }, 12_000);
    const fail = (error: Error) => {
      clearTimeout(timeout);
      socket.close();
      reject(error);
    };
    socket.onerror = () => fail(new Error('无法连接电脑，请检查防火墙和 WiFi'));
    socket.onopen = () => {
      socket.send(JSON.stringify(createEnvelope('pair.start', {
        code: ticket.code,
        device_id: identity.deviceId,
        device_name: identity.deviceName,
        public_key: identity.publicKey,
      })));
    };
    socket.onmessage = (event) => {
      try {
        const envelope = parseEnvelope(event.data);
        if (envelope.type === 'error') {
          const payload = envelope.payload as { message?: string };
          throw new Error(payload.message ?? '配对失败');
        }
        if (envelope.type !== 'pair.challenge') return;
        const challenge = envelope.payload as unknown as PairChallenge;
        if (ticket.serverPublicKey && challenge.server_public_key !== ticket.serverPublicKey) {
          throw new Error('电脑公钥与二维码不一致，配对已中止');
        }
        clearTimeout(timeout);
        socket.onerror = null;
        resolve({
          socket,
          pairId: challenge.pair_id,
          shortWord: challenge.short_word,
          device: { ...identity.device, deviceId: identity.deviceId, token: '' },
        });
      } catch (error) {
        fail(error instanceof Error ? error : new Error('配对响应无效'));
      }
    };
  });
}

export async function confirmPairing(session: PairSession): Promise<PairedDevice> {
  const { socket } = session;
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error('确认配对超时'));
    }, 12_000);
    socket.onerror = () => {
      clearTimeout(timeout);
      reject(new Error('确认配对时连接中断'));
    };
    socket.onmessage = (event) => {
      try {
        const envelope = parseEnvelope(event.data);
        if (envelope.type === 'error') {
          throw new Error((envelope.payload as { message?: string }).message ?? '配对失败');
        }
        if (envelope.type !== 'pair.result') return;
        const result = envelope.payload as unknown as PairResult;
        const device = { ...session.device, deviceId: result.device_id, token: result.token };
        savePairedDevice(device)
          .then(() => {
            clearTimeout(timeout);
            socket.close();
            resolve(device);
          })
          .catch((error) => {
            clearTimeout(timeout);
            socket.close();
            reject(error instanceof Error ? error : new Error('无法安全保存配对凭据'));
          });
      } catch (error) {
        clearTimeout(timeout);
        socket.close();
        reject(error instanceof Error ? error : new Error('配对结果无效'));
      }
    };
    socket.send(JSON.stringify(createEnvelope('pair.confirm', { pair_id: session.pairId })));
  });
}
