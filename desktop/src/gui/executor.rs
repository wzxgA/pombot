// @author wzxgqtl 2026-11-22
// 动作序列执行器：把一段 JSON 脚本跑成真实操作。
//
// 每一步都是同一个骨架：观察 → 定位 → 执行 → 再观察 → 判定。
// 之所以强制"每步都验证"，是因为 GUI 自动化最致命的失败是静默无效：
// 点偏几像素、界面没变，流程却继续走，最后拿到莫名其妙的结果。
//
// 三条硬上限（步数 / 时长 / 重复落点）用于兜住死循环：
// 没有它们，一个点不中的按钮会让程序安静地转十分钟。

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_LEFT, VK_MENU};

use crate::gui::{
    capture::capture_window_scene,
    input::{
        click_at, ctrl_key, double_click_at, drag, move_to, right_click_at, scroll_at, tap_key,
        type_text,
    },
    locate::find_best_occurrence,
    ocr::recognize_shot_lines,
    verify::{Observation, Stagnation, compare},
};

/// 识别时统一放大 2 倍。小字号是这个引擎最弱的地方，放大是最廉价的补偿。
const OCR_SCALE: f32 = 2.0;
/// 动作执行后等画面稳定再截图。太短会截到过渡帧，误判成"没变化"。
const SETTLE_MS: u64 = 250;

/// 一个动作的目标：给文字走 OCR 定位，给坐标直接点。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Target {
    /// 目标文字。OCR 会说"点『导出』"，精确落点由包围盒决定。
    pub text: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// 同一文字出现多次时取第几个，从 0 开始。
    #[serde(default)]
    pub occurrence: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Click {
        target: Target,
    },
    DoubleClick {
        target: Target,
    },
    RightClick {
        target: Target,
    },
    Move {
        target: Target,
    },
    Drag {
        from: Target,
        to: Target,
    },
    Scroll {
        target: Target,
        lines: i32,
    },
    Type {
        text: String,
    },
    /// 组合键：`ctrl+a`、`enter`、`esc`、`tab`、`delete`、`space`。
    Keys {
        combo: String,
    },
    Wait {
        ms: u64,
    },
    /// 断言界面上出现（或消失）某段文字。
    ExpectText {
        text: String,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// 条件等待，避免用固定 sleep 猜测页面加载时间。
    WaitForText {
        text: String,
        #[serde(default = "default_wait_timeout_ms")]
        timeout_ms: u64,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// 处理浏览器原生 alert/confirm。弹窗不属于浏览器客户区，直接发送 Enter 或 Esc。
    Dialog {
        #[serde(default = "default_true")]
        accept: bool,
    },
    /// 保存当前窗口截图，便于成功步骤或失败步骤回归取证。
    Screenshot {
        path: String,
    },
}

fn default_true() -> bool {
    true
}

fn default_wait_timeout_ms() -> u64 {
    10_000
}

/// 运行上限。默认值按"一次正常操作"留足余量，但仍能挡住死循环。
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_steps: usize,
    pub max_seconds: u64,
    /// 同一个屏幕点连续点多少次就判定卡住。
    pub duplicate_clicks: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 40,
            max_seconds: 300,
            duplicate_clicks: 3,
        }
    }
}

/// 一次运行的结论。
#[derive(Clone, Debug)]
pub struct Report {
    pub steps: usize,
    pub elapsed_ms: u128,
    /// 最后一步是否被判定为产生了变化。
    pub last_changed: bool,
}

pub struct Executor {
    hwnd: HWND,
    limits: Limits,
    stagnation: Stagnation,
    click_counts: HashMap<(i32, i32), usize>,
    before: Option<Observation>,
    /// 逐动作打印判定结果。命令行跑脚本时打开，便于看清每一步到底做了什么。
    trace: bool,
    cancel: Arc<AtomicBool>,
}

