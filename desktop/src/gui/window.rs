// @author wzxgqtl 2026-11-20
// 窗口枚举与置前。这里只用窗口级信息（标题、矩形、可见性），
// 不触碰控件树 —— 项目采用纯视觉路线，控件语义一律不引入。

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};
use windows::core::BOOL;

// @author wzxgqtl 2026-11-20
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub visible: bool,
}

// @author wzxgqtl 2026-11-20
unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len > 0 {
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if n > 0 {
            out.push(WindowInfo {
                hwnd,
                title: String::from_utf16_lossy(&buf[..n as usize]),
                visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
            });
        }
    }
    BOOL(1)
}

/// 枚举所有有标题的顶层窗口。
// @author wzxgqtl 2026-11-20
pub fn list_windows() -> Vec<WindowInfo> {
    let mut v: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// 按标题子串查找可见窗口（不区分大小写）。
/// 探针用它定位 Chrome / 记事本；真实流程里这一层会被更强的匹配替代。
// @author wzxgqtl 2026-11-20
pub fn find_by_title(substr: &str) -> Option<WindowInfo> {
    let needle = substr.to_lowercase();
    list_windows()
        .into_iter()
        .find(|w| w.visible && w.title.to_lowercase().contains(&needle))
}

/// 恢复并置前窗口。
// @author wzxgqtl 2026-11-20
pub fn focus(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        if !SetForegroundWindow(hwnd).as_bool() {
            return Err("SetForegroundWindow 被系统拒绝（前台锁定）".into());
        }
    }
    Ok(())
}
