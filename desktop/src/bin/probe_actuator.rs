// @author wzxgqtl 2026-11-22
// 执行层未知数探针。
//
// 回答两个决定动作层接口的问题：
// 1. SetForegroundWindow 的前台锁在本机是否会发生，AttachThreadInput 能否兜底
// 2. 中文输入走 Unicode 直发是否可靠，还是必须退回剪贴板粘贴
//
// 判据不用 OCR：记事本打开的是一个已知路径的临时文件，输入后 Ctrl+S 保存，
// 直接比对文件内容。这样"没打进去"和"OCR 没认出来"不会混在一起。

use std::{process::Command, thread::sleep, time::Duration};

use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;

use pombot::gui::{
    capture::visible_bounds,
    init_dpi_awareness,
    input::{click_at, ctrl_key, paste_clipboard, set_clipboard_text, tap_key, type_unicode},
    window::{WindowInfo, find_by_title, focus_verified, foreground},
};

/// VK_A / VK_DELETE / VK_S。windows crate 未导出，按 WinUser.h 取值。
const VK_A: VIRTUAL_KEY = VIRTUAL_KEY(0x41);
const VK_S: VIRTUAL_KEY = VIRTUAL_KEY(0x53);
const VK_DELETE: VIRTUAL_KEY = VIRTUAL_KEY(0x2E);

struct Outcome {
    label: &'static str,
    passed: bool,
    detail: String,
}

fn main() {
    init_dpi_awareness();

    let marker = "pombot-actuator-probe";
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
        println!("未找到记事本窗口（标题应包含 {marker}）");
        let _ = notepad.kill();
        return;
    };
    println!("记事本窗口：{}", window.title);

    let mut results: Vec<Outcome> = Vec::new();

    // ---- 1. 前台锁 ----
    let focused = focus_verified(window.hwnd);
    let is_foreground = foreground() == Some(window.hwnd);
    results.push(Outcome {
        label: "前台激活",
        passed: focused.is_ok() && is_foreground,
        detail: match &focused {
            Ok(()) => "成功且 GetForegroundWindow 校验一致".to_owned(),
            Err(error) => format!("失败：{error}"),
        },
    });

    // 置前不等于编辑区有焦点（新记事本带标签栏与工具栏），先点一下正文。
    let bounds = visible_bounds(window.hwnd).unwrap_or((0, 0, 1280, 800));
    let clicked = click_at(bounds.0 + bounds.2 / 2, bounds.1 + bounds.3 / 2).is_ok();
    sleep(Duration::from_millis(200));
    println!("点击正文区域：{}", if clicked { "成功" } else { "失败" });

    // ---- 2. 三条输入路径 ----
    results.push(typing_case(
        &file,
        "英文与数字（Unicode 直发）",
        "PomBot 2026 abc 0123456789",
        type_unicode,
    ));
    results.push(typing_case(
        &file,
        "中文（Unicode 直发）",
        "中文输入测试",
        type_unicode,
    ));
    results.push(typing_case(
        &file,
        "混合长串（剪贴板粘贴）",
        "PomBot 2026 abc 0123456789 中文混合",
        |text| {
            set_clipboard_text(text)?;
            sleep(Duration::from_millis(150));
            paste_clipboard()
        },
    ));

    // ---- 汇总 ----
    println!("\n== 结论 ==");
    println!("{:<26} {:<6} 说明", "场景", "结果");
    println!("{}", "-".repeat(70));
    for item in &results {
        println!(
            "{:<26} {:<6} {}",
            item.label,
            if item.passed { "通过" } else { "失败" },
            item.detail
        );
    }

    let unicode_ok = results
        .iter()
        .filter(|item| item.label.contains("Unicode"))
        .all(|item| item.passed);
    let clipboard_ok = results
        .iter()
        .any(|item| item.label.contains("剪贴板") && item.passed);
    println!(
        "\n总体：{}",
        if unicode_ok {
            "Unicode 直发可用，动作层以它为主路径，剪贴板仅作备选"
        } else if clipboard_ok {
            "Unicode 直发在记事本上失效，动作层必须内建剪贴板回退"
        } else {
            "两条路径都不通，先排查焦点与权限"
        }
    );

    let _ = notepad.kill();
    let _ = std::fs::remove_file(&file);
}

/// 清空编辑区（Ctrl+A → Delete），输入文本，保存到磁盘，再比对文件内容。
fn typing_case<F>(file: &std::path::Path, label: &'static str, text: &str, type_it: F) -> Outcome
where
    F: Fn(&str) -> Result<(), String>,
{
    // 先清空，避免上一条用例的残留影响判断。
    let _ = ctrl_key(VK_A);
    sleep(Duration::from_millis(80));
    let _ = tap_key(VK_DELETE);
    sleep(Duration::from_millis(80));

    if let Err(error) = type_it(text) {
        return Outcome {
            label,
            passed: false,
            detail: format!("注入失败：{error}"),
        };
    }
    sleep(Duration::from_millis(200));

    // Ctrl+S 落盘。记事本打开的是已存在的文件，因此是直接覆盖，不会弹另存为。
    let _ = ctrl_key(VK_S);
    sleep(Duration::from_millis(350));

    let saved = std::fs::read_to_string(file).unwrap_or_default();
    let passed = saved.trim() == text;
    Outcome {
        label,
        passed,
        detail: if passed {
            "文件内容一致".to_owned()
        } else {
            format!(
                "文件内容为：{}",
                saved.trim().chars().take(40).collect::<String>()
            )
        },
    }
}

/// 轮询等待标题包含 `marker` 的窗口出现。
fn wait_for_window(marker: &str, timeout: Duration) -> Option<WindowInfo> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(window) = find_by_title(marker) {
            // 窗口刚创建时标题可能还没填好，稍等再确认一次。
            sleep(Duration::from_millis(300));
            return find_by_title(marker).or(Some(window));
        }
        sleep(Duration::from_millis(150));
    }
    None
}
