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
}
