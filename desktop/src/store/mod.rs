use std::{
    fs,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contract::{ArtifactInfo, ObjectDraft, ObjectRecord, ObjectType};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub pubkey: String,
    pub paired_at: i64,
    pub last_seen: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("无法创建数据库目录 {}", parent.display()))?;
        }
        let connection =
            Connection::open(path).with_context(|| format!("无法打开数据库 {}", path.display()))?;
        Self::initialize(connection)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        connection
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS devices (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    pubkey TEXT NOT NULL UNIQUE,
                    token_hash TEXT NOT NULL,
                    paired_at INTEGER NOT NULL,
                    last_seen INTEGER NOT NULL,
                    last_seq INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    tool_calls_json TEXT,
                    job_id TEXT,
                    created_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS messages_session_created
                    ON messages(session_id, created_at, id);
                CREATE TABLE IF NOT EXISTS nonces (
                    device_id TEXT NOT NULL,
                    nonce TEXT NOT NULL,
                    seen_at INTEGER NOT NULL,
                    PRIMARY KEY(device_id, nonce)
                );
                CREATE TABLE IF NOT EXISTS meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                -- 对象表：笔记 / 备忘 / 待办 / 产物统一存放。
                -- 用单表而不是每类一张，是为了让新增类型只加一个枚举值，不必写迁移与客户端页面。
                CREATE TABLE IF NOT EXISTS objects (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    content TEXT NOT NULL DEFAULT '',
                    meta_json TEXT NOT NULL DEFAULT '{}',
                    artifact_json TEXT,
                    seq INTEGER NOT NULL,
                    rev INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS objects_seq ON objects(seq);
                CREATE INDEX IF NOT EXISTS objects_kind ON objects(kind, updated_at DESC);

                -- 悲观编辑锁：同一对象同一时刻只允许一个设备持有。
                CREATE TABLE IF NOT EXISTS edit_locks (
                    object_id TEXT PRIMARY KEY,
                    device_id TEXT NOT NULL,
                    acquired_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL
                );

                -- 产物实体。内容按需通过 HTTP 拉取，同步只传元数据。
                CREATE TABLE IF NOT EXISTS blobs (
                    id TEXT PRIMARY KEY,
                    sha256 TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    mime TEXT NOT NULL,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    thumb_path TEXT,
                    created_at INTEGER NOT NULL
                );
                ",
            )
            .context("初始化数据库失败")?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn signing_key(&self) -> Result<SigningKey> {
        let connection = self.lock()?;
        if let Some(encoded) = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'pc_ed25519_key'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("读取 PC 密钥失败")?
        {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("PC 密钥不是有效 Base64")?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("PC 密钥长度无效"))?;
            return Ok(SigningKey::from_bytes(&bytes));
        }

        let mut secret = [0_u8; 32];
        OsRng.fill_bytes(&mut secret);
        let encoded = URL_SAFE_NO_PAD.encode(secret);
        connection
            .execute(
                "INSERT INTO meta(key, value) VALUES ('pc_ed25519_key', ?1)",
                params![encoded],
            )
            .context("保存 PC 密钥失败")?;
        Ok(SigningKey::from_bytes(&secret))
    }

    pub fn add_device(
        &self,
        id: &str,
        name: &str,
        pubkey: &str,
        token_hash: &str,
    ) -> Result<Device> {
        let now = now_millis();
        self.lock()?
            .execute(
                "INSERT INTO devices(id, name, pubkey, token_hash, paired_at, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id, name, pubkey, token_hash, now],
            )
            .context("保存配对设备失败；该设备公钥可能已经配对")?;
        Ok(Device {
            id: id.to_owned(),
            name: name.to_owned(),
            pubkey: pubkey.to_owned(),
            paired_at: now,
            last_seen: now,
        })
    }

    pub fn authenticate(&self, device_id: &str, token_hash: &str) -> Result<Option<Device>> {
        let connection = self.lock()?;
        let device = connection
            .query_row(
                "SELECT id, name, pubkey, paired_at, last_seen FROM devices
                 WHERE id = ?1 AND token_hash = ?2",
                params![device_id, token_hash],
                map_device,
            )
            .optional()
            .context("校验设备失败")?;
        if device.is_some() {
            connection
                .execute(
                    "UPDATE devices SET last_seen = ?2 WHERE id = ?1",
                    params![device_id, now_millis()],
                )
                .context("更新设备在线时间失败")?;
        }
        Ok(device)
    }

    pub fn remember_nonce(&self, device_id: &str, nonce: &str, ts: i64) -> Result<bool> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM nonces WHERE seen_at < ?1",
                params![now_millis() - 300_000],
            )
            .context("清理过期 nonce 失败")?;
        let inserted = connection.execute(
            "INSERT INTO nonces(device_id, nonce, seen_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id, nonce) DO NOTHING",
            params![device_id, nonce, ts],
        )?;
        Ok(inserted == 1)
    }

    pub fn append_message(&self, session_id: &str, role: &str, content: &str) -> Result<Message> {
        let message = Message {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            created_at: now_millis(),
        };
        self.lock()?
            .execute(
                "INSERT INTO messages(id, session_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    message.id,
                    message.session_id,
                    message.role,
                    message.content,
                    message.created_at
                ],
            )
            .context("保存消息失败")?;
        Ok(message)
    }

    pub fn update_message_content(&self, message_id: &str, content: &str) -> Result<()> {
        let changed = self
            .lock()?
            .execute(
                "UPDATE messages SET content = ?2 WHERE id = ?1",
                params![message_id, content],
            )
            .context("更新流式消息失败")?;
        if changed != 1 {
            bail!("找不到要更新的消息 {message_id}");
        }
        Ok(())
    }

    pub fn history(&self, session_id: &str, limit: usize) -> Result<Vec<Message>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, session_id, role, content, created_at FROM (
                 SELECT rowid AS sequence, id, session_id, role, content, created_at FROM messages
                 WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2
             ) ORDER BY created_at ASC, sequence ASC",
        )?;
        let rows = statement.query_map(params![session_id, limit as i64], map_message)?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.context("读取历史消息失败")?);
        }
        Ok(messages)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("数据库锁已失效"))
    }
}

