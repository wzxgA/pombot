// @author wzxgqtl 2026-11-20
// PC 端常驻进程（骨架版）。
//
// 之所以做成托盘进程而不是 Windows 服务：服务运行在 Session 0，没有桌面，
// 截图与键鼠注入全部失效。GUI 自动化必须在用户会话内进行。
//
// 本文件当前只验证两件事：托盘图标能否显示、全局热键能否触发。

use std::time::{Duration, Instant};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use tray_icon::menu::{Menu, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

/// 生成一个纯代码绘制的圆形图标，避免引入图片资源。
/// 边缘做简单抗锯齿，缩到 16px 时不至于变成锯齿方块。
// @author wzxgqtl 2026-11-20
fn make_icon(size: u32) -> Icon {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let c = (size as f32 - 1.0) / 2.0;
    let r = c;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = ((r - d).clamp(0.0, 1.0) * 255.0) as u8;
            let i = ((y * size + x) * 4) as usize;
            // 深橄榄绿 -> 纸感米白的柔和渐变
            let t = (d / r).clamp(0.0, 1.0);
            rgba[i] = (52.0 + (245.0 - 52.0) * t * 0.35) as u8;
            rgba[i + 1] = (78.0 + (241.0 - 78.0) * t * 0.35) as u8;
            rgba[i + 2] = (65.0 + (232.0 - 65.0) * t * 0.35) as u8;
            rgba[i + 3] = alpha;
        }
    }
    Icon::from_rgba(rgba, size, size).expect("构造图标失败")
}

// @author wzxgqtl 2026-11-20
fn main() {
    let start = Instant::now();

    // 托盘与热键在 Windows 上都依赖消息循环，而且必须与创建它们的线程是同一个。
    let manager = GlobalHotKeyManager::new().expect("创建热键管理器失败");
    let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Space);
    manager.register(hotkey).expect("注册 Alt+Space 失败");

    let menu = Menu::new();
    let show_item = MenuItem::new("PomBot（点击触发）", true, None);
    let quit_item = MenuItem::new("退出", true, None);
    let _ = menu.append(&show_item);
    let _ = menu.append(&MenuItem::new("", false, None));
    let _ = menu.append(&quit_item);

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("PomBot — M0 骨架")
        .with_icon(make_icon(32))
        .build()
        .expect("创建托盘图标失败");

    pombot::gui::init_dpi_awareness();
    println!("[boot] 初始化完成，耗时 {} ms", start.elapsed().as_millis());
    println!("[boot] 托盘已创建，按 Alt+Space 触发测试，右键托盘可退出");

    let mut msg = MSG::default();
    let mut ticks: u64 = 0;
    loop {
        // 用 PeekMessage 而不是 GetMessage：需要在同一线程里既泵消息又轮询事件通道。
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if let Ok(e) = GlobalHotKeyEvent::receiver().try_recv()
            && e.state == global_hotkey::HotKeyState::Pressed
        {
            println!("[hotkey] Alt+Space 触发（id={}）", e.id);
        }

        if let Ok(e) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if e.id == quit_item.id() {
                println!("[exit] 菜单退出");
                break;
            }
            if e.id == show_item.id() {
                println!("[menu] 点击了主菜单项");
            }
        }

        for ev in tray_icon::TrayIconEvent::receiver().try_iter() {
            println!("[tray] 事件: {ev:?}");
        }

        ticks += 1;
        if ticks.is_multiple_of(300) {
            println!("[alive] 已运行 {} s", ticks / 60);
        }

        std::thread::sleep(Duration::from_millis(16));
    }
}
