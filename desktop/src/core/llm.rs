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
}

#[derive(Serialize)]
struct RequestMessage {
    role: String,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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

    pub async fn stream_chat<F>(&self, history: Vec<Message>, mut on_delta: F) -> Result<String>
    where
        F: FnMut(String),
    {
        let messages = history
            .into_iter()
            .map(|message| RequestMessage {
                role: message.role,
                content: MessageContent::Text(message.content),
            })
            .collect();
        let request = ChatRequest {
            model: self.profile.model.clone(),
            messages,
            stream: true,
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
        let mut full = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取模型流失败")?;
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    return Ok(full);
                }
                let event: StreamResponse = serde_json::from_str(&data)
                    .with_context(|| format!("无法解析模型流事件：{}", excerpt(&data)))?;
                for choice in event.choices {
                    if let Some(content) = choice.delta.content
                        && !content.is_empty()
                    {
                        full.push_str(&content);
                        on_delta(content);
                    }
                }
            }
        }
        Ok(full)
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
}
