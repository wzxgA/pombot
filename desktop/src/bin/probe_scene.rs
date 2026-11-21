// @author wzxgqtl 2026-11-22
// 感知闭环探针：观察 → 识别 → 定位 → 点击 → 验证。
//
// 这一步不是验证单个 API，而是验证整条链路能拼起来：
// 截图坐标 + 窗口原点 = 屏幕坐标，算错一处就会点到别的地方，
// 而"点错"在日志里看起来和"点对了但没生效"完全一样。

use std::{process::Command, thread::sleep, time::Duration};

use pombot::gui::{
    capture::capture_window_scene,
    init_dpi_awareness,
    input::{click_at, type_text},
    locate::find_best,
    ocr::recognize_shot_lines,
    verify::{Observation, compare},
    window::{WindowInfo, find_by_title},
};

fn main() {
    init_dpi_awareness();

    let marker = "pombot-scene-probe";
    let file = std::env::temp_dir().join(format!("{marker}.txt"));
    let _ = std::fs::write(&file, "");

    let mut notepad = match Command::new("notepad.exe").arg(&file).spawn() {
        Ok(child) => child,
        Err(error) => {
            println!("无法启动记事本：{error}");
            return;
        }
    };

    let Some(window) = wait_for_window(marker, Duration::from_secs(8)) else {
        println!("未找到记事本窗口");
        let _ = notepad.kill();
        return;
    };

    // 先输入一段已知文本，作为定位目标。
    // 顺序不能反：置前只给窗口焦点，编辑区还要点一下，
    // 否则文字会打到别的窗口里去（这条已经踩过一次）。
    if let Err(error) = pombot::gui::window::focus_verified(window.hwnd) {
        println!("置前失败：{error}");
    }
    if let Ok((x, y, width, height)) = pombot::gui::capture::visible_bounds(window.hwnd) {
        let _ = click_at(x + width / 2, y + height / 2);
        sleep(Duration::from_millis(150));
    }
    if let Err(error) = type_text("PomBot 场景探针 scene") {
        println!("输入失败：{error}");
    }
    sleep(Duration::from_millis(300));

    let before = match observe(&window, "观察（点击前）") {
        Some(value) => value,
        None => {
            let _ = notepad.kill();
            return;
        }
    };

    // 定位：只给文字，坐标由 OCR 的包围盒算。
    let lines = match recognize_shot_lines(&before.observation.shot, 2.0) {
        Ok(lines) => lines,
        Err(error) => {
            println!("识别失败：{error}");
            let _ = notepad.kill();
            return;
        }
    };
    println!("识别到 {} 行文本", lines.len());
    for (index, line) in lines.iter().take(6).enumerate() {
        println!("  行{index}: {}", line.text);
    }

    let Some(hit) = find_best(&lines, "场景探针") else {
        println!("未定位到「场景探针」，放弃点击");
        let _ = notepad.kill();
        return;
    };
    println!(
        "定位命中：截图内 ({}, {}) {}x{}，中心 ({}, {})",
        hit.rect.x, hit.rect.y, hit.rect.width, hit.rect.height, hit.center.0, hit.center.1
    );

    // 关键换算：截图坐标是相对窗口的，点击要的是屏幕坐标。
    let screen = (
        hit.center.0 + before.origin.0,
        hit.center.1 + before.origin.1,
    );
    println!("换算到屏幕坐标：({}, {})", screen.0, screen.1);
    if let Err(error) = click_at(screen.0, screen.1) {
        println!("点击失败：{error}");
    }
    sleep(Duration::from_millis(250));

    let Some(after) = observe(&window, "观察（点击后）") else {
        let _ = notepad.kill();
        return;
    };
    let verdict = compare(&before.observation, &after.observation.shot);
    println!(
        "变化判定：changed={} hamming={} ratio={:.5}",
        verdict.changed, verdict.hamming, verdict.ratio
    );
    if !verdict.changed {
        println!("说明：点击落点未造成可测变化。若落点在文本上，光标移动可能低于阈值。");
    }

    let _ = notepad.kill();
    let _ = std::fs::remove_file(&file);
}

/// 一次带原点的观察。原点必须跟着截图一起传递，
/// 否则后面算屏幕坐标时无处可查。
struct Seen {
    observation: Observation,
    origin: (i32, i32),
}

/// 抓一次窗口画面，顺带打印耗时与尺寸。
fn observe(window: &WindowInfo, label: &str) -> Option<Seen> {
    let started = std::time::Instant::now();
    match capture_window_scene(window.hwnd) {
        Ok(scene) => {
            println!(
                "{}：{}x{} 原点 ({},{}) 耗时 {:?}",
                label,
                scene.shot.width,
                scene.shot.height,
                scene.origin.0,
                scene.origin.1,
                started.elapsed()
            );
            Some(Seen {
                observation: Observation::new(scene.shot),
                origin: scene.origin,
            })
        }
        Err(error) => {
            println!("{label} 失败：{error}");
            None
        }
    }
}

fn wait_for_window(marker: &str, timeout: Duration) -> Option<WindowInfo> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(window) = find_by_title(marker) {
            sleep(Duration::from_millis(300));
            return find_by_title(marker).or(Some(window));
        }
        sleep(Duration::from_millis(150));
    }
    None
}