// ===================== 对象、同步与编辑锁（M2） =====================

/// 编辑锁的租期。心跳每 15s 续约，因此 60s 足够容忍一次丢包；
/// 而手机进程被杀时，另一端最多等这么久就能接管。
pub const LOCK_TTL_MS: i64 = 60_000;

/// 写入结果。发生冲突时不覆盖服务端数据，而是把当前版本回传，由客户端另存副本。
#[derive(Debug)]
pub enum UpsertOutcome {
    Saved(ObjectRecord),
    Conflict(ObjectRecord),
}

/// 一次取锁尝试的结果。未获得时 `holder_name` 是当前持有者。
#[derive(Clone, Debug)]
pub struct LockAttempt {
    pub granted: bool,
    pub holder_name: String,
    pub expires_in_ms: i64,
}

/// 当前锁的只读快照，用于状态查询与广播。
#[derive(Clone, Debug)]
pub struct LockSnapshot {
    pub device_id: String,
    pub device_name: String,
    pub expires_in_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Blob {
    pub id: String,
    pub sha256: String,
    pub size: i64,
    pub mime: String,
    pub name: String,
    pub path: String,
    pub thumb_path: Option<String>,
    pub created_at: i64,
}

const OBJECT_COLUMNS: &str =
    "id, kind, title, content, artifact_json, seq, rev, created_at, updated_at, deleted_at";

impl Store {
    /// 全局自增序号。客户端的增量同步游标就是它。
    pub fn max_seq(&self) -> Result<i64> {
        let connection = self.lock()?;
        let value: i64 = connection
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'object_seq'",
                [],
                |row| row.get(0),
            )
            .optional()
            .context("读取 seq 失败")?
            .unwrap_or(0);
        Ok(value)
    }

