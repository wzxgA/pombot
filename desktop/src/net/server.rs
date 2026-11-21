use std::{
    collections::HashMap,
    net::UdpSocket,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use qrcode::QrCode;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc};
use tracing::{info, warn};
use url::Url;

/// 一次对话最多允许几轮工具调用。超过就停下来，避免模型反复调工具烧 token。
const MAX_TOOL_ROUNDS: usize = 4;

use crate::{
    config::{AppConfig, AppPaths},
    contract::{
        Auth, AuthOk, ChatDelta, ChatDone, ChatHistory, ChatHistoryResult, ChatSend, Envelope,
        LockChanged, LockRequest, LockState, ObjectOpKind, PairChallenge, PairConfirm, PairResult,
        PairStart, Risk, SyncChanges, SyncPull, SyncPush, SyncPushResult, ToolConfirm,
        ToolDecision, Type,
    },
    core::llm::{LlmClient, RequestMessage},
    net::{
        discovery,
        pairing::{PairingManager, hash_token},
        protocol::{self, error_envelope, new_envelope},
    },
    store::{LOCK_TTL_MS, Message, Store, UpsertOutcome},
};

#[derive(Clone)]
struct AppState {
    config: AppConfig,
    store: Arc<Store>,
    pairing: Arc<PairingManager>,
    local_url: String,
    /// 工具产出文件与产物实体都落在这下面。
    data_dir: std::path::PathBuf,
    /// 当前在线设备 -> 出站通道。锁变更需要广播给所有设备，因此必须集中登记。
    clients: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Envelope>>>>,
    /// 等待用户裁决的高风险工具调用。key 是 call_id。
    pending_confirms: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
}

/// 高风险操作的确认窗口。超时按拒绝处理，避免无人值守时自动放行。
const CONFIRM_TIMEOUT_MS: i64 = 60_000;

/// 把一条消息推给所有在线设备。锁的归属变化需要让每一端立刻刷新只读态。
fn broadcast(state: &AppState, envelope_type: Type, payload: impl serde::Serialize) {
    let Ok(envelope) = new_envelope(envelope_type, payload) else {
        return;
    };
    let clients = state
        .clients
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for sender in clients.values() {
        let _ = sender.send(envelope.clone());
    }
}

pub async fn serve(config: AppConfig, paths: AppPaths) -> Result<()> {
    let store = Arc::new(Store::open(&paths.database_file)?);
    let pairing = Arc::new(PairingManager::new(store.signing_key()?));
    let ip = local_ip().unwrap_or_else(|| "127.0.0.1".to_owned());
    let local_url = format!("http://{ip}:{}", config.server.port);
    let state = AppState {
        config: config.clone(),
        store,
        pairing,
        local_url: local_url.clone(),
        data_dir: paths.data_dir.clone(),
        clients: Arc::new(Mutex::new(HashMap::new())),
        pending_confirms: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/artifacts/{blob_id}", get(artifact))
        .route("/pairing", get(pairing_page))
        .route("/pairing/qr.svg", get(pairing_qr))
        .route("/pairing/status", get(pairing_status))
        .route("/ws/pair", get(pair_upgrade))
        .route("/ws", get(device_upgrade))
        .with_state(state);

    let address = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("无法监听 {address}；请检查端口是否被占用"))?;
    let _mdns = if ip != "127.0.0.1" && config.server.host != "127.0.0.1" {
        match discovery::advertise(config.server.port, &ip) {
            Ok(daemon) => {
                info!("mDNS 已广播 _pombot._tcp.local.");
                Some(daemon)
            }
            Err(error) => {
                warn!("mDNS 广播失败，仍可使用二维码连接：{error:#}");
                None
            }
        }
    } else {
        None
    };
    if let Some(problem) = config.llm_problem() {
        warn!("{problem}");
    }
    info!("配对页面：http://127.0.0.1:{}/pairing", config.server.port);
    info!("手机连接地址：{local_url}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .context("网络服务异常退出")
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}))
}

