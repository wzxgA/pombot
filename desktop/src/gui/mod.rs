// @author wzxgqtl 2026-11-20
// GUI 底层能力：截图、键鼠注入、窗口枚举、OCR。

pub mod capture;
pub mod confirm;
pub mod digest;
pub mod executor;
pub mod input;
pub mod locate;
pub mod ocr;
pub mod verify;
pub mod window;

use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

/// 必须在任何坐标计算之前调用。
///
/// 进程默认是 DPI 不感知的，此时 `GetSystemMetrics` 与截图返回的都是被系统
/// 虚拟化（拉伸）后的逻辑像素，注入坐标会整体偏移。声明成 Per-Monitor V2 之后，
/// 所有 API 统一返回物理像素。
// @author wzxgqtl 2026-11-20
pub fn init_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 打印当前进程的 DPI 与显示器相关信息，便于排查坐标问题。
// @author wzxgqtl 2026-11-20
pub fn dpi_report() -> String {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemDpiForProcess};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    unsafe {
        let dpi = GetDpiForSystem();
        let pdpi = GetSystemDpiForProcess(windows::Win32::Foundation::HANDLE::default());
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        format!(
            "system_dpi={dpi} (scale={:.0}%) process_dpi={pdpi} virtual_screen=({vx},{vy}) {vw}x{vh}",
            dpi as f32 / 96.0 * 100.0
        )
    }
}
