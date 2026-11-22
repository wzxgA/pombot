// @author wzxgqtl 2026-11-20
// 此文件由 scripts/gen-contract.sh 生成，请修改 shared/contract/protocol.json。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contract {
    pub artifact_info: Option<ArtifactInfo>,

    pub auth: Option<Auth>,

    pub auth_ok: Option<AuthOk>,

    pub chat_delta: Option<ChatDelta>,

    pub chat_done: Option<ChatDone>,

    pub chat_history: Option<ChatHistory>,

    pub chat_history_result: Option<ChatHistoryResult>,

    pub chat_send: Option<ChatSend>,

    pub vision_cancel: Option<VisionCancel>,

    pub vision_done: Option<VisionDone>,

    pub vision_progress: Option<VisionProgress>,

    pub vision_start: Option<VisionStart>,

    pub envelope: Option<Envelope>,

    pub error: Option<Error>,

    pub lock_changed: Option<LockChanged>,

    pub lock_request: Option<LockRequest>,

    pub lock_state: Option<LockState>,

    pub object_draft: Option<ObjectDraft>,

    pub object_op: Option<ObjectOp>,

    pub object_record: Option<ObjectRecord>,

    pub pair_challenge: Option<PairChallenge>,

    pub pair_confirm: Option<PairConfirm>,

    pub pair_result: Option<PairResult>,

    pub pair_start: Option<PairStart>,

    pub sync_changes: Option<SyncChanges>,

    pub sync_pull: Option<SyncPull>,

    pub sync_push: Option<SyncPush>,

    pub sync_push_result: Option<SyncPushResult>,

    pub tool_confirm: Option<ToolConfirm>,

    pub tool_decision: Option<ToolDecision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub blob_id: String,

    pub mime: String,

    pub name: String,

    pub sha256: String,

    pub size: i64,

    pub thumb_blob_id: Option<String>,
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

    #[serde(rename = "vision.cancel")]
    VisionCancel,

    #[serde(rename = "vision.done")]
    VisionDone,

    #[serde(rename = "vision.progress")]
    VisionProgress,

    #[serde(rename = "vision.start")]
    VisionStart,

    Error,

    #[serde(rename = "lock.acquire")]
    LockAcquire,

    #[serde(rename = "lock.changed")]
    LockChanged,

    #[serde(rename = "lock.heartbeat")]
    LockHeartbeat,

    #[serde(rename = "lock.release")]
    LockRelease,

    #[serde(rename = "lock.state")]
    LockState,

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

    #[serde(rename = "sync.changes")]
    SyncChanges,

    #[serde(rename = "sync.pull")]
    SyncPull,

    #[serde(rename = "sync.push")]
    SyncPush,

    #[serde(rename = "sync.push.result")]
    SyncPushResult,

    #[serde(rename = "tool.confirm")]
    ToolConfirm,

    #[serde(rename = "tool.decision")]
    ToolDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionCancel {
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionDone {
    pub message: String,

    pub success: bool,

    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionProgress {
    pub action_count: i64,

    pub message: String,

    pub status: VisionStatus,

    pub step: i64,

    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionStart {
    pub goal: String,

    pub task_id: String,

    pub window_title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: Option<String>,

    pub message: String,

    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockChanged {
    pub expires_in_ms: i64,

    /// 空字符串表示锁已释放
    pub holder_device_name: String,

    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockRequest {
    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockState {
    pub expires_in_ms: i64,

    pub granted: bool,

    /// 未持有时为空字符串
    pub holder_device_name: String,

    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectDraft {
    /// 客户端编辑时持有的版本号；0 表示新建
    pub base_rev: i64,

    pub content: String,

    /// 空字符串表示新建，由服务端分配
    pub id: String,

    pub title: String,

    #[serde(rename = "type")]
    pub object_draft_type: ObjectType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    Artifact,

    Memo,

    Note,

    Todo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectOp {
    pub draft: ObjectDraft,

    pub op: ObjectOpKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectOpKind {
    Delete,

    Upsert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectRecord {
    pub artifact: Option<ArtifactInfo>,

    pub content: String,

    pub created_at: i64,

    /// 0 表示未删除；非 0 为墓碑，客户端据此移除
    pub deleted_at: i64,

    pub id: String,

    pub rev: i64,

    pub seq: i64,

    pub title: String,

    #[serde(rename = "type")]
    pub object_record_type: ObjectType,

    pub updated_at: i64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncChanges {
    pub objects: Vec<ObjectRecord>,

    /// 本次结果对应的最新 seq，客户端应存下来作为下次 since_seq
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPull {
    pub limit: Option<i32>,

    pub since_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPush {
    pub ops: Vec<ObjectOp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPushResult {
    pub accepted: i64,

    /// 提交时 base_rev 已过期的对象，回传服务端当前版本供客户端另存副本
    pub conflicts: Vec<ObjectRecord>,

    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolConfirm {
    pub call_id: String,

    pub expires_in_ms: i64,

    pub risk: Risk,

    /// 给用户看的一句话说明，描述这次操作到底做什么
    pub summary: String,

    pub tool: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Blocked,

    High,

    Low,

    Medium,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionStatus {
    Actions,
    Cancelled,
    Failed,
    Finished,
    Observed,
    Started,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDecision {
    pub approve: bool,

    pub call_id: String,
}
