// @author wzxgqtl 2026-11-20
// 键鼠注入。只走 SendInput，不依赖 UI Automation。
//
// 坐标约定：项目内部统一使用"虚拟桌面物理像素坐标"，原点为整个虚拟桌面的左上角，
// 可为负数（副屏在主屏左侧时）。SendInput 需要的是 0~65535 的归一化值，
// 因此每次注入前都要过一遍 to_absolute。

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

/// 虚拟桌面矩形 (x, y, width, height)。
// @author wzxgqtl 2026-11-20
pub fn virtual_desktop() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// 纯函数：物理像素 -> 归一化。与系统当前状态无关，因此可以单测。
///
/// 分母必须是 `size - 1`：归一化区间是闭区间 0..=65535，
/// 用 `size` 做分母会让最右/最下一列永远点不到。
// @author wzxgqtl 2026-11-20
pub fn normalize_point(x: i32, y: i32, vx: i32, vy: i32, vw: i32, vh: i32) -> (i32, i32) {
    let nx = (x - vx) as f64 * 65535.0 / (vw.max(2) - 1) as f64;
    let ny = (y - vy) as f64 * 65535.0 / (vh.max(2) - 1) as f64;
    (nx.round() as i32, ny.round() as i32)
}

/// 纯函数：归一化 -> 物理像素。与 [`normalize_point`] 互为逆运算。
// @author wzxgqtl 2026-11-20
pub fn denormalize_point(nx: i32, ny: i32, vx: i32, vy: i32, vw: i32, vh: i32) -> (i32, i32) {
    let x = (nx as f64 * (vw.max(2) - 1) as f64 / 65535.0).round() as i32 + vx;
    let y = (ny as f64 * (vh.max(2) - 1) as f64 / 65535.0).round() as i32 + vy;
    (x, y)
}

/// 虚拟桌面物理像素 -> SendInput 归一化坐标。
// @author wzxgqtl 2026-11-20
pub fn to_absolute(x: i32, y: i32) -> (i32, i32) {
    let (vx, vy, vw, vh) = virtual_desktop();
    normalize_point(x, y, vx, vy, vw, vh)
}

/// 归一化坐标 -> 虚拟桌面物理像素。探针用它反推实际落点。
// @author wzxgqtl 2026-11-20
pub fn from_absolute(nx: i32, ny: i32) -> (i32, i32) {
    let (vx, vy, vw, vh) = virtual_desktop();
    denormalize_point(nx, ny, vx, vy, vw, vh)
}

// @author wzxgqtl 2026-11-20
fn mouse_input(
    dx: i32,
    dy: i32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// 把光标移到虚拟桌面物理像素坐标 (x, y)。
// @author wzxgqtl 2026-11-20
pub fn move_to(x: i32, y: i32) -> Result<(), String> {
    let (nx, ny) = to_absolute(x, y);
    let flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let inputs = [mouse_input(nx, ny, flags)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(format!("SendInput 移动失败，返回 {sent}"));
    }
    Ok(())
}

/// 在 (x, y) 处单击。
// @author wzxgqtl 2026-11-20
pub fn click_at(x: i32, y: i32) -> Result<(), String> {
    let (nx, ny) = to_absolute(x, y);
    let move_flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let inputs = [
        mouse_input(nx, ny, move_flags),
        mouse_input(0, 0, MOUSEEVENTF_LEFTDOWN),
        mouse_input(0, 0, MOUSEEVENTF_LEFTUP),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(format!(
            "SendInput 点击只发送了 {sent}/{} 个事件",
            inputs.len()
        ));
    }
    Ok(())
}

/// 当前光标位置（虚拟桌面物理像素）。
// @author wzxgqtl 2026-11-20
pub fn cursor_pos() -> Result<(i32, i32), String> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.map_err(|e| e.to_string())?;
    Ok((p.x, p.y))
}

// 坐标换算是纯数学，与系统 DPI 无关，因此这里用单测把边界情况锁死。
// 150% 缩放与多显示器的实机验证仍需人工切换环境，但公式本身不能靠侥幸。
// @author wzxgqtl 2026-11-20
#[cfg(test)]
mod tests {
    use super::{denormalize_point, normalize_point};

    /// 单屏 2560x1440，原点 (0,0)。
    #[test]
    fn roundtrip_single_monitor() {
        let (vx, vy, vw, vh) = (0, 0, 2560, 1440);
        for &(x, y) in &[(0, 0), (1, 1), (1279, 719), (2559, 1439), (128, 72)] {
            let (nx, ny) = normalize_point(x, y, vx, vy, vw, vh);
            let (bx, by) = denormalize_point(nx, ny, vx, vy, vw, vh);
            assert!(
                (bx - x).abs() <= 1 && (by - y).abs() <= 1,
                "({x},{y}) -> ({nx},{ny}) -> ({bx},{by})"
            );
        }
    }

    /// 四角必须精确落在 0 与 65535，这是最容易写错的地方。
    #[test]
    fn corners_are_exact() {
        let (vx, vy, vw, vh) = (0, 0, 2560, 1440);
        assert_eq!(normalize_point(vx, vy, vx, vy, vw, vh), (0, 0));
        assert_eq!(
            normalize_point(vx + vw - 1, vy + vh - 1, vx, vy, vw, vh),
            (65535, 65535)
        );
    }

    /// 副屏在主屏左侧时，虚拟桌面原点为负数。符号错一位就会整体跑到另一块屏上。
    #[test]
    fn roundtrip_negative_origin() {
        let (vx, vy, vw, vh) = (-1920, 0, 4480, 1440);
        for &(x, y) in &[(-1920, 0), (-1, 700), (0, 0), (2559, 1439), (1000, 900)] {
            let (nx, ny) = normalize_point(x, y, vx, vy, vw, vh);
            let (bx, by) = denormalize_point(nx, ny, vx, vy, vw, vh);
            assert!(
                (bx - x).abs() <= 1 && (by - y).abs() <= 1,
                "({x},{y}) -> ({nx},{ny}) -> ({bx},{by})"
            );
            assert!((0..=65535).contains(&nx) && (0..=65535).contains(&ny));
        }
    }

    /// 奇数尺寸与 4K 竖屏，验证取整不会累计出 >1px 的误差。
    #[test]
    fn roundtrip_odd_and_large_sizes() {
        for &(vw, vh) in &[(1921, 1081), (3840, 2160), (2160, 3840), (800, 601)] {
            let (vx, vy) = (0, 0);
            for fx in [0.0, 0.03, 0.5, 0.97, 1.0] {
                for fy in [0.0, 0.5, 1.0] {
                    let x = (fx * (vw - 1) as f64).round() as i32;
                    let y = (fy * (vh - 1) as f64).round() as i32;
                    let (nx, ny) = normalize_point(x, y, vx, vy, vw, vh);
                    let (bx, by) = denormalize_point(nx, ny, vx, vy, vw, vh);
                    assert!(
                        (bx - x).abs() <= 1 && (by - y).abs() <= 1,
                        "{vw}x{vh} ({x},{y}) -> ({bx},{by})"
                    );
                }
            }
        }
    }
}
