import * as Crypto from 'expo-crypto';
import * as Device from 'expo-device';
import * as SecureStore from 'expo-secure-store';
import nacl from 'tweetnacl';

const STORAGE_KEY = 'pombot.device.v1';

export interface PairTicket {
  serverUrl: string;
  code: string;
  serverPublicKey: string;
}

export interface PairedDevice {
  deviceId: string;
  token: string;
  seed: string;
  publicKey: string;
  serverPublicKey: string;
  serverUrl: string;
}

export function encodeBase64Url(bytes: Uint8Array): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  let encoded = '';
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index];
    const second = bytes[index + 1] ?? 0;
    const third = bytes[index + 2] ?? 0;
    const value = (first << 16) | (second << 8) | third;
    encoded += alphabet[(value >> 18) & 63];
    encoded += alphabet[(value >> 12) & 63];
    encoded += index + 1 < bytes.length ? alphabet[(value >> 6) & 63] : '=';
    encoded += index + 2 < bytes.length ? alphabet[value & 63] : '=';
  }
  return encoded.replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

export function decodeBase64Url(value: string): Uint8Array {
  const normalized = value.replaceAll('-', '+').replaceAll('_', '/');
  const padded = normalized + '='.repeat((4 - (normalized.length % 4)) % 4);
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const clean = padded.replace(/=+$/, '');
  const output: number[] = [];
  let buffer = 0;
  let bits = 0;
  for (const character of clean) {
    const index = alphabet.indexOf(character);
    if (index < 0) throw new Error('密钥编码无效');
    buffer = (buffer << 6) | index;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      output.push((buffer >> bits) & 255);
    }
  }
  return new Uint8Array(output);
}

export function parsePairTicket(data: string): PairTicket {
  const url = new URL(data);
  const serverUrl = url.searchParams.get('url') ?? '';
  const code = url.searchParams.get('code') ?? '';
  const serverPublicKey = url.searchParams.get('pubkey') ?? '';
  const endpoint = new URL(serverUrl);
  if (url.protocol !== 'pombot:' || !['http:', 'https:'].includes(endpoint.protocol)) {
    throw new Error('这不是有效的 PomBot 配对二维码');
  }
  if (!/^\d{6}$/.test(code) || !serverPublicKey) {
    throw new Error('二维码缺少配对码或公钥，请刷新后重试');
  }
  return { serverUrl: serverUrl.replace(/\/$/, ''), code, serverPublicKey };
}

export async function loadPairedDevice(): Promise<PairedDevice | null> {
  const value = await SecureStore.getItemAsync(STORAGE_KEY);
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as PairedDevice;
    if (!parsed.deviceId || !parsed.token || !parsed.seed || !parsed.serverUrl) return null;
    return parsed;
  } catch {
    return null;
  }
}

export async function savePairedDevice(device: PairedDevice): Promise<void> {
  await SecureStore.setItemAsync(STORAGE_KEY, JSON.stringify(device));
}

export async function clearPairedDevice(): Promise<void> {
  await SecureStore.deleteItemAsync(STORAGE_KEY);
}

export async function createIdentity(ticket: PairTicket) {
  const seed = Crypto.getRandomBytes(32);
  const keyPair = nacl.sign.keyPair.fromSeed(seed);
  return {
    deviceId: Crypto.randomUUID(),
    deviceName: Device.modelName?.slice(0, 40) || 'Android',
    publicKey: encodeBase64Url(keyPair.publicKey),
    device: {
      deviceId: '',
      token: '',
      seed: encodeBase64Url(seed),
      publicKey: encodeBase64Url(keyPair.publicKey),
      serverPublicKey: ticket.serverPublicKey,
      serverUrl: ticket.serverUrl,
    } satisfies PairedDevice,
  };
}

export function publicKeyFromSeed(seed: string): string {
  return encodeBase64Url(nacl.sign.keyPair.fromSeed(decodeBase64Url(seed)).publicKey);
}
