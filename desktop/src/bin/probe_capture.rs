// @author wzxgqtl 2026-11-20
// 截图探针：对若干真实目标窗口，横向比较三种截屏路径的可用性。
//
// 判定"黑屏"不看人眼，而是算亮度均值与近黑像素占比，避免主观误判。

use std::time::Instant;

use pombot::gui::capture::{
    Shot, ShotStats, capture_screen, capture_window_dc, capture_window_printwindow, outer_bounds,
    visible_bounds,
};
use pombot::gui::window::find_by_title;
use pombot::gui::{dpi_report, init_dpi_awareness};

// @author wzxgqtl 2026-11-20
struct Row {
    method: &'static str,
    target: String,
    ms: u128,
    size: String,
    stats: Option<ShotStats>,
    note: String,
}

// @author wzxgqtl 2026-11-20
fn main() {
    init_dpi_awareness();
    println!("== 环境 ==");
    println!("{}", dpi_report());

    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "probe-results/capture".to_string());
    let out_dir = std::path::PathBuf::from(out_dir);
    println!("输出目录: {}\n", out_dir.display());

    let mut rows: Vec<Row> = Vec::new();

    // 屏幕整屏截图作为基准：如果连屏都截不到，问题不在目标窗口。
    let t0 = Instant::now();
    match capture_screen() {
        Ok(shot) => {
            let ms = t0.elapsed().as_millis();
            let stats = shot.stats();
            let path = out_dir.join("screen_bitblt.png");
            let _ = shot.save_png(&path);
            rows.push(Row {
                method: "bitblt_screen",
                target: "整个桌面".into(),
                ms,
                size: format!("{}x{}", shot.width, shot.height),
                stats: Some(stats),
                note: "基准".into(),
            });
        }
        Err(e) => rows.push(Row {
            method: "bitblt_screen",
            target: "整个桌面".into(),
            ms: t0.elapsed().as_millis(),
            size: "-".into(),
            stats: None,
            note: e,
        }),
    }

    // 目标可自定义：第一个参数是输出目录，之后的参数都是窗口标题子串。
    // 默认覆盖三类渲染路径：Chromium 系（GPU 合成）、Electron、以及系统原生应用。
    let cli_targets: Vec<String> = std::env::args().skip(2).collect();
    let targets: Vec<String> = if cli_targets.is_empty() {
        vec![
            "Chrome".into(),
            "Edge".into(),
            "Firefox".into(),
            "CodeBuddy".into(),
            "记事本".into(),
            "Notepad".into(),
        ]
    } else {
        cli_targets
    };
    let mut seen: Vec<String> = Vec::new();
    for t in targets {
        if seen.iter().any(|s| s.eq_ignore_ascii_case(&t)) {
            continue;
        }
        seen.push(t.clone());
        let Some(w) = find_by_title(&t) else {
            println!("[跳过] 未找到窗口: {t}");
            continue;
        };
        println!("[命中] {} -> {}", t, w.title);

        probe_window(&w.hwnd, &w.title, &out_dir, &mut rows);
    }

    print_table(&rows);
    println!("\n提示：截图文件已保存，可直接打开肉眼复核。");
}

// @author wzxgqtl 2026-11-20
fn probe_window(
    hwnd: &windows::Win32::Foundation::HWND,
    title: &str,
    out_dir: &std::path::Path,
    rows: &mut Vec<Row>,
) {
    // 先量化 GetWindowRect 与真实可见边界的差值。
    // 这个差值就是"直接拿 GetWindowRect 算坐标"会产生的系统性偏移。
    match (outer_bounds(*hwnd), visible_bounds(*hwnd)) {
        (Ok(o), Ok(v)) => {
            println!(
                "       边界: GetWindowRect=({},{},{}x{})  DWM可见=({},{},{}x{})  差值=左{} 上{} 右{} 下{}",
                o.0,
                o.1,
                o.2,
                o.3,
                v.0,
                v.1,
                v.2,
                v.3,
                v.0 - o.0,
                v.1 - o.1,
                (o.0 + o.2) - (v.0 + v.2),
                (o.1 + o.3) - (v.1 + v.3)
            );
        }
        (Err(e), _) => println!("       边界: 读取失败 {e}"),
        (_, Err(e)) => println!("       边界: DWM 读取失败 {e}"),
    }

    // 方式一：窗口 DC + BitBlt
    let t0 = Instant::now();
    match capture_window_dc(*hwnd) {
        Ok(s) => push(
            rows,
            "bitblt_window_dc",
            title,
            t0,
            &s,
            out_dir,
            "window_dc",
        ),
        Err(e) => rows.push(fail("bitblt_window_dc", title, t0, e)),
    }

    // 方式二：PrintWindow 不带 RENDERFULLCONTENT
    let t0 = Instant::now();
    match capture_window_printwindow(*hwnd, false) {
        Ok(s) => push(rows, "printwindow", title, t0, &s, out_dir, "pw"),
        Err(e) => rows.push(fail("printwindow", title, t0, e)),
    }

    // 方式三：PrintWindow + RENDERFULLCONTENT，Chrome 的关键路径
    let t0 = Instant::now();
    match capture_window_printwindow(*hwnd, true) {
        Ok(s) => push(rows, "printwindow_full", title, t0, &s, out_dir, "pwfull"),
        Err(e) => rows.push(fail("printwindow_full", title, t0, e)),
    }
}

// @author wzxgqtl 2026-11-20
fn push(
    rows: &mut Vec<Row>,
    method: &'static str,
    title: &str,
    t0: Instant,
    s: &Shot,
    out_dir: &std::path::Path,
    tag: &str,
) {
    let ms = t0.elapsed().as_millis();
    let stats = s.stats();
    let safe: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(24)
        .collect();
    let path = out_dir.join(format!("{safe}_{tag}.png"));
    let _ = s.save_png(&path);
    rows.push(Row {
        method,
        target: title.to_string(),
        ms,
        size: format!("{}x{}", s.width, s.height),
        stats: Some(stats),
        note: path.display().to_string(),
    });
}

// @author wzxgqtl 2026-11-20
fn fail(method: &'static str, title: &str, t0: Instant, e: String) -> Row {
    Row {
        method,
        target: title.to_string(),
        ms: t0.elapsed().as_millis(),
        size: "-".into(),
        stats: None,
        note: e,
    }
}

// @author wzxgqtl 2026-11-20
fn print_table(rows: &[Row]) {
    println!("\n== 结果 ==");
    println!(
        "{:<20} {:<28} {:>7} {:>12} {:>9} {:>8}  判定/备注",
        "方式", "目标", "耗时ms", "尺寸", "亮度均值", "黑占比"
    );
    println!("{}", "-".repeat(120));
    for r in rows {
        let (luma, black, verdict) = match &r.stats {
            Some(s) => {
                let v = if s.looks_black() { "黑屏" } else { "可用" };
                (
                    format!("{:.1}", s.mean_luma),
                    format!("{:.1}%", s.black_ratio * 100.0),
                    v.to_string(),
                )
            }
            None => ("-".into(), "-".into(), "失败".into()),
        };
        let note = if r.stats.is_some() {
            r.note.clone()
        } else {
            r.note.chars().take(40).collect()
        };
        println!(
            "{:<20} {:<28} {:>7} {:>12} {:>9} {:>8}  {} {}",
            r.method, r.target, r.ms, r.size, luma, black, verdict, note
        );
    }
}
