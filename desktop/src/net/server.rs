use std::{net::UdpSocket, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use qrcode::QrCode;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc};
use tracing::{info, warn};
use url::Url;

use crate::{
    config::{AppConfig, AppPaths},
    contract::{
        Auth, AuthOk, ChatDelta, ChatDone, ChatHistory, ChatHistoryResult, ChatSend, Envelope,
        PairChallenge, PairConfirm, PairResult, PairStart, Type,
    },
    core::llm::LlmClient,
    net::{
        discovery,
        pairing::{PairingManager, hash_token},
        protocol::{self, error_envelope, new_envelope},
    },
    store::{Message, Store},
};

#[derive(Clone)]
struct AppState {
    config: AppConfig,
    store: Arc<Store>,
    pairing: Arc<PairingManager>,
    local_url: String,
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
    };
    let app = Router::new()
        .route("/health", get(health))
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
                device_name: device.name,
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
            _ => {
                let _ = outbound.send(error_envelope("不支持的消息类型", Some(request_id)));
            }
        }
    }
    writer.abort();
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
    let mut history = match state.store.history(&session_id, 50) {
        Ok(history) => history,
        Err(error) => {
            let _ = outbound.send(error_envelope(
                format!("读取会话失败：{error:#}"),
                Some(request_id),
            ));
            return;
        }
    };
    let user = match state.store.append_message(&session_id, "user", content) {
        Ok(message) => message,
        Err(error) => {
            let _ = outbound.send(error_envelope(
                format!("保存消息失败：{error:#}"),
                Some(request_id),
            ));
            return;
        }
    };
    history.push(user);
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
    let delta_session = session_id.clone();
    let delta_message = message_id.clone();
    let delta_outbound = outbound.clone();
    let result = client
        .stream_chat(history, move |delta| {
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
        })
        .await;
    let content = match result {
        Ok(content) => content,
        Err(error) => {
            let message = format!("模型请求失败：{error:#}");
            let _ = outbound.send(error_envelope(message.clone(), Some(request_id.clone())));
            message
        }
    };
    if let Err(error) = state.store.update_message_content(&message_id, &content) {
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
            content,
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
</style></head><body><main><h1>把绒球装进口袋</h1><p>使用 PomBot Android 扫描</p>
<img src="/pairing/qr.svg" alt="配对二维码"><p>六位配对码</p><code id="code">------</code>
<div id="pending" hidden><p>请核对手机与这里显示的短词</p><div class="word" id="word"></div><p id="device"></p></div>
<p><small>二维码每 5 分钟更新。请在同一 WiFi 下操作，不要把配对码发给其他人。</small></p>
</main><script>
let currentCode='';
async function refresh(){try{const r=await fetch('/pairing/status');const s=await r.json();const changed=currentCode&&currentCode!==s.code;currentCode=s.code;document.getElementById('code').textContent=s.code;
if(s.pending){document.getElementById('pending').hidden=false;document.getElementById('word').textContent=s.pending.short_word;document.getElementById('device').textContent='设备：'+s.pending.device_name;}
if(changed||s.expires_in<2){document.querySelector('img').src='/pairing/qr.svg?t='+Date.now();}}catch(e){}}
refresh();setInterval(refresh,2000);
</script></body></html>"#;
