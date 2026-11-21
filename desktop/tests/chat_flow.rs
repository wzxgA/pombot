use std::{net::SocketAddr, time::Duration};

use axum::{Router, routing::post};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use pombot::{
    config::{AppConfig, AppPaths, LlmConfig, LlmProfile, ServerConfig},
    contract::{ChatDone, ChatSend, Envelope, PairResult, Type},
    net::{protocol::new_envelope, server::serve},
    store::Store,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[tokio::test(flavor = "multi_thread")]
async fn phone_pairs_authenticates_and_receives_streamed_answer() {
    let (mock_address, mock_server) = mock_llm().await;
    let port = unused_port().await;
    let data_dir = std::env::temp_dir().join(format!("pombot-flow-{}", Uuid::new_v4()));
    let paths = AppPaths {
        database_file: data_dir.join("pombot.db"),
        config_file: data_dir.join("config.toml"),
        data_dir: data_dir.clone(),
    };
    let config = AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".to_owned(),
            port,
            public_url: None,
        },
        cloudflare: Default::default(),
        llm: LlmConfig {
            default: LlmProfile {
                base_url: format!("http://{mock_address}/v1"),
                api_key: "test-key".to_owned(),
                model: "test-model".to_owned(),
            },
        },
    };
    let app_server = tokio::spawn(serve(config, paths.clone()));
    wait_until_ready(port).await;

    let status: serde_json::Value = reqwest::get(format!("http://127.0.0.1:{port}/pairing/status"))
        .await
        .expect("读取配对状态")
        .json()
        .await
        .expect("解析配对状态");
    let code = status["code"].as_str().expect("配对码").to_owned();
    let qr = reqwest::get(format!("http://127.0.0.1:{port}/pairing/qr.svg"))
        .await
        .expect("读取配对二维码")
        .text()
        .await
        .expect("解析二维码 SVG");
    assert!(qr.starts_with("<?xml") && qr.contains("<svg") && qr.ends_with("</svg>"));
    let device_id = Uuid::new_v4().to_string();
    let pair_url = format!("ws://127.0.0.1:{port}/ws/pair");
    let (mut pairing, _) = connect_async(pair_url).await.expect("连接配对服务");
    send(
        &mut pairing,
        new_envelope(
            Type::PairStart,
            serde_json::json!({
                "code": code,
                "device_id": device_id,
                "device_name": "测试手机",
                "public_key": URL_SAFE_NO_PAD.encode([9_u8; 32]),
            }),
        )
        .expect("配对消息"),
    )
    .await;
    let challenge = receive(&mut pairing).await;
    assert_eq!(challenge.envelope_type, Type::PairChallenge);
    let pair_id = challenge.payload.clone().expect("payload")["pair_id"]
        .as_str()
        .expect("pair_id")
        .to_owned();
    send(
        &mut pairing,
        new_envelope(Type::PairConfirm, serde_json::json!({"pair_id": pair_id})).expect("确认消息"),
    )
    .await;
    let result = receive(&mut pairing).await;
    let result: PairResult =
        serde_json::from_value(result.payload.expect("配对结果")).expect("配对结果字段");

    let (mut chat, _) = connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .expect("连接对话服务");
    send(
        &mut chat,
        new_envelope(
            Type::Auth,
            serde_json::json!({"device_id": result.device_id.clone(), "token": result.token.clone()}),
        )
        .expect("认证消息"),
    )
    .await;
    assert_eq!(receive(&mut chat).await.envelope_type, Type::AuthOk);
    let request = new_envelope(
        Type::ChatSend,
        ChatSend {
            session_id: Some("default".to_owned()),
            content: "你好".to_owned(),
        },
    )
    .expect("对话消息");
    send(&mut chat, request.clone()).await;

    let mut deltas = String::new();
    let done = loop {
        let envelope = receive(&mut chat).await;
        match envelope.envelope_type {
            Type::ChatDelta => {
                deltas.push_str(
                    envelope.payload.expect("delta payload")["delta"]
                        .as_str()
                        .expect("delta"),
                );
            }
            Type::ChatDone => {
                break serde_json::from_value::<ChatDone>(envelope.payload.expect("done payload"))
                    .expect("done 字段");
            }
            Type::Error => panic!("对话失败：{}", envelope.payload.expect("error payload")),
            other => panic!("意外消息：{other:?}"),
        }
    };
    assert_eq!(deltas, "你好，Pom！");
    assert_eq!(done.content, "你好，Pom！");

    send(&mut chat, request).await;
    let replay = receive(&mut chat).await;
    assert_eq!(replay.envelope_type, Type::Error);
    assert!(
        replay.payload.expect("错误")["message"]
            .as_str()
            .expect("错误文本")
            .contains("nonce")
    );

    chat.close(None).await.expect("关闭首次连接");
    let (mut reconnected, _) = connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .expect("重新连接对话服务");
    send(
        &mut reconnected,
        new_envelope(
            Type::Auth,
            serde_json::json!({"device_id": result.device_id, "token": result.token}),
        )
        .expect("重连认证消息"),
    )
    .await;
    assert_eq!(receive(&mut reconnected).await.envelope_type, Type::AuthOk);
    send(
        &mut reconnected,
        new_envelope(
            Type::ChatHistory,
            serde_json::json!({"session_id": "default", "limit": 100}),
        )
        .expect("历史请求"),
    )
    .await;
    let restored = receive(&mut reconnected).await;
    assert_eq!(restored.envelope_type, Type::ChatHistoryResult);
    assert_eq!(
        restored.payload.expect("历史 payload")["messages"]
            .as_array()
            .expect("历史数组")
            .len(),
        2
    );

    let store = Store::open(&paths.database_file).expect("打开结果数据库");
    let history = store.history("default", 10).expect("消息历史");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "你好");
    assert_eq!(history[1].content, "你好，Pom！");
    drop(store);
    app_server.abort();
    mock_server.abort();
    let _ = app_server.await;
    let _ = mock_server.await;
    let _ = std::fs::remove_dir_all(data_dir);
}

async fn mock_llm() -> (SocketAddr, JoinHandle<Result<(), std::io::Error>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("模拟模型端口");
    let address = listener.local_addr().expect("模拟模型地址");
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好，\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Pom！\"}}]}\n\ndata: [DONE]\n\n"
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    (address, server)
}

async fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .expect("空闲端口")
        .local_addr()
        .expect("端口地址")
        .port()
}

async fn wait_until_ready(port: u16) {
    for _ in 0..50 {
        if reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("服务未按时启动");
}

async fn send(socket: &mut Socket, envelope: Envelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).expect("序列化消息").into(),
        ))
        .await
        .expect("发送 WebSocket 消息");
}

async fn receive(socket: &mut Socket) -> Envelope {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).expect("解析服务端消息");
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => panic!("WebSocket 错误：{error}"),
            None => panic!("WebSocket 意外关闭"),
        }
    }
}