async fn pairing_status(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(address): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<Value>, StatusCode> {
    require_local(address)?;
    Ok(Json(
        serde_json::to_value(state.pairing.view()).unwrap_or_else(|_| json!({})),
    ))
}

async fn pairing_qr(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(address): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<impl IntoResponse, StatusCode> {
    require_local(address)?;
    let view = state.pairing.view();
    let mut url = Url::parse("pombot://pair").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("url", &state.local_url)
        .append_pair("code", &view.code)
        .append_pair("pubkey", &view.public_key);
    let code = QrCode::new(url.as_str()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(qrcode::render::svg::Color("#23362f"))
        .light_color(qrcode::render::svg::Color("#fffdf8"))
        .build();
    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], svg))
}

async fn pairing_page(
    axum::extract::ConnectInfo(address): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Html<&'static str>, StatusCode> {
    require_local(address)?;
    Ok(Html(PAIRING_PAGE))
}

#[derive(Deserialize)]
struct ArtifactQuery {
    device_id: String,
    token: String,
    /// 传 1 时返回缩略图，列表页用它避免整图解码。
    thumb: Option<String>,
}

/// 产物下载。支持 Range，方便手机端断点续传与边下边显示。
///
/// 鉴权走查询参数而不是请求头：React Native 的 <Image> 无法附加自定义头，
/// 图片必须能直接用 URL 加载。
async fn artifact(
    State(state): State<AppState>,
    Path(blob_id): Path<String>,
    Query(query): Query<ArtifactQuery>,
    headers: HeaderMap,
) -> Response {
    if state
        .store
        .authenticate(&query.device_id, &hash_token(&query.token))
        .ok()
        .flatten()
        .is_none()
    {
        return (StatusCode::UNAUTHORIZED, "设备未授权").into_response();
    }

    let blob = match state.store.blob_by_id(&blob_id) {
        Ok(Some(blob)) => blob,
        Ok(None) => return (StatusCode::NOT_FOUND, "产物不存在").into_response(),
        Err(error) => {
            warn!("读取产物失败：{error:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "读取产物失败").into_response();
        }
    };

    let want_thumb = query.thumb.as_deref() == Some("1");
    let (path, mime) = match (&blob.thumb_path, want_thumb) {
        (Some(thumb), true) => (thumb.clone(), "image/png".to_owned()),
        _ => (blob.path.clone(), blob.mime.clone()),
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!("读取产物文件失败 {}: {error}", path);
            return (StatusCode::NOT_FOUND, "产物文件已丢失").into_response();
        }
    };

    let total = bytes.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, total));

    match range {
        Some((start, end)) => {
            let slice = bytes[start..=end].to_vec();
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, slice.len())
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{total}"),
                )
                .body(Body::from(slice))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, total)
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

/// 解析 `bytes=start-end`。只处理单区间，够用且不引入额外依赖。
fn parse_range(value: &str, total: usize) -> Option<(usize, usize)> {
    let spec = value.strip_prefix("bytes=")?;
    if total == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let start: usize = start.parse().ok()?;
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<usize>().ok()?
    };
    if start > end || end >= total {
        return None;
    }
    Some((start, end))
}

fn require_local(address: std::net::SocketAddr) -> Result<(), StatusCode> {
    if address.ip().is_loopback() {
        Ok(())
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn pair_upgrade(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle_pairing(socket, state))
}

async fn device_upgrade(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle_device(socket, state))
}