impl Executor {
    // @author wzxgqtl 2026-11-22
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            limits: Limits::default(),
            stagnation: Stagnation::default(),
            click_counts: HashMap::new(),
            trace: false,
            cancel: Arc::new(AtomicBool::new(false)),
            before: None,
        }
    }

    // @author wzxgqtl 2026-11-22
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// 打开逐动作日志。命令行跑脚本时用，库调用方默认关闭。
    // @author wzxgqtl 2026-11-22
    pub fn set_trace(&mut self, enabled: bool) {
        self.trace = enabled;
    }

    /// 返回可共享的取消句柄，手机中止或全局 Esc 监听可调用它。
    pub fn cancellation_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// 请求当前动作序列尽快停止。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// 按顺序执行动作。任何一步失败或判定卡死都会立刻返回错误，
    /// 不做"继续试下一个动作"的补救——那只会把问题推到更难定位的地方。
    // @author wzxgqtl 2026-11-22
    pub fn run(&mut self, actions: &[Action]) -> Result<Report> {
        if actions.len() > self.limits.max_steps {
            bail!(
                "动作数量 {} 超过上限 {}",
                actions.len(),
                self.limits.max_steps
            );
        }
        let started = Instant::now();
        let mut last_changed = false;

        for (index, action) in actions.iter().enumerate() {
            if self.cancel.load(Ordering::Acquire) {
                bail!("动作序列已被取消（第 {} 步前）", index + 1);
            }
            if started.elapsed() > Duration::from_secs(self.limits.max_seconds) {
                bail!("执行超时：已超过 {} 秒", self.limits.max_seconds);
            }
            let scene = scene_of(self.hwnd)?;
            self.before = Some(Observation::new(scene.shot));

            self.perform(action, scene.origin).map_err(|error| {
                anyhow::anyhow!("第 {} 步（{:?}）失败：{error}", index + 1, action)
            })?;

            if self.sleep_or_cancel(Duration::from_millis(SETTLE_MS)) {
                bail!("动作序列已被取消（第 {} 步后）", index + 1);
            }

            // 等待类动作、纯输入和截图不走变化判定：它们本来就可能不改变画面。
            if matches!(
                action,
                Action::Wait { .. }
                    | Action::Type { .. }
                    | Action::WaitForText { .. }
                    | Action::Dialog { .. }
                    | Action::Screenshot { .. }
            ) {
                continue;
            }

            let after = scene_of(self.hwnd)?;
            let Some(before) = self.before.take() else {
                continue;
            };
            let verdict = compare(&before, &after.shot);
            last_changed = verdict.changed;

            if self.trace {
                eprintln!(
                    "[{}/{}] {:?} → changed={} hamming={} ratio={:.5}",
                    index + 1,
                    actions.len(),
                    action,
                    verdict.changed,
                    verdict.hamming,
                    verdict.ratio
                );
            }

            if self.stagnation.record(verdict.changed) {
                bail!(
                    "连续 {} 步界面没有任何变化，已中止（第 {} 步：{:?}）",
                    self.stagnation.streak(),
                    index + 1,
                    action
                );
            }
        }

        Ok(Report {
            steps: actions.len(),
            elapsed_ms: started.elapsed().as_millis(),
            last_changed,
        })
    }

    fn perform(&mut self, action: &Action, origin: (i32, i32)) -> Result<()> {
        let map = |target: &Target| -> Result<(i32, i32)> { self.resolve(target, origin) };
        match action {
            Action::Click { target } => {
                let point = map(target)?;
                self.note_click(point)?;
                click_at(point.0, point.1).map_err(anyhow::Error::msg)
            }
            Action::DoubleClick { target } => {
                let point = map(target)?;
                double_click_at(point.0, point.1).map_err(anyhow::Error::msg)
            }
            Action::RightClick { target } => {
                let point = map(target)?;
                right_click_at(point.0, point.1).map_err(anyhow::Error::msg)
            }
            Action::Move { target } => {
                let point = map(target)?;
                move_to(point.0, point.1).map_err(anyhow::Error::msg)
            }
            Action::Drag { from, to } => {
                let start = map(from)?;
                let end = map(to)?;
                drag(start, end).map_err(anyhow::Error::msg)
            }
            Action::Scroll { target, lines } => {
                let point = map(target)?;
                scroll_at(point.0, point.1, *lines).map_err(anyhow::Error::msg)
            }
            Action::Type { text } => type_text(text).map_err(anyhow::Error::msg),
            Action::Keys { combo } => press(combo),
            Action::Wait { ms } => {
                if self.sleep_or_cancel(Duration::from_millis(*ms)) {
                    bail!("动作序列已被取消（等待 {} ms 时）", ms);
                }
                Ok(())
            }
            Action::ExpectText { text, present } => self.expect_text(text, *present),
            Action::WaitForText {
                text,
                timeout_ms,
                present,
            } => {
                let deadline = Instant::now() + Duration::from_millis(*timeout_ms);
                loop {
                    if self.text_present(text)? == *present {
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "等待文本「{text}」超时（{} ms，期望 present={present}）",
                            timeout_ms
                        );
                    }
                    if self.sleep_or_cancel(Duration::from_millis(100)) {
                        bail!("动作序列已被取消（等待文本「{text}」时）");
                    }
                }
            }
            Action::Dialog { accept } => {
                if *accept {
                    tap_key(VIRTUAL_KEY(0x0D)).map_err(anyhow::Error::msg)
                } else {
                    tap_key(VIRTUAL_KEY(0x1B)).map_err(anyhow::Error::msg)
                }
            }
            Action::Screenshot { path } => {
                let scene = scene_of(self.hwnd)?;
                scene
                    .shot
                    .save_png(Path::new(path))
                    .map_err(anyhow::Error::msg)
            }
        }
    }

    fn sleep_or_cancel(&self, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return true;
            }
            std::thread::sleep(
                Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        self.cancel.load(Ordering::Acquire)
    }

    fn expect_text(&self, text: &str, present: bool) -> Result<()> {
        let found = self.text_present(text)?;
        if found == present {
            Ok(())
        } else if present {
            bail!("期望界面上出现「{text}」，但没有找到")
        } else {
            bail!("期望「{text}」已经消失，但它还在")
        }
    }

    fn text_present(&self, text: &str) -> Result<bool> {
        let scene = scene_of(self.hwnd)?;
        let lines = recognize_shot_lines(&scene.shot, OCR_SCALE).map_err(anyhow::Error::msg)?;
        Ok(find_best_occurrence(&lines, text, 0).is_some())
    }

    /// 文字目标靠 OCR 定位，坐标目标直接换算。返回值是**屏幕坐标**。
    fn resolve(&self, target: &Target, origin: (i32, i32)) -> Result<(i32, i32)> {
        if let (Some(x), Some(y)) = (target.x, target.y) {
            return Ok((x, y));
        }
        let Some(text) = target.text.as_deref() else {
            bail!("目标必须给 text 或 x/y");
        };
        let scene = scene_of(self.hwnd)?;
        let lines = recognize_shot_lines(&scene.shot, OCR_SCALE).map_err(anyhow::Error::msg)?;
        let hit = find_best_occurrence(&lines, text, target.occurrence).ok_or_else(|| {
            anyhow::anyhow!(
                "界面上找不到「{text}」的第 {} 个命中",
                target.occurrence + 1
            )
        })?;
        let _ = origin;
        // 截图坐标是相对窗口的，点击要的是屏幕坐标。
        Ok((hit.center.0 + scene.origin.0, hit.center.1 + scene.origin.1))
    }

    fn note_click(&mut self, point: (i32, i32)) -> Result<()> {
        let counter = self.click_counts.entry(point).or_insert(0);
        *counter += 1;
        if *counter >= self.limits.duplicate_clicks {
            bail!(
                "同一个位置 ({}, {}) 已经点了 {} 次，判定为卡住",
                point.0,
                point.1,
                *counter
            );
        }
        Ok(())
    }
}

