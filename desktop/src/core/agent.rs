use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::future::Future;

use crate::gui::executor::Action;

#[derive(Clone, Debug, Serialize)]
pub struct AgentObservation {
    pub step: usize,
    pub goal: String,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VisionDecision {
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    Started,
    Observed { step: usize },
    ActionBatch { step: usize, count: usize },
    Finished { steps: usize },
    Cancelled { step: usize },
}

#[derive(Clone, Debug)]
pub struct AgentLimits {
    pub max_steps: usize,
    pub max_seconds: u64,
    pub max_empty_decisions: usize,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_steps: 30,
            max_seconds: 600,
            max_empty_decisions: 2,
        }
    }
}

pub trait AgentDriver {
    fn observe(&mut self, observation: &AgentObservation) -> Result<String>;
    fn decide(
        &mut self,
        observation: &AgentObservation,
        perception: &str,
    ) -> Result<VisionDecision>;
    fn execute(&mut self, actions: &[Action]) -> Result<()>;
}

pub trait AsyncAgentDriver {
    type ObserveFuture<'a>: Future<Output = Result<String>> + 'a
    where
        Self: 'a;
    type DecideFuture<'a>: Future<Output = Result<VisionDecision>> + 'a
    where
        Self: 'a;
    type ExecuteFuture<'a>: Future<Output = Result<()>> + 'a
    where
        Self: 'a;

    fn observe_async<'a>(
        &'a mut self,
        observation: &'a AgentObservation,
    ) -> Self::ObserveFuture<'a>;
    fn decide_async<'a>(
        &'a mut self,
        observation: &'a AgentObservation,
        perception: &'a str,
    ) -> Self::DecideFuture<'a>;
    fn execute_async<'a>(&'a mut self, actions: &'a [Action]) -> Self::ExecuteFuture<'a>;
}

pub struct AgentLoop<D> {
    driver: D,
    limits: AgentLimits,
    cancel: Arc<AtomicBool>,
}

impl<D: AgentDriver> AgentLoop<D> {
    pub fn new(driver: D) -> Self {
        Self::new_with_cancellation(driver, Arc::new(AtomicBool::new(false)))
    }

    pub fn new_with_cancellation(driver: D, cancel: Arc<AtomicBool>) -> Self {
        Self {
            driver,
            limits: AgentLimits::default(),
            cancel,
        }
    }

    pub fn with_limits(mut self, limits: AgentLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn cancellation_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn into_driver(self) -> D {
        self.driver
    }

    pub async fn run_async<F>(&mut self, goal: impl Into<String>, mut on_event: F) -> Result<String>
    where
        D: AsyncAgentDriver,
        F: FnMut(AgentEvent),
    {
        let goal = goal.into();
        let started = Instant::now();
        let mut empty = 0;
        on_event(AgentEvent::Started);
        for step in 0..self.limits.max_steps {
            if self.cancel.load(Ordering::Acquire) {
                on_event(AgentEvent::Cancelled { step });
                bail!("视觉任务已取消（第 {} 步）", step + 1);
            }
            if started.elapsed() > Duration::from_secs(self.limits.max_seconds) {
                bail!("视觉任务超时（{} 秒）", self.limits.max_seconds);
            }
            let observation = AgentObservation {
                step,
                goal: goal.clone(),
                summary: String::new(),
            };
            let perception = self
                .driver
                .observe_async(&observation)
                .await
                .context("构造视觉感知包失败")?;
            on_event(AgentEvent::Observed { step });
            let decision = self
                .driver
                .decide_async(&observation, &perception)
                .await
                .context("视觉模型决策失败")?;
            if decision.done {
                on_event(AgentEvent::Finished { steps: step + 1 });
                return Ok(decision.message);
            }
            if decision.actions.is_empty() {
                empty += 1;
                if empty > self.limits.max_empty_decisions {
                    bail!("视觉模型连续返回空动作，任务未推进");
                }
                continue;
            }
            empty = 0;
            let count = decision.actions.len();
            on_event(AgentEvent::ActionBatch { step, count });
            self.driver
                .execute_async(&decision.actions)
                .await
                .context("执行视觉动作失败")?;
        }
        bail!("视觉任务超过最大步数 {}", self.limits.max_steps)
    }

    pub fn run<F>(&mut self, goal: impl Into<String>, mut on_event: F) -> Result<String>
    where
        F: FnMut(AgentEvent),
    {
        let goal = goal.into();
        let started = Instant::now();
        let mut empty = 0;
        on_event(AgentEvent::Started);

        for step in 0..self.limits.max_steps {
            if self.cancel.load(Ordering::Acquire) {
                on_event(AgentEvent::Cancelled { step });
                bail!("视觉任务已取消（第 {} 步）", step + 1);
            }
            if started.elapsed() > Duration::from_secs(self.limits.max_seconds) {
                bail!("视觉任务超时（{} 秒）", self.limits.max_seconds);
            }
            let observation = AgentObservation {
                step,
                goal: goal.clone(),
                summary: String::new(),
            };
            let perception = self
                .driver
                .observe(&observation)
                .context("构造视觉感知包失败")?;
            on_event(AgentEvent::Observed { step });
            let decision = self
                .driver
                .decide(&observation, &perception)
                .context("视觉模型决策失败")?;
            if decision.done {
                on_event(AgentEvent::Finished { steps: step + 1 });
                return Ok(decision.message);
            }
            if decision.actions.is_empty() {
                empty += 1;
                if empty > self.limits.max_empty_decisions {
                    bail!("视觉模型连续返回空动作，任务未推进");
                }
                continue;
            }
            empty = 0;
            let count = decision.actions.len();
            on_event(AgentEvent::ActionBatch { step, count });
            self.driver
                .execute(&decision.actions)
                .context("执行视觉动作失败")?;
        }
        bail!("视觉任务超过最大步数 {}", self.limits.max_steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        decisions: Vec<VisionDecision>,
        executed: usize,
    }
    impl AgentDriver for Fake {
        fn observe(&mut self, o: &AgentObservation) -> Result<String> {
            Ok(format!("obs:{}", o.step))
        }
        fn decide(&mut self, _: &AgentObservation, _: &str) -> Result<VisionDecision> {
            Ok(self.decisions.remove(0))
        }
        fn execute(&mut self, actions: &[Action]) -> Result<()> {
            self.executed += actions.len();
            Ok(())
        }
    }

    #[test]
    fn loops_until_done_and_reports_progress() {
        let fake = Fake {
            decisions: vec![
                VisionDecision {
                    actions: vec![Action::Wait { ms: 1 }],
                    done: false,
                    message: String::new(),
                },
                VisionDecision {
                    actions: vec![],
                    done: true,
                    message: "完成".into(),
                },
            ],
            executed: 0,
        };
        let mut agent = AgentLoop::new(fake);
        let mut events = Vec::new();
        let result = agent
            .run("测试", |event| events.push(event))
            .expect("应完成");
        assert_eq!(result, "完成");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::ActionBatch { count: 1, .. }))
        );
        assert_eq!(agent.into_driver().executed, 1);
    }

    #[test]
    fn cancels_before_next_step() {
        let fake = Fake {
            decisions: vec![],
            executed: 0,
        };
        let mut agent = AgentLoop::new(fake);
        agent.cancel();
        assert!(agent.run("测试", |_| {}).is_err());
    }
}
