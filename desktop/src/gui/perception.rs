use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;

use crate::{
    core::llm::{ContentPart, ImageUrl, MessageContent, RequestMessage},
    gui::{capture::Shot, ocr::OcrLine},
};

#[derive(Clone, Debug, Serialize)]
pub struct PerceptionPacket {
    pub width: usize,
    pub height: usize,
    pub ocr: Vec<OcrBlock>,
    pub active_window: String,
    pub origin_x: i32,
    pub origin_y: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct OcrBlock {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PerceptionPacket {
    pub fn from_lines(shot: &Shot, lines: &[OcrLine], active_window: impl Into<String>) -> Self {
        Self {
            width: shot.width,
            height: shot.height,
            active_window: active_window.into(),
            origin_x: 0,
            origin_y: 0,
            ocr: lines
                .iter()
                .flat_map(|line| line.words.iter())
                .map(|word| OcrBlock {
                    text: word.text.clone(),
                    x: word.x,
                    y: word.y,
                    width: word.width,
                    height: word.height,
                })
                .collect(),
        }
    }

    pub fn prompt(&self) -> Result<String> {
        serde_json::to_string(self).context("序列化感知包失败")
    }
}

pub fn shot_png_data_url(shot: &Shot) -> Result<String> {
    let image = image::RgbaImage::from_raw(shot.width as u32, shot.height as u32, shot.to_rgba())
        .ok_or_else(|| anyhow::anyhow!("构造感知图像失败"))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .context("编码感知截图失败")?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    ))
}

pub fn vision_message(
    goal: &str,
    packet: &PerceptionPacket,
    shot: &Shot,
) -> Result<RequestMessage> {
    Ok(RequestMessage {
        role: "user".to_owned(),
        content: Some(MessageContent::Parts(vec![
            ContentPart::Text {
                text: format!(
                    "目标：{goal}\n当前感知包：{}\n请只通过 gui_action 工具推进任务。",
                    packet.prompt()?
                ),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: shot_png_data_url(shot)?,
                    detail: Some("high".to_owned()),
                },
            },
        ])),
        tool_calls: None,
        tool_call_id: None,
    })
}
