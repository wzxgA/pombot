// @author wzxgqtl 2026-11-20
// OCR 探针：验证 Windows 内置引擎对中文界面文本的识别能力。
//
// 关键对照是"原尺寸 vs 放大 2 倍"。系统 OCR 对小字号明显偏弱，
// 而放大几乎不增加成本，需要实测确认它是否足以补救。

use std::time::Instant;

use pombot::gui::capture::{Shot, capture_screen};
use pombot::gui::ocr::{engine_report, recognize_png, recognize_shot};
use pombot::gui::{dpi_report, init_dpi_awareness};

// @author wzxgqtl 2026-11-20
fn main() {
    init_dpi_awareness();
    println!("== 环境 ==");
    println!("{}", dpi_report());
    println!("{}", engine_report());
    println!();

    // 三种用法：
    //   probe_ocr                     截全屏后识别（默认）
    //   probe_ocr <png 路径>          只识别该文件
    //   probe_ocr --region x,y,w,h    截全屏后裁出该区域再识别，用于测量「局部识别」的真实耗时
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--region") {
        let spec = args.get(1).cloned().unwrap_or_default();
        let nums: Vec<i32> = spec
            .split(',')
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .collect();
        if nums.len() != 4 {
            println!("用法: probe_ocr --region x,y,w,h");
            return;
        }
        let shot = match capture_screen() {
            Ok(s) => s,
            Err(e) => {
                println!("截图失败: {e}");
                return;
            }
        };
        let cropped = crop(&shot, nums[0], nums[1], nums[2], nums[3]);
        println!(
            "== 局部识别 ==\n区域: ({},{}) {}x{}\n",
            nums[0], nums[1], nums[2], nums[3]
        );
        run_pair(std::path::Path::new("."), Some(&cropped));
        return;
    }

    // 传入图片路径时只识别该文件；不传则先截一张全屏，模拟真实取图链路。
    let arg = args.into_iter().next();
    match arg {
        Some(p) => {
            let path = std::path::PathBuf::from(p);
            println!("== 识别文件 ==\n{}\n", path.display());
            run_pair(&path, None);
        }
        None => {
            println!("== 截屏后识别 ==（未传参，自动截取全屏）");
            let shot: Shot = match capture_screen() {
                Ok(s) => s,
                Err(e) => {
                    println!("截图失败: {e}");
                    return;
                }
            };
            println!("截图尺寸: {}x{}", shot.width, shot.height);

            let mut base = std::env::temp_dir();
            base.push(format!("pombot-probe-ocr-{}.png", std::process::id()));
            if let Err(e) = shot.save_png(&base) {
                println!("保存失败: {e}");
                return;
            }
            println!("临时文件: {}\n", base.display());
            run_pair(&base, Some(&shot));
        }
    }
}

/// 对同一画面分别跑原尺寸与 2 倍放大，打印耗时与文本，便于对比。
// @author wzxgqtl 2026-11-20
fn run_pair(path: &std::path::Path, shot: Option<&Shot>) {
    let t0 = Instant::now();
    let r1 = if let Some(s) = shot {
        recognize_shot(s, 1.0).map(|(t, _)| t)
    } else {
        recognize_png(path)
    };
    let ms1 = t0.elapsed().as_millis();
    report("原尺寸", ms1, &r1);

    let t0 = Instant::now();
    let r2 = if let Some(s) = shot {
        recognize_shot(s, 2.0).map(|(t, _)| t)
    } else {
        // 无像素数据时只能重读文件，放大交给外部工具处理。
        recognize_png(path)
    };
    let ms2 = t0.elapsed().as_millis();
    report("放大2倍", ms2, &r2);

    println!("\n提示：把两张结果贴到决策文档里，作为 OCR 策略的依据。");
}

/// 按物理像素裁切。局部识别是控制单步开销的关键手段，
/// 全屏识别单次接近 800ms，不可能每步都跑。
// @author wzxgqtl 2026-11-20
fn crop(shot: &Shot, x: i32, y: i32, w: i32, h: i32) -> Shot {
    let sx = x.clamp(0, shot.width as i32) as usize;
    let sy = y.clamp(0, shot.height as i32) as usize;
    let sw = w.clamp(1, shot.width as i32 - sx as i32) as usize;
    let sh = h.clamp(1, shot.height as i32 - sy as i32) as usize;

    let mut bgra = Vec::with_capacity(sw * sh * 4);
    for row in sy..sy + sh {
        let start = (row * shot.width + sx) * 4;
        bgra.extend_from_slice(&shot.bgra[start..start + sw * 4]);
    }
    Shot {
        width: sw,
        height: sh,
        bgra,
    }
}

// @author wzxgqtl 2026-11-20
fn report(label: &str, ms: u128, r: &Result<String, String>) {
    match r {
        Ok(text) => {
            let chars = text.chars().count();
            println!("== {label} == 耗时 {ms}ms 字符数 {chars}");
            let preview: String = text.chars().take(400).collect();
            println!("{preview}");
            println!();
        }
        Err(e) => {
            println!("== {label} == 耗时 {ms}ms 失败: {e}\n");
        }
    }
}
