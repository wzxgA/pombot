// @author wzxgqtl 2026-11-21
// OpenAI 兼容的流式对话客户端，支持 function calling。
//
// 工具调用在流式响应里是按 index 分片到达的：name 只在第一片出现，arguments 会被
// 切成任意多段。因此必须按 index 累积，不能假设一次事件就是一条完整调用。

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{config::LlmProfile, store::Message};

#[derive(Clone)]
pub struct LlmClient {
    profile: LlmProfile,
    client: Client,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<RequestMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDef>,
}

/// 发往模型的一条消息。工具调用需要三种形态：普通文本、assistant 带 tool_calls、
/// 以及回填结果的 tool 消息，因此字段全部可选。
#[derive(Serialize, Clone)]
pub struct RequestMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl RequestMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(MessageContent::Text(content.into())),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// 模型要求调用工具时，必须把这条 assistant 消息原样回灌，否则下一轮上下文不完整。
    pub fn assistant_with_tool_calls(calls: &[ToolCall]) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: None,
            tool_calls: Some(
                calls
                    .iter()
                    .map(|call| ToolCallRequest {
                        id: call.id.clone(),
                        kind: "function",
                        function: FunctionCallRequest {
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        },
                    })
                    .collect(),
            ),
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_owned(),
            content: Some(MessageContent::Text(content.into())),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionCallRequest,
}

#[derive(Serialize, Clone)]
pub struct FunctionCallRequest {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolFunctionDef,
}

#[derive(Serialize)]
pub struct ToolFunctionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize, Clone)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 累积完成后的一次工具调用。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// 一轮请求的结果：既可能有文本，也可能要求调用工具。
#[derive(Default)]
pub struct ChatOutcome {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: u32,
    id: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

impl LlmClient {
    pub fn new(profile: LlmProfile) -> Result<Self> {
        if profile.base_url.trim().is_empty() || profile.model.trim().is_empty() {
            bail!("模型 base_url 与 model 不能为空");
        }
        let url =
            reqwest::Url::parse(profile.base_url.trim()).context("模型 base_url 不是有效 URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("模型 base_url 仅支持 http 或 https");
        }
        Ok(Self {
            profile,
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .context("创建模型 HTTP 客户端失败")?,
        })
    }

    /// 把库里的历史消息转成请求消息。历史里只有纯文本，工具轮次不落库。
    pub fn history_messages(history: Vec<Message>) -> Vec<RequestMessage> {
        history
            .into_iter()
            .map(|message| RequestMessage::text(message.role, message.content))
            .collect()
    }

    pub async fn stream_chat<F>(
        &self,
        messages: Vec<RequestMessage>,
        tools: Vec<ToolDef>,
        mut on_delta: F,
    ) -> Result<ChatOutcome>
    where
        F: FnMut(String),
    {
        let request = ChatRequest {
            model: self.profile.model.clone(),
            messages,
            stream: true,
            tools,
        };
        let endpoint = format!(
            "{}/chat/completions",
            self.profile.base_url.trim_end_matches('/')
        );
        let mut builder = self.client.post(endpoint).json(&request);
        if !self.profile.api_key.trim().is_empty() {
            builder = builder.bearer_auth(self.profile.api_key.trim());
        }
        let response = builder.send().await.context("无法连接模型服务")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let excerpt: String = body.chars().take(500).collect();
            bail!("模型服务返回 HTTP {status}：{excerpt}");
        }

        let mut decoder = SseDecoder::default();
        let mut outcome = ChatOutcome::default();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取模型流失败")?;
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    outcome.tool_calls = calls;
                    return Ok(outcome);
                }
                // 某些网关会插入空 data 或心跳，解析失败不该中断整条流。
                let Ok(event) = serde_json::from_str::<StreamResponse>(&data) else {
                    tracing::debug!("跳过无法解析的流事件：{}", excerpt(&data));
                    continue;
                };
                for choice in event.choices {
                    if let Some(content) = choice.delta.content
                        && !content.is_empty()
                    {
                        outcome.content.push_str(&content);
                        on_delta(content);
                    }
                    for delta in choice.delta.tool_calls.unwrap_or_default() {
                        merge_tool_call(&mut calls, delta);
                    }
                }
            }
        }
        outcome.tool_calls = calls;
        Ok(outcome)
    }
}

/// 按 index 合并分片。`name` 与 `id` 通常只在第一片出现，`arguments` 会被切碎。
fn merge_tool_call(calls: &mut Vec<ToolCall>, delta: ToolCallDelta) {
    let index = delta.index as usize;
    while calls.len() <= index {
        calls.push(ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
    }
    let slot = &mut calls[index];
    if let Some(id) = delta.id
        && !id.is_empty()
    {
        slot.id = id;
    }
    if let Some(function) = delta.function {
        if let Some(name) = function.name
            && !name.is_empty()
        {
            slot.name = name;
        }
        if let Some(arguments) = function.arguments {
            slot.arguments.push_str(&arguments);
        }
    }
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            let line = std::str::from_utf8(&line)
                .context("模型 SSE 包含无效 UTF-8")?
                .trim();
            if let Some(data) = line.strip_prefix("data:") {
                events.push(data.trim().to_owned());
            }
        }
        Ok(events)
    }
}

fn excerpt(text: &str) -> String {
    text.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_sse_lines_and_multibyte_content() {
        let mut decoder = SseDecoder::default();
        let bytes = "event: message\ndata: {\"text\":\"你\"}\r\n\r\n".as_bytes();
        let split = bytes
            .windows(3)
            .position(|window| window == "你".as_bytes())
            .expect("找到中文")
            + 1;
        assert!(decoder.push(&bytes[..split]).expect("首段").is_empty());
        let events = decoder.push(&bytes[split..]).expect("完整事件");
        assert_eq!(events, vec!["{\"text\":\"你\"}"]);
    }

    #[test]
    fn request_supports_reserved_image_content() {
        let content = MessageContent::Parts(vec![
            ContentPart::Text {
                text: "看图".to_owned(),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/png;base64,AA==".to_owned(),
                    detail: Some("high".to_owned()),
                },
            },
        ]);
        let json = serde_json::to_value(content).expect("序列化多模态内容");
        assert_eq!(json[1]["type"], "image_url");
        assert_eq!(json[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn merges_fragmented_tool_call_deltas() {
        // 真实响应里 name 只在第一片出现，arguments 会被切成多段。
        let mut calls = Vec::new();
        merge_tool_call(
            &mut calls,
            ToolCallDelta {
                index: 0,
                id: Some("call_1".to_owned()),
                function: Some(FunctionDelta {
                    name: Some("create_text_file".to_owned()),
                    arguments: Some("{".to_owned()),
                }),
            },
        );
        merge_tool_call(
            &mut calls,
            ToolCallDelta {
                index: 0,
                id: None,
                function: Some(FunctionDelta {
                    name: None,
                    arguments: Some("\"name\":\"周报.md\"}".to_owned()),
                }),
            },
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "create_text_file");
        assert_eq!(calls[0].arguments, "{\"name\":\"周报.md\"}");
    }

    #[test]
    fn tool_result_message_uses_tool_role_and_call_id() {
        let json = serde_json::to_value(RequestMessage::tool_result("call_1", "完成"))
            .expect("序列化 tool 消息");
        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_call_id"], "call_1");
        assert!(json.get("tool_calls").is_none());
    }
}
