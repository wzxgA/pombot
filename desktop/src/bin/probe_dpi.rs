// @author wzxgqtl 2026-11-20
// DPI / 显示器诊断探针。
//
// 用途：在切换系统缩放（100% / 150%）或多显示器环境下，快速确认
// 进程的 DPI 感知状态、每块屏的真实 DPI，以及绝对坐标注入是否仍然准确。
// 坐标公式本身由 `gui::input` 的单测覆盖，这里只验证实机行为。

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS, GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetProcessDpiAwareness,
    GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI, PROCESS_DPI_AWARENESS,
};
use windows::core::BOOL;

use pombot::gui::input::{cursor_pos, move_to, virtual_desktop};
use pombot::gui::{dpi_report, init_dpi_awareness};

// @author wzxgqtl 2026-11-20
#[derive(Debug)]
struct Monitor {
    index: usize,
    rect: (i32, i32, i32, i32),
    dpi: u32,
    scale: u32,
    primary: bool,
    name: String,
}

// @author wzxgqtl 2026-11-20
unsafe extern "system" fn enum_mon(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<Monitor>) };

    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut mi.monitorInfo) }.as_bool() {
        let r = mi.monitorInfo.rcMonitor;
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let name_len = mi
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(mi.szDevice.len());
        out.push(Monitor {
            index: out.len(),
            rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
            dpi: dpi_x,
            scale: (dpi_x as f32 / 96.0 * 100.0).round() as u32,
            primary: mi.monitorInfo.dwFlags & 1 != 0,
            name: String::from_utf16_lossy(&mi.szDevice[..name_len]),
        });
    }
    BOOL(1)
}

// @author wzxgqtl 2026-11-20
fn awareness_label(a: DPI_AWARENESS) -> &'static str {
    // DPI_AWARENESS 的取值是 INVALID=-1 / UNAWARE=0 / SYSTEM_AWARE=1 / PER_MONITOR_AWARE=2，
    // 注意它与 PROCESS_DPI_AWARENESS（UNAWARE=0/SYSTEM=1/PER_MONITOR=2）恰好同值但含义不同，
    // 不要按"从 1 开始"去猜，容易误判成 SYSTEM_AWARE。
    match a.0 {
        0 => "UNAWARE（坐标会被系统虚拟化，不可用）",
        1 => "SYSTEM_AWARE（仅主屏 DPI 正确，多屏会错）",
        2 => "PER_MONITOR_AWARE（正确）",
        _ => "INVALID",
    }
}

// @author wzxgqtl 2026-11-20
fn main() {
    init_dpi_awareness();

    println!("== 进程与系统 ==");
    println!("{}", dpi_report());
    unsafe {
        let ctx = GetThreadDpiAwarenessContext();
        let a = GetAwarenessFromDpiAwarenessContext(ctx);
        println!("线程 DPI 感知: raw={} -> {}", a.0, awareness_label(a));

        // 交叉验证：GetProcessDpiAwareness 走另一套枚举（0=unaware,1=system,2=per-monitor），
        // 两个 API 结论不一致时，说明看的是不同层级（进程 vs 线程），不能只看一个。
        match GetProcessDpiAwareness(None) {
            Ok(pa) => {
                let pa: PROCESS_DPI_AWARENESS = pa;
                println!(
                    "进程 DPI 感知: raw={} -> {}",
                    pa.0,
                    match pa.0 {
                        0 => "UNAWARE",
                        1 => "SYSTEM_AWARE",
                        2 => "PER_MONITOR_AWARE",
                        _ => "INVALID",
                    }
                );
            }
            Err(e) => println!("GetProcessDpiAwareness 调用失败: {e}"),
        }
    }

    let mut monitors: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_mon),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }

    println!("\n== 显示器 ({} 个) ==", monitors.len());
    println!(
        "{:>3} {:<16} {:>22} {:>6} {:>6} {:>8}",
        "#", "设备", "矩形(x,y,w,h)", "DPI", "缩放", "主屏"
    );
    println!("{}", "-".repeat(70));
    for m in &monitors {
        println!(
            "{:>3} {:<16} {:>22} {:>6} {:>6} {:>8}",
            m.index,
            m.name,
            format!("{},{} {}x{}", m.rect.0, m.rect.1, m.rect.2, m.rect.3),
            m.dpi,
            format!("{}%", m.scale),
            if m.primary { "是" } else { "否" }
        );
    }

    let (vx, vy, vw, vh) = virtual_desktop();
    println!("\n虚拟桌面: 原点=({vx},{vy}) 尺寸={vw}x{vh}");

    // 每块屏取中心 + 四角内缩 5%，覆盖边缘换算与跨屏偏移。
    println!("\n== 逐屏落点精度 ==");
    println!(
        "{:<14} {:>18} {:>18} {:>10} {:>6}",
        "屏/位置", "目标(px)", "实际(px)", "误差", "判定"
    );
    println!("{}", "-".repeat(74));

    let mut worst = 0i32;
    let mut total = 0;
    let mut passed = 0;

    for m in &monitors {
        let (mx, my, mw, mh) = m.rect;
        let points = [
            ("中心", mx + mw / 2, my + mh / 2),
            (
                "左上",
                mx + (mw as f32 * 0.05) as i32,
                my + (mh as f32 * 0.05) as i32,
            ),
            (
                "右上",
                mx + (mw as f32 * 0.95) as i32,
                my + (mh as f32 * 0.05) as i32,
            ),
            (
                "左下",
                mx + (mw as f32 * 0.05) as i32,
                my + (mh as f32 * 0.95) as i32,
            ),
            (
                "右下",
                mx + (mw as f32 * 0.95) as i32,
                my + (mh as f32 * 0.95) as i32,
            ),
        ];

        for (label, tx, ty) in points {
            total += 1;
            let tag = format!("{}/{}", m.index, label);
            if let Err(e) = move_to(tx, ty) {
                println!("{:<14} 注入失败: {e}", tag);
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            match cursor_pos() {
                Ok((ax, ay)) => {
                    let err = (ax - tx).abs().max((ay - ty).abs());
                    worst = worst.max(err);
                    let ok = err <= 1;
                    if ok {
                        passed += 1;
                    }
                    println!(
                        "{:<14} {:>18} {:>18} {:>10} {:>6}",
                        tag,
                        format!("{tx},{ty}"),
                        format!("{ax},{ay}"),
                        err,
                        if ok { "通过" } else { "偏差" }
                    );
                }
                Err(e) => println!("{:<14} 读回光标失败: {e}", tag),
            }
        }
    }

    println!("\n== 结论 ==");
    println!("通过 {passed}/{total} 点，最大误差 {worst}px");
    let scopes: Vec<String> = monitors.iter().map(|m| format!("{}%", m.scale)).collect();
    println!("当前缩放: {}", scopes.join(", "));
    if worst <= 1 {
        println!("判定：在当前缩放与显示器布局下坐标换算正确。");
    } else {
        println!("判定：存在系统性偏移，坐标公式需修正。");
        println!(
            "排查：DPI 感知级别是否 PER_MONITOR、是否漏算虚拟桌面原点、窗口矩形是否用了 GetWindowRect（含 8px 边框）。"
        );
    }
}

// 让编译器确认 MONITORINFO 被引用（GetMonitorInfoW 需要父结构）。
// @author wzxgqtl 2026-11-20
#[allow(dead_code)]
fn _assert_monitorinfo(_m: &MONITORINFO) {}
