// @author wzxgqtl 2026-11-22
// 用户偏好。与设备身份分开存放：身份丢了就要重新配对，
// 而"上次选了哪种连接方式"丢了大不了回到自动。

import * as SecureStore from 'expo-secure-store';

export type ConnectionMode = 'auto' | 'lan' | 'public';

const MODE_KEY = 'pombot.connection.mode';

const MODES: ConnectionMode[] = ['auto', 'lan', 'public'];

export async function loadConnectionMode(): Promise<ConnectionMode> {
  try {
    const value = await SecureStore.getItemAsync(MODE_KEY);
    if (value && MODES.includes(value as ConnectionMode)) {
      return value as ConnectionMode;
    }
  } catch {
    // 读不出来就用默认值，不该因为偏好读取失败挡住连接。
  }
  return 'auto';
}

export async function saveConnectionMode(mode: ConnectionMode): Promise<void> {
  try {
    await SecureStore.setItemAsync(MODE_KEY, mode);
  } catch {
    // 写失败只影响下次启动的默认值，忽略。
  }
}
