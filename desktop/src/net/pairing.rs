use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use rand::{Rng, RngCore, rngs::OsRng};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const PAIRING_TTL: Duration = Duration::from_secs(300);
const MAX_ATTEMPTS: u8 = 10;
const WORDS: [&str; 32] = [
    "amber", "apple", "birch", "breeze", "brook", "cedar", "cloud", "coral", "dawn", "dune", "elm",
    "fern", "field", "flint", "fox", "glen", "gold", "grove", "harbor", "hazel", "hill", "iris",
    "lake", "leaf", "maple", "moon", "moss", "oak", "pine", "rain", "river", "stone",
];

#[derive(Clone, Serialize)]
pub struct PairingView {
    pub code: String,
    pub public_key: String,
    pub expires_in: u64,
    pub pending: Option<PendingView>,
    /// 最近一次配对成功的设备名。配对页据此给出"已配对"反馈，
    /// 否则配对完成后页面毫无变化，用户会以为没成功。
    pub paired_device: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct PendingView {
    pub id: String,
    pub device_name: String,
    pub short_word: String,
}

pub struct PendingPair {
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub short_word: String,
    pub token: String,
    pub token_hash: String,
    created: Instant,
}

struct PairingState {
    code: String,
    created: Instant,
    attempts: u8,
    pending: HashMap<String, PendingPair>,
    /// 进程内保留最近一次配对成功的设备名，用于配对页的完成反馈。
    last_paired: Option<String>,
}

pub struct PairingManager {
    signing_key: SigningKey,
    state: Mutex<PairingState>,
}

impl PairingManager {
    pub fn new(signing_key: SigningKey) -> Self {
        Self {
            signing_key,
            state: Mutex::new(PairingState {
                code: fresh_code(),
                created: Instant::now(),
                attempts: 0,
                pending: HashMap::new(),
                last_paired: None,
            }),
        }
    }

    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.signing_key.verifying_key().to_bytes())
    }

    pub fn view(&self) -> PairingView {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.created.elapsed() >= PAIRING_TTL {
            state.code = fresh_code();
            state.created = Instant::now();
            state.attempts = 0;
            state.pending.clear();
        }
        state
            .pending
            .retain(|_, pair| pair.created.elapsed() < PAIRING_TTL);
        PairingView {
            code: state.code.clone(),
            public_key: self.public_key(),
            expires_in: PAIRING_TTL
                .saturating_sub(state.created.elapsed())
                .as_secs(),
            pending: state.pending.values().last().map(|pair| PendingView {
                id: pair.device_id.clone(),
                device_name: pair.device_name.clone(),
                short_word: pair.short_word.clone(),
            }),
            paired_device: state.last_paired.clone(),
        }
    }

    /// 记录最近一次配对成功的设备，供配对页展示完成状态。
    pub fn mark_paired(&self, device_name: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.last_paired = Some(device_name.to_owned());
    }

    pub fn start(
        &self,
        code: &str,
        device_id: &str,
        device_name: &str,
        public_key: &str,
    ) -> Result<(String, String)> {
        let key_bytes = URL_SAFE_NO_PAD.decode(public_key)?;
        if key_bytes.len() != 32 {
            bail!("设备公钥长度无效");
        }
        Uuid::parse_str(device_id).map_err(|_| anyhow::anyhow!("设备 ID 必须是 UUID"))?;
        if device_name.trim().is_empty() || device_name.chars().count() > 40 {
            bail!("设备名称需为 1 到 40 个字符");
        }

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.created.elapsed() >= PAIRING_TTL || state.attempts >= MAX_ATTEMPTS {
            state.code = fresh_code();
            state.created = Instant::now();
            state.attempts = 0;
            state.pending.clear();
            bail!("配对码已更新，请重新扫描 PC 上的二维码");
        }
        if code != state.code {
            state.attempts += 1;
            bail!("配对码不正确或已过期");
        }

        let pair_id = Uuid::new_v4().to_string();
        let short_word = short_word(&self.public_key(), public_key, &state.code);
        let mut token = [0_u8; 32];
        OsRng.fill_bytes(&mut token);
        let token = URL_SAFE_NO_PAD.encode(token);
        let token_hash = hash_token(&token);
        state.pending.insert(
            pair_id.clone(),
            PendingPair {
                device_id: device_id.to_owned(),
                device_name: device_name.trim().to_owned(),
                public_key: public_key.to_owned(),
                short_word: short_word.clone(),
                token,
                token_hash,
                created: Instant::now(),
            },
        );
        Ok((pair_id, short_word))
    }

    pub fn confirm(&self, pair_id: &str) -> Result<PendingPair> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let pair = state
            .pending
            .remove(pair_id)
            .ok_or_else(|| anyhow::anyhow!("配对请求不存在，请重新扫码"))?;
        if pair.created.elapsed() >= PAIRING_TTL {
            bail!("配对请求已过期，请重新扫码");
        }
        Ok(pair)
    }

    pub fn cancel(&self, pair_id: &str) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .remove(pair_id);
    }
}

pub fn hash_token(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

fn fresh_code() -> String {
    format!("{:06}", OsRng.gen_range(0..1_000_000))
}

fn short_word(server_key: &str, client_key: &str, code: &str) -> String {
    let digest = Sha256::digest(format!("{server_key}:{client_key}:{code}").as_bytes());
    digest[..4]
        .iter()
        .map(|byte| WORDS[(byte & 31) as usize])
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> PairingManager {
        PairingManager::new(SigningKey::from_bytes(&[7_u8; 32]))
    }

    #[test]
    fn paired_device_is_reported_until_restart() {
        let manager = manager();
        assert!(manager.view().paired_device.is_none());
        manager.mark_paired("Pixel 8");
        assert_eq!(manager.view().paired_device.as_deref(), Some("Pixel 8"));
        // 配对码轮换不应抹掉完成反馈，否则页面刷新一下就"退回去"了。
        assert!(manager.view().code.len() == 6);
        assert_eq!(manager.view().paired_device.as_deref(), Some("Pixel 8"));
    }

    #[test]
    fn pairing_requires_matching_code_and_confirmation() {
        let manager = manager();
        let code = manager.view().code;
        assert!(
            manager
                .start(
                    "000000",
                    &Uuid::new_v4().to_string(),
                    "Phone",
                    &URL_SAFE_NO_PAD.encode([8_u8; 32])
                )
                .is_err()
        );
        let device_id = Uuid::new_v4().to_string();
        let (pair_id, word) = manager
            .start(
                &code,
                &device_id,
                "Pixel",
                &URL_SAFE_NO_PAD.encode([8_u8; 32]),
            )
            .expect("正确配对码应接受");
        assert_eq!(word.split('-').count(), 4);
        let pair = manager.confirm(&pair_id).expect("确认应成功");
        assert_eq!(pair.device_id, device_id);
        assert!(manager.confirm(&pair_id).is_err());
    }

    #[test]
    fn short_words_change_with_client_key() {
        assert_ne!(short_word("a", "b", "1"), short_word("a", "c", "1"));
    }
}