/// 抓一次窗口画面，把字符串错误转成 anyhow。
fn scene_of(hwnd: HWND) -> Result<crate::gui::capture::WindowScene> {
    capture_window_scene(hwnd).map_err(anyhow::Error::msg)
}

/// 组合键解析。故意只支持常用几个：GUI 自动化里真正需要的就这些，
/// 支持全部虚拟键码只会让错误输入更难发现。
fn press(combo: &str) -> Result<()> {
    let combo = combo.trim().to_ascii_lowercase();
    if combo == "alt+left" {
        return send_alt_key(VK_LEFT).map_err(anyhow::Error::msg);
    }
    if let Some(rest) = combo.strip_prefix("ctrl+") {
        let mut chars = rest.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            bail!("ctrl+ 后面只支持单个字符，收到「{combo}」");
        };
        return ctrl_key(VIRTUAL_KEY(ch.to_ascii_uppercase() as u16)).map_err(anyhow::Error::msg);
    }
    let vk = match combo.as_str() {
        "enter" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "space" => 0x20,
        "delete" | "del" => 0x2E,
        "backspace" => 0x08,
        other => bail!("不支持的按键「{other}」"),
    };
    tap_key(VIRTUAL_KEY(vk)).map_err(anyhow::Error::msg)
}

fn send_alt_key(vk: VIRTUAL_KEY) -> Result<(), String> {
    use crate::gui::input::send_key_sequence;
    send_key_sequence(&[(VK_MENU, false), (vk, false), (vk, true), (VK_MENU, true)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions_from_json() {
        let script = r#"[
            {"action":"click","target":{"text":"导出"}},
            {"action":"type","text":"PomBot"},
            {"action":"keys","combo":"ctrl+a"},
            {"action":"wait","ms":500},
            {"action":"expect_text","text":"完成"},
            {"action":"expect_text","text":"错误","present":false}
        ]"#;
        let actions: Vec<Action> = serde_json::from_str(script).expect("应能解析");
        assert_eq!(actions.len(), 6);
        assert!(matches!(actions[0], Action::Click { .. }));
        assert!(matches!(
            actions[4],
            Action::ExpectText { present: true, .. }
        ));
        assert!(matches!(
            actions[5],
            Action::ExpectText { present: false, .. }
        ));

        let conditional: Vec<Action> = serde_json::from_str(
            r#"[
                {"action":"wait_for_text","text":"完成","timeout_ms":3000},
                {"action":"dialog","accept":true},
                    {"action":"screenshot","path":"artifacts/step.png"}
                ]"#,
        )
        .expect("应能解析条件等待和截图");
        assert!(matches!(
            conditional[0],
            Action::WaitForText {
                timeout_ms: 3000,
                ..
            }
        ));
        assert!(matches!(conditional[1], Action::Dialog { accept: true }));
        assert!(matches!(conditional[2], Action::Screenshot { .. }));
    }

    #[test]
    fn target_accepts_point_or_text() {
        let point: Target = serde_json::from_str(r#"{"x":100,"y":200}"#).expect("坐标目标");
        assert_eq!(point.x, Some(100));
        let text: Target = serde_json::from_str(r#"{"text":"确定"}"#).expect("文字目标");
        assert_eq!(text.text.as_deref(), Some("确定"));
        assert_eq!(text.occurrence, 0);
    }

    #[test]
    fn rejects_unknown_key_names() {
        // 拼错键名要当场报错，而不是静默什么都不做。
        assert!(press("hyper+q").is_err());
        assert!(press("ctrl+ab").is_err());
    }

    #[test]
    fn step_limit_is_enforced_before_running() {
        // 上限检查在跑之前就做，避免跑到一半才停。
        let mut executor = Executor::new(HWND::default());
        let actions: Vec<Action> = (0..3).map(|_| Action::Wait { ms: 1 }).collect();
        let result = executor.run(&actions);
        // 只有 3 步、上限 40，不应因上限失败（真实执行会因为窗口无效而失败，
        // 所以这里只断言不是"超过上限"这一类错误）。
        if let Err(error) = result {
            assert!(!error.to_string().contains("超过上限"), "实际：{error}");
        }
    }
}