    /// 新建或更新一条对象。
    ///
    /// `base_rev` 是客户端编辑时持有的版本号：0 表示新建，非 0 且与服务端不一致时判为冲突。
    pub fn upsert_object(&self, draft: &ObjectDraft) -> Result<UpsertOutcome> {
        let connection = self.lock()?;
        let now = now_millis();
        let existing = if draft.id.is_empty() {
            None
        } else {
            load_object(&connection, &draft.id)?
        };

        match existing {
            None => {
                let id = if draft.id.is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    draft.id.clone()
                };
                let seq = bump_seq(&connection)?;
                connection
                    .execute(
                        "INSERT INTO objects(id, kind, title, content, seq, rev, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                        params![
                            id,
                            kind_str(&draft.object_draft_type),
                            draft.title,
                            draft.content,
                            seq,
                            now
                        ],
                    )
                    .context("新建对象失败")?;
                let record = load_object(&connection, &id)?.context("新建后读回失败")?;
                Ok(UpsertOutcome::Saved(record))
            }
            Some(current) => {
                // 已删除的对象再被修改：一律按冲突处理，让客户端看到墓碑而不是悄悄复活。
                if current.deleted_at != 0 {
                    return Ok(UpsertOutcome::Conflict(current));
                }
                if draft.base_rev != 0 && draft.base_rev != current.rev {
                    return Ok(UpsertOutcome::Conflict(current));
                }
                let seq = bump_seq(&connection)?;
                connection
                    .execute(
                        "UPDATE objects
                         SET kind = ?2, title = ?3, content = ?4, seq = ?5,
                             rev = rev + 1, updated_at = ?6
                         WHERE id = ?1",
                        params![
                            draft.id,
                            kind_str(&draft.object_draft_type),
                            draft.title,
                            draft.content,
                            seq,
                            now
                        ],
                    )
                    .context("更新对象失败")?;
                let record = load_object(&connection, &draft.id)?.context("更新后读回失败")?;
                Ok(UpsertOutcome::Saved(record))
            }
        }
    }

    /// 软删除：写入墓碑并分配新的 seq，客户端据此移除本地条目。
    pub fn delete_object(&self, object_id: &str, base_rev: i64) -> Result<UpsertOutcome> {
        let connection = self.lock()?;
        let current = match load_object(&connection, object_id)? {
            Some(record) => record,
            None => bail!("对象不存在：{object_id}"),
        };
        if current.deleted_at != 0 {
            return Ok(UpsertOutcome::Conflict(current));
        }
        if base_rev != 0 && base_rev != current.rev {
            return Ok(UpsertOutcome::Conflict(current));
        }
        let now = now_millis();
        let seq = bump_seq(&connection)?;
        connection
            .execute(
                "UPDATE objects SET deleted_at = ?2, seq = ?3, rev = rev + 1, updated_at = ?2
                 WHERE id = ?1",
                params![object_id, now, seq],
            )
            .context("删除对象失败")?;
        let record = load_object(&connection, object_id)?.context("删除后读回失败")?;
        Ok(UpsertOutcome::Saved(record))
    }

    /// 增量拉取：返回 seq 大于 `since_seq` 的对象，按 seq 升序。
    pub fn objects_since(&self, since_seq: i64, limit: usize) -> Result<Vec<ObjectRecord>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {OBJECT_COLUMNS} FROM objects WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2"
        ))?;
        let rows = statement.query_map(params![since_seq, limit as i64], map_object)?;
        let mut objects = Vec::new();
        for row in rows {
            objects.push(row.context("读取对象失败")?);
        }
        Ok(objects)
    }

    pub fn object_by_id(&self, object_id: &str) -> Result<Option<ObjectRecord>> {
        let connection = self.lock()?;
        load_object(&connection, object_id)
    }

    /// 取锁或续约。同一设备重复获取视为续约，不会自锁。
    pub fn acquire_lock(
        &self,
        object_id: &str,
        device_id: &str,
        ttl_ms: i64,
    ) -> Result<LockAttempt> {
        let connection = self.lock()?;
        let now = now_millis();
        expire_locks(&connection, now)?;

        let existing = connection
            .query_row(
                "SELECT l.device_id, COALESCE(d.name, '未知设备'), l.expires_at
                 FROM edit_locks l LEFT JOIN devices d ON d.id = l.device_id
                 WHERE l.object_id = ?1",
                params![object_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .context("读取编辑锁失败")?;

        match existing {
            Some((holder_id, holder_name, expires_at)) if holder_id != device_id => {
                Ok(LockAttempt {
                    granted: false,
                    holder_name,
                    expires_in_ms: (expires_at - now).max(0),
                })
            }
            Some(_) => {
                connection
                    .execute(
                        "UPDATE edit_locks SET expires_at = ?2 WHERE object_id = ?1",
                        params![object_id, now + ttl_ms],
                    )
                    .context("续约编辑锁失败")?;
                Ok(LockAttempt {
                    granted: true,
                    holder_name: String::new(),
                    expires_in_ms: ttl_ms,
                })
            }
            None => {
                connection
                    .execute(
                        "INSERT INTO edit_locks(object_id, device_id, acquired_at, expires_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![object_id, device_id, now, now + ttl_ms],
                    )
                    .context("创建编辑锁失败")?;
                Ok(LockAttempt {
                    granted: true,
                    holder_name: String::new(),
                    expires_in_ms: ttl_ms,
                })
            }
        }
    }

    /// 释放锁。只有持有者才能释放，返回是否真的释放了。
    pub fn release_lock(&self, object_id: &str, device_id: &str) -> Result<bool> {
        let changed = self
            .lock()?
            .execute(
                "DELETE FROM edit_locks WHERE object_id = ?1 AND device_id = ?2",
                params![object_id, device_id],
            )
            .context("释放编辑锁失败")?;
        Ok(changed == 1)
    }

    /// 设备断线时清掉它持有的全部锁，返回被释放的对象 id，供广播使用。
    pub fn release_device_locks(&self, device_id: &str) -> Result<Vec<String>> {
        let connection = self.lock()?;
        let mut statement =
            connection.prepare("SELECT object_id FROM edit_locks WHERE device_id = ?1")?;
        let rows = statement.query_map(params![device_id], |row| row.get::<_, String>(0))?;
        let mut released = Vec::new();
        for row in rows {
            released.push(row?);
        }
        connection
            .execute(
                "DELETE FROM edit_locks WHERE device_id = ?1",
                params![device_id],
            )
            .context("清理设备锁失败")?;
        Ok(released)
    }

    pub fn current_lock(&self, object_id: &str) -> Result<Option<LockSnapshot>> {
        let connection = self.lock()?;
        let now = now_millis();
        expire_locks(&connection, now)?;
        let snapshot = connection
            .query_row(
                "SELECT l.device_id, COALESCE(d.name, '未知设备'), l.expires_at
                 FROM edit_locks l LEFT JOIN devices d ON d.id = l.device_id
                 WHERE l.object_id = ?1",
                params![object_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .context("读取编辑锁失败")?;
        Ok(
            snapshot.map(|(device_id, device_name, expires_at)| LockSnapshot {
                device_id,
                device_name,
                expires_in_ms: (expires_at - now).max(0),
            }),
        )
    }

    /// 以产物身份新建一条对象。产物内容不进同步通道，只有这里写入的元数据会同步到手机。
    pub fn create_artifact_object(&self, info: &ArtifactInfo, title: &str) -> Result<ObjectRecord> {
        let connection = self.lock()?;
        let now = now_millis();
        let id = Uuid::new_v4().to_string();
        let seq = bump_seq(&connection)?;
        let artifact_json = serde_json::to_string(info).context("序列化产物信息失败")?;
        connection
            .execute(
                "INSERT INTO objects(id, kind, title, content, artifact_json, seq, rev,
                                     created_at, updated_at)
                 VALUES (?1, 'artifact', ?2, '', ?3, ?4, 1, ?5, ?5)",
                params![id, title, artifact_json, seq, now],
            )
            .context("保存产物对象失败")?;
        load_object(&connection, &id)?.context("读回产物对象失败")
    }

    pub fn insert_blob(&self, blob: &Blob) -> Result<()> {
        self.lock()?
            .execute(
                "INSERT INTO blobs(id, sha256, size, mime, name, path, thumb_path, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    blob.id,
                    blob.sha256,
                    blob.size,
                    blob.mime,
                    blob.name,
                    blob.path,
                    blob.thumb_path,
                    blob.created_at
                ],
            )
            .context("保存产物失败")?;
        Ok(())
    }

    pub fn blob_by_id(&self, blob_id: &str) -> Result<Option<Blob>> {
        let blob = self
            .lock()?
            .query_row(
                "SELECT id, sha256, size, mime, name, path, thumb_path, created_at
                 FROM blobs WHERE id = ?1",
                params![blob_id],
                |row| {
                    Ok(Blob {
                        id: row.get(0)?,
                        sha256: row.get(1)?,
                        size: row.get(2)?,
                        mime: row.get(3)?,
                        name: row.get(4)?,
                        path: row.get(5)?,
                        thumb_path: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("读取产物失败")?;
        Ok(blob)
    }
}

fn bump_seq(connection: &Connection) -> Result<i64> {
    let current: i64 = connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'object_seq'",
            [],
            |row| row.get(0),
        )
        .optional()
        .context("读取 seq 失败")?
        .unwrap_or(0);
    let next = current + 1;
    connection
        .execute(
            "INSERT INTO meta(key, value) VALUES ('object_seq', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![next.to_string()],
        )
        .context("更新 seq 失败")?;
    Ok(next)
}

fn expire_locks(connection: &Connection, now: i64) -> Result<()> {
    connection
        .execute(
            "DELETE FROM edit_locks WHERE expires_at <= ?1",
            params![now],
        )
        .context("清理过期锁失败")?;
    Ok(())
}

fn load_object(connection: &Connection, object_id: &str) -> Result<Option<ObjectRecord>> {
    connection
        .query_row(
            &format!("SELECT {OBJECT_COLUMNS} FROM objects WHERE id = ?1"),
            params![object_id],
            map_object,
        )
        .optional()
        .context("读取对象失败")
}

fn kind_str(kind: &ObjectType) -> &'static str {
    match kind {
        ObjectType::Note => "note",
        ObjectType::Memo => "memo",
        ObjectType::Todo => "todo",
        ObjectType::Artifact => "artifact",
    }
}

fn parse_kind(value: &str) -> ObjectType {
    match value {
        "memo" => ObjectType::Memo,
        "todo" => ObjectType::Todo,
        "artifact" => ObjectType::Artifact,
        _ => ObjectType::Note,
    }
}

fn map_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<ObjectRecord> {
    let artifact: Option<String> = row.get(4)?;
    Ok(ObjectRecord {
        id: row.get(0)?,
        object_record_type: parse_kind(&row.get::<_, String>(1)?),
        title: row.get(2)?,
        content: row.get(3)?,
        artifact: artifact.and_then(|value| serde_json::from_str::<ArtifactInfo>(&value).ok()),
        seq: row.get(5)?,
        rev: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        deleted_at: row.get(9)?,
    })
}

fn map_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        pubkey: row.get(2)?,
        paired_at: row.get(3)?,
        last_seen: row.get(4)?,
    })
}

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        Store::in_memory().expect("测试数据库应创建")
    }

    #[test]
    fn persists_devices_messages_and_rejects_replayed_nonce() {
        let store = test_store();
        let device = store
            .add_device("phone", "Pixel", "public-key", "token-hash")
            .expect("设备应保存");
        assert!(
            store
                .authenticate(&device.id, "token-hash")
                .expect("认证查询")
                .is_some()
        );
        assert!(
            store
                .authenticate(&device.id, "wrong")
                .expect("认证查询")
                .is_none()
        );
        assert!(
            store
                .remember_nonce(&device.id, "n1", now_millis())
                .expect("nonce")
        );
        assert!(
            !store
                .remember_nonce(&device.id, "n1", now_millis())
                .expect("nonce")
        );

        store
            .append_message("default", "user", "你好")
            .expect("用户消息");
        store
            .append_message("default", "assistant", "你好，我是 Pom")
            .expect("助手消息");
        let history = store.history("default", 10).expect("历史");
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].content, "你好，我是 Pom");
    }

    #[test]
    fn signing_key_is_stable() {
        let store = test_store();
        let first = store.signing_key().expect("首次密钥");
        let second = store.signing_key().expect("再次读取密钥");
        assert_eq!(first.to_bytes(), second.to_bytes());
    }

    fn note_draft(id: &str, title: &str, content: &str, base_rev: i64) -> ObjectDraft {
        ObjectDraft {
            id: id.to_owned(),
            object_draft_type: ObjectType::Note,
            title: title.to_owned(),
            content: content.to_owned(),
            base_rev,
        }
    }

    #[test]
    fn objects_sync_incrementally_and_detect_rev_conflict() {
        let store = test_store();

        let UpsertOutcome::Saved(created) = store
            .upsert_object(&note_draft("", "第一篇", "正文 A", 0))
            .expect("新建")
        else {
            panic!("新建不应冲突");
        };
        assert_eq!(created.rev, 1);
        assert!(created.seq > 0);
        assert_eq!(store.objects_since(0, 100).expect("全量").len(), 1);

        let UpsertOutcome::Saved(updated) = store
            .upsert_object(&note_draft(&created.id, "第一篇", "正文 B", created.rev))
            .expect("更新")
        else {
            panic!("正常更新不应冲突");
        };
        assert_eq!(updated.rev, 2);
        assert!(updated.seq > created.seq);

        // 用过期版本号再提交：必须判冲突，并回传服务端当前内容。
        let UpsertOutcome::Conflict(current) = store
            .upsert_object(&note_draft(&created.id, "第一篇", "正文 C", created.rev))
            .expect("冲突")
        else {
            panic!("旧 base_rev 应判冲突");
        };
        assert_eq!(current.content, "正文 B");

        // 增量只应拉到 seq 更大的那条。
        let delta = store.objects_since(created.seq, 100).expect("增量");
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].rev, 2);

        // 软删除后仍出现在增量里，并带墓碑标记。
        let UpsertOutcome::Saved(deleted) = store.delete_object(&created.id, 2).expect("删除")
        else {
            panic!("删除不应冲突");
        };
        assert!(deleted.deleted_at > 0);
        let after = store.objects_since(0, 100).expect("删除后");
        assert_eq!(after.len(), 1);
        assert!(after[0].deleted_at > 0);
    }

    #[test]
    fn edit_lock_is_exclusive_then_expires() {
        let store = test_store();
        store
            .add_device("phone", "Pixel", "pk-phone", "tk-phone")
            .expect("设备");
        store
            .add_device("pc", "台式机", "pk-pc", "tk-pc")
            .expect("设备");

        let UpsertOutcome::Saved(record) = store
            .upsert_object(&note_draft("", "锁测试", "", 0))
            .expect("新建")
        else {
            panic!("新建不应冲突");
        };

        let first = store
            .acquire_lock(&record.id, "phone", LOCK_TTL_MS)
            .expect("取锁");
        assert!(first.granted);

        let second = store
            .acquire_lock(&record.id, "pc", LOCK_TTL_MS)
            .expect("取锁");
        assert!(!second.granted);
        assert_eq!(second.holder_name, "Pixel");

        // 同一设备重复获取应视为续约，不能自己被自己挡住。
        assert!(
            store
                .acquire_lock(&record.id, "phone", LOCK_TTL_MS)
                .expect("续约")
                .granted
        );

        // 租期为 0 时立即过期，另一方应当能接管。
        assert!(
            store
                .acquire_lock(&record.id, "phone", 0)
                .expect("零租期")
                .granted
        );
        let takeover = store
            .acquire_lock(&record.id, "pc", LOCK_TTL_MS)
            .expect("接管");
        assert!(takeover.granted);

        // 断线时清掉该设备持有的锁，返回待广播的对象 id。
        let released = store.release_device_locks("pc").expect("断线释放");
        assert_eq!(released, vec![record.id.clone()]);
        assert!(store.current_lock(&record.id).expect("查询").is_none());
    }
}
