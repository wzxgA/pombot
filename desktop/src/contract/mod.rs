// @author wzxgqtl 2026-11-20
// 此文件由 scripts/gen-contract.sh 生成，请修改 shared/contract/protocol.json。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contract {
    pub auth: Option<Auth>,

    pub auth_ok: Option<AuthOk>,

    pub chat_delta: Option<ChatDelta>,

    pub chat_done: Option<ChatDone>,

    pub chat_history: Option<ChatHistory>,

    pub chat_history_result: Option<ChatHistoryResult>,

    pub chat_send: Option<ChatSend>,

    pub envelope: Option<Envelope>,

    pub error: Option<Error>,

    pub pair_challenge: Option<PairChallenge>,

    pub pair_confirm: Option<PairConfirm>,

    pub pair_result: Option<PairResult>,

    pub pair_start: Option<PairStart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Auth {
    pub device_id: String,

    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthOk {
    pub device_id: String,

    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDelta {
    pub delta: String,

    pub message_id: String,

    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDone {
    pub content: String,

    pub message_id: String,

    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatHistory {
    pub limit: Option<i32>,

    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatHistoryResult {
    pub messages: Vec<Protocol>,

    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Protocol {
    pub content: String,

    pub created_at: i64,

    pub id: String,

    pub role: Role,

    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Assistant,

    System,

    User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSend {
    pub content: String,

    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,

    pub nonce: String,

    pub payload: Option<serde_json::Value>,

    pub ts: i64,

    #[serde(rename = "type")]
    pub envelope_type: Type,

    pub v: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    Auth,

    #[serde(rename = "auth.ok")]
    AuthOk,

    #[serde(rename = "chat.delta")]
    ChatDelta,

    #[serde(rename = "chat.done")]
    ChatDone,

    #[serde(rename = "chat.history")]
    ChatHistory,

    #[serde(rename = "chat.history.result")]
    ChatHistoryResult,

    #[serde(rename = "chat.send")]
    ChatSend,

    Error,

    #[serde(rename = "pair.challenge")]
    PairChallenge,

    #[serde(rename = "pair.confirm")]
    PairConfirm,

    #[serde(rename = "pair.result")]
    PairResult,

    #[serde(rename = "pair.start")]
    PairStart,

    Ping,

    Pong,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: Option<String>,

    pub message: String,

    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairChallenge {
    pub pair_id: String,

    pub server_public_key: String,

    pub short_word: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairConfirm {
    pub pair_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairResult {
    pub device_id: String,

    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairStart {
    pub code: String,

    pub device_id: String,

    pub device_name: String,

    pub public_key: String,
}
