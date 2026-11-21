// @author wzxgqtl 2026-11-21
// 对象（笔记 / 备忘 / 产物）的本地镜像与编辑锁。
//
// 同步模型：电脑是唯一真相源，手机只记录 lastSeq，每次 pull 拉取 seq 更大的条目。
// 编辑模型：悲观锁——同一时刻只有一台设备能写，另一端只读并显示持有者。

import type {
  Envelope,
  LockChangedPayload as LockChanged,
  LockStatePayload as LockState,
  ObjectRecord,
  SyncChangesPayload as SyncChanges,
  SyncPushResultPayload as SyncPushResult,
} from '../generated/contract';
import type { ServerConnection } from './connection';
import type { PairedDevice } from './identity';

export type ObjectKind = 'note' | 'memo' | 'todo' | 'artifact';

export interface ObjectItem {
  id: string;
  kind: ObjectKind;
  title: string;
  content: string;
  seq: number;
  rev: number;
  createdAt: number;
  updatedAt: number;
  deletedAt: number;
  artifact?: {
    blobId: string;
    name: string;
    mime: string;
    size: number;
  };
}

export interface LockInfo {
  holder: string;
  expiresIn: number;
}

interface NotesCallbacks {
  onChanged: () => void;
  onError: (message: string) => void;
  onLockChanged: (objectId: string, info: LockInfo | null) => void;
}

export interface LockResult {
  granted: boolean;
  holder: string;
  expiresIn: number;
}

export class NotesStore {
  private readonly objects = new Map<string, ObjectItem>();
  private readonly locks = new Map<string, LockInfo>();
  private readonly pendingLocks = new Map<string, (result: LockResult) => void>();
  private lastSeq = 0;
  private readonly unsubscribe: () => void;

  constructor(
    private readonly connection: ServerConnection,
    private readonly callbacks: NotesCallbacks,
  ) {
    this.unsubscribe = connection.subscribe((envelope) => this.handle(envelope));
  }

  /** 从上次位置增量拉取。首次调用传 0 即全量。 */
  pull() {
    this.connection.send('sync.pull', { since_seq: this.lastSeq, limit: 1000 });
  }

  list(kind: ObjectKind): ObjectItem[] {
    return [...this.objects.values()]
      .filter((item) => item.kind === kind && item.deletedAt === 0)
      .sort((left, right) => right.updatedAt - left.updatedAt);
  }

  byId(id: string): ObjectItem | undefined {
    return this.objects.get(id);
  }

  lockOf(id: string): LockInfo | undefined {
    return this.locks.get(id);
  }

  /** 新建或更新。`baseRev` 为当前版本号，新建传 0。 */
  save(draft: { id: string; kind: ObjectKind; title: string; content: string; baseRev: number }) {
    this.connection.send('sync.push', {
      ops: [
        {
          op: 'upsert',
          draft: {
            id: draft.id,
            type: draft.kind,
            title: draft.title,
            content: draft.content,
            base_rev: draft.baseRev,
          },
        },
      ],
    });
  }

  remove(id: string, baseRev: number) {
    const existing = this.objects.get(id);
    this.connection.send('sync.push', {
      ops: [
        {
          op: 'delete',
          draft: {
            id,
            type: existing?.kind ?? 'note',
            title: existing?.title ?? '',
            content: '',
            base_rev: baseRev,
          },
        },
      ],
    });
  }

  /** 取锁。返回结果时 granted=false 表示另一端正在编辑。 */
  acquireLock(objectId: string): Promise<LockResult> {
    return new Promise((resolve) => {
      // 超时兜底：网络异常时不能让编辑页永远卡在"正在获取编辑权"。
      const timer = setTimeout(() => {
        this.pendingLocks.delete(objectId);
        resolve({ granted: false, holder: '未知设备', expiresIn: 0 });
      }, 5000);
      this.pendingLocks.set(objectId, (result) => {
        clearTimeout(timer);
        resolve(result);
      });
      if (!this.connection.send('lock.acquire', { object_id: objectId })) {
        clearTimeout(timer);
        this.pendingLocks.delete(objectId);
        resolve({ granted: false, holder: '电脑未连接', expiresIn: 0 });
      }
    });
  }

  heartbeat(objectId: string) {
    this.connection.send('lock.heartbeat', { object_id: objectId });
  }

  releaseLock(objectId: string) {
    this.connection.send('lock.release', { object_id: objectId });
  }

  dispose() {
    this.unsubscribe();
  }

  private handle(envelope: Envelope) {
    switch (envelope.type) {
      case 'sync.changes': {
        const payload = envelope.payload as unknown as SyncChanges;
        for (const record of payload.objects) this.merge(record);
        this.lastSeq = Math.max(this.lastSeq, payload.seq);
        this.callbacks.onChanged();
        break;
      }
      case 'sync.push.result': {
        const payload = envelope.payload as unknown as SyncPushResult;
        this.lastSeq = Math.max(this.lastSeq, payload.seq);
        for (const record of payload.conflicts) {
          // 冲突不覆盖：服务端版本直接落地，界面会显示最新内容，避免用户误以为保存成功。
          this.merge(record);
        }
        if (payload.conflicts.length > 0) {
          this.callbacks.onError('有内容在你编辑期间被改动，已显示电脑上的最新版本');
        }
        this.callbacks.onChanged();
        this.pull();
        break;
      }
      case 'lock.state': {
        const payload = envelope.payload as unknown as LockState;
        const pending = this.pendingLocks.get(payload.object_id);
        if (pending) {
          this.pendingLocks.delete(payload.object_id);
          pending({
            granted: payload.granted,
            holder: payload.holder_device_name,
            expiresIn: payload.expires_in_ms,
          });
        }
        this.applyLock(payload.object_id, payload.granted ? null : {
          holder: payload.holder_device_name,
          expiresIn: payload.expires_in_ms,
        });
        break;
      }
      case 'lock.changed': {
        const payload = envelope.payload as unknown as LockChanged;
        const info =
          payload.holder_device_name === ''
            ? null
            : { holder: payload.holder_device_name, expiresIn: payload.expires_in_ms };
        this.applyLock(payload.object_id, info);
        break;
      }
      default:
        break;
    }
  }

  private applyLock(objectId: string, info: LockInfo | null) {
    if (info) this.locks.set(objectId, info);
    else this.locks.delete(objectId);
    this.callbacks.onLockChanged(objectId, info);
  }

  private merge(record: ObjectRecord) {
    this.objects.set(record.id, {
      id: record.id,
      kind: record.type as ObjectKind,
      title: record.title,
      content: record.content,
      seq: record.seq,
      rev: record.rev,
      createdAt: record.created_at,
      updatedAt: record.updated_at,
      deletedAt: record.deleted_at,
      artifact: record.artifact
        ? {
            blobId: record.artifact.blob_id,
            name: record.artifact.name,
            mime: record.artifact.mime,
            size: record.artifact.size,
          }
        : undefined,
    });
  }
}

/**
 * 产物下载地址。鉴权走查询参数，因为 <Image> 无法附加请求头。
 *
 * `baseUrl` 由连接层提供而不是直接取 `device.serverUrl`：
 * 手机可能正在用公网地址，写死内网地址会导致图片加载失败。
 */
export function artifactUrl(
  baseUrl: string,
  device: PairedDevice,
  blobId: string,
  thumb = false,
): string {
  const query = new URLSearchParams({
    device_id: device.deviceId,
    token: device.token,
    ...(thumb ? { thumb: '1' } : {}),
  });
  return `${baseUrl.replace(/\/$/, '')}/artifacts/${blobId}?${query.toString()}`;
}
