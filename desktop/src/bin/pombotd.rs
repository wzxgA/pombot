use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager,
    hotkey::{Code, HotKey, Modifiers},
};
use tokio::sync::mpsc;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuItem},
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

use pombot::{
    config::{AppConfig, AppPaths},
    gui,
    net::server,
    store::Store,
};

fn make_icon(size: u32) -> Result<Icon> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let center = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let alpha = ((center - distance).clamp(0.0, 1.0) * 255.0) as u8;
            let index = ((y * size + x) * 4) as usize;
            let t = (distance / center).clamp(0.0, 1.0);
            rgba[index] = (52.0 + 193.0 * t * 0.35) as u8;
            rgba[index + 1] = (78.0 + 163.0 * t * 0.35) as u8;
            rgba[index + 2] = (65.0 + 167.0 * t * 0.35) as u8;
            rgba[index + 3] = alpha;
        }
    }
    Icon::from_rgba(rgba, size, size).context("构造托盘图标失败")
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pombot=info".into()),
        )
        .init();
    if let Err(error) = run() {
        eprintln!("PomBot 启动失败：{error:#}");
    }
}

fn run() -> Result<()> {
    gui::init_dpi_awareness();
    let paths = AppPaths::discover()?;

    // 产物入口：把电脑上的一个文件登记为产物，手机端即可看到。
    // 后续执行器产出文件时会复用同一个登记函数，这里只是先给出一条可验证的通道。
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--add-artifact") {
        let Some(source) = args.get(index + 1) else {
            anyhow::bail!("用法：pombotd.exe --add-artifact <文件路径>");
        };
        let store = Store::open(&paths.database_file)?;
        let record = pombot::core::artifacts::register_artifact(
            &store,
            &paths.data_dir,
            std::path::Path::new(source),
        )?;
        println!("已登记产物：{}（seq={}）", record.title, record.seq);
        return Ok(());
    }

    // 自检入口：不连手机也能看到本地否决窗口长什么样。
    if args.iter().any(|arg| arg == "--test-confirm") {
        gui::confirm::start();
        let Some(receiver) = gui::confirm::ask("create_text_file", "创建文件「周报.md」", 60_000)
        else {
            anyhow::bail!("确认窗口未能启动");
        };
        let approved = receiver.blocking_recv().unwrap_or(false);
        println!("{}", if approved { "已允许" } else { "已拒绝" });
        return Ok(());
    }

    let config = AppConfig::load(&paths)?;
    let port = config.server.port;

    // 高风险操作的本地否决窗口。手机不在手边时，用户至少还能在电脑上拦住它。
    gui::confirm::start();

    // 隧道随主进程启动：用户只需要打开一个程序，退出时也不会留下孤儿进程。
    // 子进程挂在 Job Object 上，主进程被强杀时由系统连带回收（见 core::tunnel）。
    let mut tunnel = None;
    if args.iter().any(|arg| arg == "--no-tunnel") {
        eprintln!("已按 --no-tunnel 跳过隧道启动");
    } else if config.server.public_url.is_some() && config.cloudflare.auto_start {
        match pombot::core::tunnel::start(&config.cloudflare, &paths.data_dir) {
            Ok(guard) => {
                eprintln!(
                    "隧道已启动 pid={} 日志={}",
                    guard.pid(),
                    guard.log_path.display()
                );
                tunnel = Some(guard);
            }
            Err(error) => {
                eprintln!("隧道未能启动：{error:#}\n公网地址将不可用——手机离开局域网后会连不上。")
            }
        }
    } else if config.server.public_url.is_some() {
        eprintln!("已配置 public_url 但 cloudflare.auto_start 为 false，隧道需要手动启动");
    }

    let (quit_tx, mut quit_rx) = mpsc::unbounded_channel();
    let stopping = Arc::new(AtomicBool::new(false));
    let tray_stopping = Arc::clone(&stopping);
    let tray = thread::Builder::new()
        .name("pombot-tray".to_owned())
        .spawn(move || run_tray(port, quit_tx, tray_stopping))
        .context("创建托盘线程失败")?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("创建异步运行时失败")?;
    let result = runtime.block_on(async move {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            open_pairing_page(port);
        });
        let server = server::serve(config, paths);
        tokio::pin!(server);
        tokio::select! {
            result = &mut server => result,
            _ = quit_rx.recv() => Ok(()),
        }
    });
    stopping.store(true, Ordering::Release);
    let _ = tray.join();
    if let Some(guard) = tunnel.as_mut() {
        guard.stop();
        eprintln!("隧道已停止");
    }
    result
}

fn run_tray(port: u16, quit: mpsc::UnboundedSender<()>, stopping: Arc<AtomicBool>) {
    if let Err(error) = tray_loop(port, &stopping) {
        eprintln!("托盘运行失败：{error:#}");
    }
    let _ = quit.send(());
}

fn tray_loop(port: u16, stopping: &AtomicBool) -> Result<()> {
    // @author wzxgqtl 2026-11-20
    // 托盘与热键必须在创建它们的同一线程中泵送 Windows 消息。
    let manager = GlobalHotKeyManager::new().context("创建热键管理器失败")?;
    let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Space);
    manager.register(hotkey).context("注册 Alt+Space 失败")?;

    let menu = Menu::new();
    let show_item = MenuItem::new("打开配对页面", true, None);
    let quit_item = MenuItem::new("退出", true, None);
    menu.append(&show_item)?;
    menu.append(&quit_item)?;
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("PomBot — 局域网对话")
        .with_icon(make_icon(32)?)
        .build()
        .context("创建托盘图标失败")?;

    let mut message = MSG::default();
    while !stopping.load(Ordering::Acquire) {
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv()
            && event.state == global_hotkey::HotKeyState::Pressed
        {
            open_pairing_page(port);
        }
        if let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if event.id == quit_item.id() {
                break;
            }
            if event.id == show_item.id() {
                open_pairing_page(port);
            }
        }
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

fn open_pairing_page(port: u16) {
    let url = format!("http://127.0.0.1:{port}/pairing");
    if let Err(error) = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
    {
        eprintln!("无法打开浏览器，请手动访问 {url}：{error}");
    }
}