async fn handle_pairing(mut socket: WebSocket, state: AppState) {
    let mut active_pair: Option<String> = None;
    while let Some(received) = socket.recv().await {
        let envelope = match received {
            Ok(WsMessage::Text(text)) => match serde_json::from_str::<Envelope>(&text) {
                Ok(envelope) => envelope,
                Err(error) => {
                    let _ = send_envelope(
                        &mut socket,
                        error_envelope(format!("消息格式错误：{error}"), None),
                    )
                    .await;
                    continue;
                }
            },
            Ok(WsMessage::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        if let Err(error) = protocol::validate(&envelope) {
            let _ = send_envelope(
                &mut socket,
                error_envelope(error.to_string(), Some(envelope.id)),
            )
            .await;
            continue;
        }
        let request_id = envelope.id.clone();
        let result = match envelope.envelope_type {
            Type::PairStart => {
                let parsed = protocol::payload::<PairStart>(&envelope);
                match parsed {
                    Ok(payload) => match state.pairing.start(
                        &payload.code,
                        &payload.device_id,
                        &payload.device_name,
                        &payload.public_key,
                    ) {
                        Ok((pair_id, short_word)) => {
                            active_pair = Some(pair_id.clone());
                            new_envelope(
                                Type::PairChallenge,
                                PairChallenge {
                                    pair_id,
                                    server_public_key: state.pairing.public_key(),
                                    short_word,
                                },
                            )
                        }
                        Err(error) => Ok(error_envelope(error.to_string(), Some(request_id))),
                    },
                    Err(error) => Ok(error_envelope(
                        format!("配对参数错误：{error}"),
                        Some(request_id),
                    )),
                }
            }
            Type::PairConfirm => {
                let parsed = protocol::payload::<PairConfirm>(&envelope);
                match parsed {
                    Ok(payload) => match state.pairing.confirm(&payload.pair_id) {
                        Ok(pair) => match state.store.add_device(
                            &pair.device_id,
                            &pair.device_name,
                            &pair.public_key,
                            &pair.token_hash,
                        ) {
                            Ok(_) => {
                                active_pair = None;
                                // 让配对页立刻显示"已配对"，否则用户配对完看不出任何变化。
                                state.pairing.mark_paired(&pair.device_name);
                                new_envelope(
                                    Type::PairResult,
                                    PairResult {
                                        device_id: pair.device_id,
                                        token: pair.token,
                                    },
                                )
                            }
                            Err(error) => Ok(error_envelope(
                                format!("设备落库失败：{error:#}"),
                                Some(request_id),
                            )),
                        },
                        Err(error) => Ok(error_envelope(error.to_string(), Some(request_id))),
                    },
                    Err(error) => Ok(error_envelope(
                        format!("确认参数错误：{error}"),
                        Some(request_id),
                    )),
                }
            }
            _ => Ok(error_envelope(
                "配对连接仅支持 pair.start 与 pair.confirm",
                Some(request_id),
            )),
        };
        match result {
            Ok(response) => {
                if send_envelope(&mut socket, response).await.is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = send_envelope(&mut socket, error_envelope(error.to_string(), None)).await;
            }
        }
    }
    if let Some(pair_id) = active_pair {
        state.pairing.cancel(&pair_id);
    }
}

async fn handle_device(mut socket: WebSocket, state: AppState) {
    let device = match tokio::time::timeout(
        Duration::from_secs(10),
        authenticate(&mut socket, &state),
    )
    .await
    {
        Ok(Ok(device)) => device,
        Ok(Err(error)) => {
            let _ = send_envelope(
                &mut socket,
                error_envelope(format!("认证失败：{error:#}"), None),
            )
            .await;
            return;
        }
        Err(_) => {
            let _ = send_envelope(&mut socket, error_envelope("认证超时", None)).await;
            return;
        }
    };
    if send_envelope(
        &mut socket,
        new_envelope(
            Type::AuthOk,
            AuthOk {
                device_id: device.id.clone(),
                // 保留一份名字：后续广播锁归属时还要用。
                device_name: device.name.clone(),
            },
        )
        .unwrap_or_else(|error| error_envelope(error.to_string(), None)),
    )
    .await
    .is_err()
    {
        return;
    }

    let (mut socket_sender, mut socket_receiver) = socket.split();
    let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<Envelope>();
    state
        .clients
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(device.id.clone(), outbound.clone());
    let writer = tokio::spawn(async move {
        while let Some(envelope) = outbound_rx.recv().await {
            let Ok(text) = serde_json::to_string(&envelope) else {
                continue;
            };
            if socket_sender
                .send(WsMessage::Text(text.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(received) = socket_receiver.next().await {
        let envelope = match received {
            Ok(WsMessage::Text(text)) => match serde_json::from_str::<Envelope>(&text) {
                Ok(envelope) => envelope,
                Err(error) => {
                    let _ = outbound.send(error_envelope(format!("消息格式错误：{error}"), None));
                    continue;
                }
            },
            Ok(WsMessage::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        let request_id = envelope.id.clone();
        let checked = protocol::validate(&envelope).and_then(|_| {
            if state
                .store
                .remember_nonce(&device.id, &envelope.nonce, envelope.ts)?
            {
                Ok(())
            } else {
                bail!("检测到重复 nonce，消息已拒绝")
            }
        });
        if let Err(error) = checked {
            let _ = outbound.send(error_envelope(error.to_string(), Some(request_id)));
            continue;
        }

        match envelope.envelope_type {
            Type::Ping => {
                if let Ok(response) = new_envelope(Type::Pong, json!({})) {
                    let _ = outbound.send(response);
                }
            }
            Type::ChatHistory => {
                match protocol::payload::<ChatHistory>(&envelope).and_then(|payload| {
                    let session_id = payload.session_id.unwrap_or_else(|| "default".to_owned());
                    let limit = payload.limit.unwrap_or(100).clamp(1, 200) as usize;
                    let messages = state.store.history(&session_id, limit)?;
                    new_envelope(
                        Type::ChatHistoryResult,
                        ChatHistoryResult {
                            messages: messages.into_iter().map(Into::into).collect(),
                            session_id,
                        },
                    )
                }) {
                    Ok(response) => {
                        let _ = outbound.send(response);
                    }
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("读取历史失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                }
            }
            Type::ChatSend => match protocol::payload::<ChatSend>(&envelope) {
                Ok(payload) => {
                    let state = state.clone();
                    let outbound = outbound.clone();
                    tokio::spawn(async move {
                        run_chat(state, outbound, request_id, payload).await;
                    });
                }
                Err(error) => {
                    let _ = outbound.send(error_envelope(
                        format!("消息参数错误：{error}"),
                        Some(request_id),
                    ));
                }
            },
            Type::SyncPull => {
                match protocol::payload::<SyncPull>(&envelope).and_then(|payload| {
                    let limit = payload.limit.unwrap_or(500).clamp(1, 1000) as usize;
                    let objects = state.store.objects_since(payload.since_seq, limit)?;
                    let seq = state.store.max_seq()?;
                    new_envelope(Type::SyncChanges, SyncChanges { objects, seq })
                }) {
                    Ok(response) => {
                        let _ = outbound.send(response);
                    }
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("拉取同步失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                }
            }
            Type::SyncPush => {
                match protocol::payload::<SyncPush>(&envelope).and_then(|payload| {
                    let mut accepted = 0_i64;
                    let mut conflicts = Vec::new();
                    for op in payload.ops {
                        let outcome = match op.op {
                            ObjectOpKind::Upsert => state.store.upsert_object(&op.draft)?,
                            ObjectOpKind::Delete => {
                                state.store.delete_object(&op.draft.id, op.draft.base_rev)?
                            }
                        };
                        match outcome {
                            UpsertOutcome::Saved(_) => accepted += 1,
                            // 冲突一律不覆盖，把服务端版本回传，由客户端另存副本。
                            UpsertOutcome::Conflict(current) => conflicts.push(current),
                        }
                    }
                    let seq = state.store.max_seq()?;
                    new_envelope(
                        Type::SyncPushResult,
                        SyncPushResult {
                            accepted,
                            conflicts,
                            seq,
                        },
                    )
                }) {
                    Ok(response) => {
                        let _ = outbound.send(response);
                    }
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("提交同步失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                }
            }
            Type::LockAcquire => {
                match protocol::payload::<LockRequest>(&envelope).and_then(|payload| {
                    let attempt =
                        state
                            .store
                            .acquire_lock(&payload.object_id, &device.id, LOCK_TTL_MS)?;
                    if attempt.granted {
                        broadcast(
                            &state,
                            Type::LockChanged,
                            LockChanged {
                                object_id: payload.object_id.clone(),
                                holder_device_name: device.name.clone(),
                                expires_in_ms: attempt.expires_in_ms,
                            },
                        );
                    }
                    new_envelope(
                        Type::LockState,
                        LockState {
                            object_id: payload.object_id,
                            granted: attempt.granted,
                            holder_device_name: attempt.holder_name,
                            expires_in_ms: attempt.expires_in_ms,
                        },
                    )
                }) {
                    Ok(response) => {
                        let _ = outbound.send(response);
                    }
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("获取编辑锁失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                }
            }
            Type::LockHeartbeat => {
                match protocol::payload::<LockRequest>(&envelope).and_then(|payload| {
                    // 续约失败通常意味着锁已被别人接管，回传真实状态让客户端退回只读。
                    let attempt =
                        state
                            .store
                            .acquire_lock(&payload.object_id, &device.id, LOCK_TTL_MS)?;
                    new_envelope(
                        Type::LockState,
                        LockState {
                            object_id: payload.object_id,
                            granted: attempt.granted,
                            holder_device_name: attempt.holder_name,
                            expires_in_ms: attempt.expires_in_ms,
                        },
                    )
                }) {
                    Ok(response) => {
                        let _ = outbound.send(response);
                    }
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("续约编辑锁失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                }
            }
            Type::LockRelease => match protocol::payload::<LockRequest>(&envelope) {
                Ok(payload) => match state.store.release_lock(&payload.object_id, &device.id) {
                    Ok(true) => {
                        broadcast(
                            &state,
                            Type::LockChanged,
                            LockChanged {
                                object_id: payload.object_id,
                                holder_device_name: String::new(),
                                expires_in_ms: 0,
                            },
                        );
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let _ = outbound.send(error_envelope(
                            format!("释放编辑锁失败：{error:#}"),
                            Some(request_id),
                        ));
                    }
                },
                Err(error) => {
                    let _ = outbound.send(error_envelope(
                        format!("消息参数错误：{error}"),
                        Some(request_id),
                    ));
                }
            },
            Type::ToolDecision => match protocol::payload::<ToolDecision>(&envelope) {
                Ok(payload) => {
                    let sender = state
                        .pending_confirms
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .remove(&payload.call_id);
                    if let Some(sender) = sender {
                        let _ = sender.send(payload.approve);
                    }
                }
                Err(error) => {
                    let _ = outbound.send(error_envelope(
                        format!("消息参数错误：{error}"),
                        Some(request_id),
                    ));
                }
            },
            _ => {
                let _ = outbound.send(error_envelope("不支持的消息类型", Some(request_id)));
            }
        }
    }
    writer.abort();

    // 断线清理：先从广播表里摘掉，再释放它持有的编辑锁，
    // 否则另一端会一直看到"某某正在编辑"，直到锁自然过期。
    state
        .clients
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(&device.id);
    match state.store.release_device_locks(&device.id) {
        Ok(released) => {
            for object_id in released {
                broadcast(
                    &state,
                    Type::LockChanged,
                    LockChanged {
                        object_id,
                        holder_device_name: String::new(),
                        expires_in_ms: 0,
                    },
                );
            }
        }
        Err(error) => warn!("释放设备编辑锁失败：{error:#}"),
    }
}

fn run_tool(state: &AppState, name: &str, arguments: &str) -> String {
    match crate::core::tools::execute(name, arguments, &state.store, &state.data_dir) {
        Ok(text) => text,
        Err(error) => format!("工具执行失败：{error:#}"),
    }
}

/// 高风险调用：广播确认请求并等待任一设备裁决。
///
/// 没有在线设备时直接拒绝——"无人值守"不能成为自动放行的理由。
/// 超时同样按拒绝处理，避免流程永久挂起。
async fn wait_for_approval(state: &AppState, call_id: &str, tool: &str, summary: &str) -> bool {
    let online = !state
        .clients
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_empty();
    if !online {
        warn!("高风险工具 {tool} 无设备在线，直接拒绝");
        return false;
    }

    let (sender, receiver) = tokio::sync::oneshot::channel();
    state
        .pending_confirms
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(call_id.to_owned(), sender);

    broadcast(
        state,
        Type::ToolConfirm,
        ToolConfirm {
            call_id: call_id.to_owned(),
            tool: tool.to_owned(),
            summary: summary.to_owned(),
            risk: Risk::High,
            expires_in_ms: CONFIRM_TIMEOUT_MS,
        },
    );

    // 双端裁决：手机点了"允许"或电脑点了"拒绝"，谁先到算谁。
    // 电脑端窗口未启动时（例如测试环境）退化为只等手机。
    let pc_receiver = crate::gui::confirm::ask(tool, summary, CONFIRM_TIMEOUT_MS);
    let mut phone_receiver = receiver;
    let decided = tokio::time::timeout(
        Duration::from_millis(CONFIRM_TIMEOUT_MS as u64),
        async move {
            match pc_receiver {
                Some(mut pc) => {
                    tokio::select! {
                        result = &mut phone_receiver => matches!(result, Ok(true)),
                        result = &mut pc => matches!(result, Ok(true)),
                    }
                }
                None => matches!(phone_receiver.await, Ok(true)),
            }
        },
    )
    .await;
    let approved = matches!(decided, Ok(true));

    state
        .pending_confirms
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(call_id);
    approved
}

async fn authenticate(socket: &mut WebSocket, state: &AppState) -> Result<crate::store::Device> {
    let envelope = loop {
        match socket.recv().await {
            Some(Ok(WsMessage::Text(text))) => break serde_json::from_str::<Envelope>(&text)?,
            Some(Ok(WsMessage::Close(_))) | None => bail!("连接在认证前关闭"),
            Some(Err(error)) => return Err(error.into()),
            _ => {}
        }
    };
    protocol::validate(&envelope)?;
    if envelope.envelope_type != Type::Auth {
        bail!("首条消息必须是 auth");
    }
    let auth: Auth = protocol::payload(&envelope)?;
    let device = state
        .store
        .authenticate(&auth.device_id, &hash_token(&auth.token))?
        .ok_or_else(|| anyhow::anyhow!("设备或 token 无效，请重新配对"))?;
    if !state
        .store
        .remember_nonce(&device.id, &envelope.nonce, envelope.ts)?
    {
        bail!("重复的认证 nonce");
    }
    Ok(device)
}

async fn run_chat(
    state: AppState,
    outbound: mpsc::UnboundedSender<Envelope>,
    request_id: String,
    payload: ChatSend,
) {
    let session_id = payload.session_id.unwrap_or_else(|| "default".to_owned());
    let content = payload.content.trim();
    if content.is_empty() || content.chars().count() > 32_000 {
        let _ = outbound.send(error_envelope(
            "消息需为 1 到 32000 个字符",
            Some(request_id),
        ));
        return;
    }
    if let Some(problem) = state.config.llm_problem() {
        let _ = outbound.send(error_envelope(problem, Some(request_id)));
        return;
    }
    let client = match LlmClient::new(state.config.llm.default.clone()) {
        Ok(client) => client,
        Err(error) => {
            let _ = outbound.send(error_envelope(
                format!("模型配置无效：{error:#}"),
                Some(request_id),
            ));
            return;
        }
    };
    let history = match state.store.history(&session_id, 50) {
        Ok(history) => history,
        Err(error) => {
            let _ = outbound.send(error_envelope(
                format!("读取会话失败：{error:#}"),
                Some(request_id),
            ));
            return;
        }
    };
    if let Err(error) = state.store.append_message(&session_id, "user", content) {
        let _ = outbound.send(error_envelope(
            format!("保存消息失败：{error:#}"),
            Some(request_id),
        ));
        return;
    }
    let assistant = match state.store.append_message(&session_id, "assistant", "") {
        Ok(message) => message,
        Err(error) => {
            let _ = outbound.send(error_envelope(
                format!("创建回答失败：{error:#}"),
                Some(request_id),
            ));
            return;
        }
    };
    let message_id = assistant.id.clone();

    let mut messages = LlmClient::history_messages(history);
    messages.push(RequestMessage::text("user", content));

    // 多轮循环：模型可能先要求调用工具，拿到结果后再组织最终回答。
    // 每轮的文本增量都落到同一个气泡里，避免回答中途"消失重来"。
    let mut accumulated = String::new();
    let mut rounds = 0_usize;
    loop {
        rounds += 1;
        let delta_session = session_id.clone();
        let delta_message = message_id.clone();
        let delta_outbound = outbound.clone();
        let outcome = match client
            .stream_chat(
                messages.clone(),
                crate::core::tools::specs(),
                move |delta| {
                    if let Ok(event) = new_envelope(
                        Type::ChatDelta,
                        ChatDelta {
                            session_id: delta_session.clone(),
                            message_id: delta_message.clone(),
                            delta,
                        },
                    ) {
                        let _ = delta_outbound.send(event);
                    }
                },
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                let message = format!("模型请求失败：{error:#}");
                let _ = outbound.send(error_envelope(message.clone(), Some(request_id.clone())));
                accumulated.push_str(&message);
                break;
            }
        };

        accumulated.push_str(&outcome.content);

        if outcome.tool_calls.is_empty() {
            break;
        }
        if rounds > MAX_TOOL_ROUNDS {
            accumulated.push_str("\n\n（已达到工具调用上限，先停在这里）");
            break;
        }

        info!(
            "模型请求调用工具：{}",
            outcome
                .tool_calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        messages.push(RequestMessage::assistant_with_tool_calls(
            &outcome.tool_calls,
        ));
        for call in &outcome.tool_calls {
            let result = match crate::core::tools::risk_of(&call.name) {
                // 被策略禁用的工具必须明确回给模型，否则它会反复重试同一个调用。
                Risk::Blocked => format!("工具「{}」不在允许列表内，不可调用。", call.name),
                Risk::High => {
                    let summary = crate::core::tools::summary_of(&call.name, &call.arguments);
                    if wait_for_approval(&state, &call.id, &call.name, &summary).await {
                        run_tool(&state, &call.name, &call.arguments)
                    } else {
                        "用户拒绝了该操作（或确认超时），已取消。".to_owned()
                    }
                }
                Risk::Medium | Risk::Low => run_tool(&state, &call.name, &call.arguments),
            };
            messages.push(RequestMessage::tool_result(&call.id, result));
        }
    }

    if let Err(error) = state
        .store
        .update_message_content(&message_id, &accumulated)
    {
        let _ = outbound.send(error_envelope(
            format!("保存回答失败：{error:#}"),
            Some(request_id),
        ));
        return;
    }
    if let Ok(event) = new_envelope(
        Type::ChatDone,
        ChatDone {
            session_id,
            message_id,
            content: accumulated,
        },
    ) {
        let _ = outbound.send(event);
    }
}

async fn send_envelope(socket: &mut WebSocket, envelope: Envelope) -> Result<()> {
    let text = serde_json::to_string(&envelope)?;
    socket.send(WsMessage::Text(text.into())).await?;
    Ok(())
}

impl From<Message> for crate::contract::Protocol {
    fn from(message: Message) -> Self {
        Self {
            id: message.id,
            session_id: message.session_id,
            role: match message.role.as_str() {
                "assistant" => crate::contract::Role::Assistant,
                "system" => crate::contract::Role::System,
                _ => crate::contract::Role::User,
            },
            content: message.content,
            created_at: message.created_at,
        }
    }
}

fn local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

const PAIRING_PAGE: &str = r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>PomBot 配对</title><style>
body{margin:0;background:#f5f1e8;color:#23362f;font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh}
main{background:#fffdf8;border-radius:28px;padding:32px;max-width:440px;box-shadow:0 18px 60px #23362f18;text-align:center}
img{width:min(320px,80vw);height:auto}code{font-size:28px;letter-spacing:8px}.word{font-size:22px;font-weight:700;color:#547665;background:#eef3ed;padding:16px;border-radius:14px}
small{color:#65736c;line-height:1.6}button{border:0;background:#547665;color:white;padding:12px 24px;border-radius:12px;font-size:16px}
.ok{background:#e7f0e7;color:#2f5644;border-radius:14px;padding:14px;margin:14px 0;font-weight:600;line-height:1.5}
</style></head><body><main><h1>把绒球装进口袋</h1><p>使用 PomBot Android 扫描</p>
<img src="/pairing/qr.svg" alt="配对二维码"><p>六位配对码</p><code id="code">------</code>
<div id="pending" hidden><p>请核对手机与这里显示的短词</p><div class="word" id="word"></div><p id="device"></p></div>
<div id="paired" class="ok" hidden></div>
<p><small>二维码每 5 分钟更新。请在同一 WiFi 下操作，不要把配对码发给其他人。</small></p>
</main><script>
let currentCode='';
async function refresh(){try{const r=await fetch('/pairing/status');const s=await r.json();const changed=currentCode&&currentCode!==s.code;currentCode=s.code;document.getElementById('code').textContent=s.code;
if(s.pending){document.getElementById('pending').hidden=false;document.getElementById('word').textContent=s.pending.short_word;document.getElementById('device').textContent='设备：'+s.pending.device_name;}
if(s.paired_device){var box=document.getElementById('paired');box.hidden=false;box.textContent='✅ 已与「'+s.paired_device+'」配对成功，这个页面可以关掉了。';document.getElementById('pending').hidden=true;}
if(changed||s.expires_in<2){document.querySelector('img').src='/pairing/qr.svg?t='+Date.now();}}catch(e){}}
refresh();setInterval(refresh,2000);
</script></body></html>"#;
