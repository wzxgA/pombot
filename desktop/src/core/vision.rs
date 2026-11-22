use std::sync::Arc;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::HWND;

use crate::{
    core::{
        agent::{AgentDriver, AgentObservation, AsyncAgentDriver, VisionDecision},
        llm::{LlmClient, RequestMessage},
    },
    gui::{
        capture::capture_window_scene,
        executor::{Action, Executor},
        ocr::recognize_shot_lines,
        perception::{PerceptionPacket, vision_message},
    },
};

const OCR_SCALE: f32 = 2.0;

pub struct VisionDriver {
    hwnd: HWND,
    client: LlmClient,
    goal: String,
    messages: Vec<RequestMessage>,
    executor: Executor,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl VisionDriver {
    pub fn new(hwnd: HWND, client: LlmClient, goal: impl Into<String>) -> Self {
        Self {
            hwnd,
            client,
            goal: goal.into(),
            messages: Vec::new(),
            executor: Executor::new(hwnd),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancellation_token(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.cancel)
    }

    fn observe_packet(&self) -> Result<(PerceptionPacket, crate::gui::capture::Shot)> {
        let scene = capture_window_scene(self.hwnd).map_err(anyhow::Error::msg)?;
        let lines = recognize_shot_lines(&scene.shot, OCR_SCALE).map_err(anyhow::Error::msg)?;
        let mut packet =
            PerceptionPacket::from_lines(&scene.shot, &lines, format!("hwnd={:?}", self.hwnd.0));
        packet.origin_x = scene.origin.0;
        packet.origin_y = scene.origin.1;
        Ok((packet, scene.shot))
    }

    fn parse_decision(raw: &str) -> Result<VisionDecision> {
        let trimmed = raw
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        serde_json::from_str(trimmed).context("视觉模型未返回合法动作 JSON")
    }
}

impl AgentDriver for VisionDriver {
    fn observe(&mut self, _: &AgentObservation) -> Result<String> {
        let (packet, _) = self.observe_packet()?;
        packet.prompt()
    }
    fn decide(&mut self, _: &AgentObservation, _: &str) -> Result<VisionDecision> {
        bail!("VisionDriver 需要异步模型接口")
    }
    fn execute(&mut self, actions: &[Action]) -> Result<()> {
        self.executor
            .run(actions)
            .map(|_| ())
            .context("执行 GUI 动作失败")
    }
}

impl AsyncAgentDriver for VisionDriver {
    type ObserveFuture<'a>
        = std::future::Ready<Result<String>>
    where
        Self: 'a;
    type DecideFuture<'a>
        = std::pin::Pin<Box<dyn std::future::Future<Output = Result<VisionDecision>> + 'a>>
    where
        Self: 'a;
    type ExecuteFuture<'a>
        = std::future::Ready<Result<()>>
    where
        Self: 'a;

    fn observe_async<'a>(&'a mut self, _: &'a AgentObservation) -> Self::ObserveFuture<'a> {
        std::future::ready(
            self.observe_packet()
                .and_then(|(packet, _)| packet.prompt()),
        )
    }

    fn decide_async<'a>(
        &'a mut self,
        observation: &'a AgentObservation,
        _: &'a str,
    ) -> Self::DecideFuture<'a> {
        Box::pin(async move {
            let (_, shot) = self.observe_packet()?;
            let packet = {
                let lines = recognize_shot_lines(&shot, OCR_SCALE).map_err(anyhow::Error::msg)?;
                PerceptionPacket::from_lines(&shot, &lines, format!("hwnd={:?}", self.hwnd.0))
            };
            self.messages
                .push(vision_message(&self.goal, &packet, &shot)?);
            let mut text = String::new();
            let outcome = self
                .client
                .stream_chat(self.messages.clone(), Vec::new(), |delta| {
                    text.push_str(&delta)
                })
                .await?;
            if text.is_empty() {
                text = outcome.content;
            }
            let decision = Self::parse_decision(&text)?;
            self.messages.push(RequestMessage::text("assistant", text));
            let _ = observation;
            Ok(decision)
        })
    }

    fn execute_async<'a>(&'a mut self, actions: &'a [Action]) -> Self::ExecuteFuture<'a> {
        std::future::ready(self.execute(actions))
    }
}
