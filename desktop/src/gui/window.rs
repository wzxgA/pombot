// @author wzxgqtl 2026-11-20
// 窗口枚举与置前。这里只用窗口级信息（标题、矩形、可见性），
// 不触碰控件树 —— 项目采用纯视觉路线，控件语义一律不引入。

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
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

/// 当前前台窗口。
// @author wzxgqtl 2026-11-22
pub fn foreground() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() { None } else { Some(hwnd) }
}

/// 置前并**校验**是否真的成功。
///
/// `SetForegroundWindow` 有个著名的前台锁：非前台进程调用时可能返回成功但窗口并未激活。
/// 因此不能信任返回值，必须比对 `GetForegroundWindow`。
/// 失败时用 `AttachThreadInput` 把本线程的输入队列临时挂到目标线程，再试一次。
// @author wzxgqtl 2026-11-22
pub fn focus_verified(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        if foreground() == Some(hwnd) {
            return Ok(());
        }

        let _ = SetForegroundWindow(hwnd);
        if foreground() == Some(hwnd) {
            return Ok(());
        }

        // 第二个办法：先模拟一次 ALT 按键。系统会因此认为"本进程刚收到用户输入"，
        // 从而解除前台锁。实测在终端里跑脚本时，前两种都会失败，只有这一条能过。
        let _ = crate::gui::input::tap_key(VIRTUAL_KEY(0x12)); // VK_MENU = ALT
        let _ = SetForegroundWindow(hwnd);
        if foreground() == Some(hwnd) {
            return Ok(());
        }

        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let current_thread = GetCurrentThreadId();
        let attached = AttachThreadInput(current_thread, target_thread, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(current_thread, target_thread, false);
        }

        if foreground() == Some(hwnd) {
            Ok(())
        } else {
            Err("前台锁：AttachThreadInput 兜底仍未激活窗口".into())
        }
    }
}
