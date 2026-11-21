// @author wzxgqtl 2026-11-21
// 高风险操作的本地否决窗口。
//
// 手机可能不在手边，所以确认必须双端可裁决：任何一端拒绝，操作就不执行。
// 这里用独立线程 + 原生控件实现——WINDOWS 的按钮自带点击命中与绘制，
// 比自绘省下大量代码，也不会因为主题变化而画错。

use std::{
    collections::VecDeque,
    ffi::c_void,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::HBRUSH,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            BN_CLICKED, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA,
            GetSystemMetrics, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SM_CXSCREEN,
            SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
            TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE, WNDCLASSW, WS_BORDER, WS_CHILD,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
        },
    },
    core::{HSTRING, PCWSTR},
};

const WIDTH: i32 = 460;
const HEIGHT: i32 = 168;
const ID_LABEL: i32 = 100;
const ID_REJECT: usize = 1;
const ID_APPROVE: usize = 2;

/// 0 表示未决；`Some(true)` 允许，`Some(false)` 拒绝。
static DECISION: Mutex<Option<bool>> = Mutex::new(None);

struct ConfirmRequest {
    tool: String,
    summary: String,
    deadline: Instant,
    reply: tokio::sync::oneshot::Sender<bool>,
}

struct Showing {
    request: ConfirmRequest,
    last_paint: Instant,
}

type Queue = Arc<Mutex<VecDeque<ConfirmRequest>>>;
static QUEUE: OnceLock<Queue> = OnceLock::new();

/// 启动本地确认窗口线程。常驻进程只应调用一次；未调用时 [`ask`] 返回 `None`。
pub fn start() {
    if QUEUE.get().is_some() {
        return;
    }
    let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
    if QUEUE.set(queue.clone()).is_err() {
        return;
    }
    std::thread::spawn(move || run_window(queue));
}

/// 在电脑上征询用户意见。返回 `None` 表示窗口未启动，调用方应退回只靠手机裁决。
pub fn ask(
    tool: &str,
    summary: &str,
    expires_in_ms: i64,
) -> Option<tokio::sync::oneshot::Receiver<bool>> {
    let queue = QUEUE.get()?;
    let (reply, receiver) = tokio::sync::oneshot::channel();
    let request = ConfirmRequest {
        tool: tool.to_owned(),
        summary: summary.to_owned(),
        deadline: Instant::now() + Duration::from_millis(expires_in_ms.max(1000) as u64),
        reply,
    };
    queue
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push_back(request);
    Some(receiver)
}

fn take_decision() -> Option<bool> {
    DECISION
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
}

fn run_window(queue: Queue) {
    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(module) => module,
            Err(_) => return,
        };
        let hinstance = HINSTANCE(module.0);
        let class_name = HSTRING::from("PomBotConfirm");

        let wndclass = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            // COLOR_WINDOW 的 stock brush 句柄就是索引 +1，省一次 GetSysColorBrush。
            hbrBackground: HBRUSH(6_usize as *mut c_void),
            ..Default::default()
        };
        if RegisterClassW(&wndclass) == 0 {
            return;
        }

        let x = GetSystemMetrics(SM_CXSCREEN) - WIDTH - 24;
        let y = GetSystemMetrics(SM_CYSCREEN) - HEIGHT - 72;
        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            &HSTRING::from("PomBot 确认"),
            WS_POPUP | WS_VISIBLE | WS_BORDER,
            x,
            y,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(hinstance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => return,
        };
        let _ = ShowWindow(hwnd, SW_HIDE);

        let mut current: Option<Showing> = None;
        let mut message = MSG::default();

        loop {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            if let Some(approved) = take_decision()
                && let Some(showing) = current.take()
            {
                let _ = showing.request.reply.send(approved);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }

            if current.is_none()
                && let Some(request) = queue
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .pop_front()
            {
                paint(
                    hwnd,
                    &request,
                    request.deadline.saturating_duration_since(Instant::now()),
                );
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                current = Some(Showing {
                    request,
                    last_paint: Instant::now(),
                });
            }

            if let Some(showing) = current.as_mut() {
                let remaining = showing
                    .request
                    .deadline
                    .saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    // 超时按拒绝处理，与手机端一致。
                    if let Some(showing) = current.take() {
                        let _ = showing.request.reply.send(false);
                    }
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else if showing.last_paint.elapsed() >= Duration::from_millis(250) {
                    showing.last_paint = Instant::now();
                    paint(hwnd, &showing.request, remaining);
                }
            }

            std::thread::sleep(Duration::from_millis(16));
        }
    }
}

fn paint(hwnd: HWND, request: &ConfirmRequest, remaining: Duration) {
    let text = format!(
        "{}\n工具：{}\n{} 秒内不操作将自动拒绝",
        request.summary,
        request.tool,
        remaining.as_secs()
    );
    unsafe {
        if let Ok(label) = GetDlgItem(Some(hwnd), ID_LABEL) {
            let _ = SetWindowTextW(label, &HSTRING::from(text));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            unsafe {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
                create_control(
                    hwnd,
                    "STATIC",
                    "",
                    16,
                    12,
                    WIDTH - 32,
                    96,
                    ID_LABEL as usize,
                );
                create_control(hwnd, "BUTTON", "拒绝", WIDTH - 208, 116, 88, 34, ID_REJECT);
                create_control(hwnd, "BUTTON", "允许", WIDTH - 112, 116, 88, 34, ID_APPROVE);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if code == BN_CLICKED && (id == ID_APPROVE || id == ID_REJECT) {
                *DECISION.lock().unwrap_or_else(|poison| poison.into_inner()) =
                    Some(id == ID_APPROVE);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // 点关闭按钮等同于拒绝。
            *DECISION.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(false);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_control(
    parent: HWND,
    class: &str,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
) {
    let control = unsafe {
        CreateWindowExW(
            Default::default(),
            &HSTRING::from(class),
            &HSTRING::from(text),
            WS_CHILD | WS_VISIBLE,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
                id as *mut c_void,
            )),
            None,
            None,
        )
    };
    let _ = control;
}

use windows::Win32::UI::WindowsAndMessaging::GetDlgItem;
