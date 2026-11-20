// @author wzxgqtl 2026-11-20
// 注入探针：验证 SendInput 绝对坐标在虚拟桌面上的落点精度。
//
// 判定方式不靠肉眼：注入后读回真实光标位置（GetCursorPos）与目标比对，
// 误差 <= 1px 视为通过。这样在任何 DPI 缩放下都能给出客观结论。

use pombot::gui::input::{cursor_pos, move_to, to_absolute, virtual_desktop};
use pombot::gui::{dpi_report, init_dpi_awareness};

// @author wzxgqtl 2026-11-20
fn main() {
    init_dpi_awareness();

    let (vx, vy, vw, vh) = virtual_desktop();
    println!("== 环境 ==");
    println!("{}", dpi_report());
    println!("虚拟桌面: 原点=({vx},{vy}) 尺寸={vw}x{vh}\n");

    // 取 5 个分布均匀的点，覆盖四角与中心：边缘最能暴露归一化分母写错的问题。
    let fracs = [
        (0.05, 0.05),
        (0.95, 0.05),
        (0.5, 0.5),
        (0.05, 0.95),
        (0.95, 0.95),
    ];

    println!("== 落点精度 ==");
    println!(
        "{:>5} {:>18} {:>18} {:>10} {:>6}",
        "#", "目标(px)", "实际(px)", "误差", "判定"
    );
    println!("{}", "-".repeat(64));

    let mut worst = 0i32;
    let mut passed = 0;

    for (i, (fx, fy)) in fracs.iter().enumerate() {
        let tx = vx + (*fx * (vw - 1) as f64).round() as i32;
        let ty = vy + (*fy * (vh - 1) as f64).round() as i32;

        let (nx, ny) = to_absolute(tx, ty);

        if let Err(e) = move_to(tx, ty) {
            println!("{i:>5} 注入失败: {e}");
            continue;
        }
        // 给系统一点时间完成光标更新。
        std::thread::sleep(std::time::Duration::from_millis(30));

        match cursor_pos() {
            Ok((ax, ay)) => {
                let dx = (ax - tx).abs();
                let dy = (ay - ty).abs();
                let err = dx.max(dy);
                worst = worst.max(err);
                let ok = err <= 1;
                if ok {
                    passed += 1;
                }
                println!(
                    "{:>5} {:>18} {:>18} {:>10} {:>6}",
                    i,
                    format!("{tx},{ty}"),
                    format!("{ax},{ay}"),
                    format!("{dx},{dy}"),
                    if ok { "通过" } else { "偏差" }
                );
                println!("       归一化: ({nx},{ny})");
            }
            Err(e) => println!("{i:>5} 读回光标失败: {e}"),
        }
    }

    println!("\n== 结论 ==");
    println!("通过 {passed}/{} 点，最大误差 {worst}px", fracs.len());
    if worst <= 1 {
        println!("判定：坐标换算正确，可进入后续实现。");
    } else {
        println!("判定：存在系统性偏移，必须先修正换算公式。");
        println!(
            "排查方向：DPI 感知是否生效、是否漏了虚拟桌面原点偏移、归一化分母是否用了 width 而非 width-1。"
        );
    }
}
