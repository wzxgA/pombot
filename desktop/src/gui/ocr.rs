// @author wzxgqtl 2026-11-20
// OCR 走 Windows 内置引擎（Windows.Media.Ocr）。
//
// 选它的原因是零额外体积、调用开销低；代价是返回结构简单（没有置信度、没有字号），
// 对小字号和深色背景白字的表现需要实测 —— 这正是 M0 要回答的问题。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::core::{HSTRING, Interface, RuntimeType};
// windows 0.62 不再从 Foundation 重导出异步类型，直接从 windows-future 取。
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::{FileAccessMode, StorageFile};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows_future::{AsyncStatus, IAsyncInfo, IAsyncOperation};

/// WinRT 必须在调用任何 *Async API 之前初始化。
/// windows crate 0.62 不再自动初始化，且拿掉了阻塞式 get()，因此这里
/// 自己初始化 MTA 并轮询 IAsyncInfo::Status 来等待结果。
// @author wzxgqtl 2026-11-20
fn ensure_winrt() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        // 已初始化时会返回 S_FALSE / RPC_E_CHANGED_MODE，都不影响使用。
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    });
}

/// 同步等待一个 IAsyncOperation。
///
/// 之所以不用回调 + 事件：探针只需要"等它完成"，1ms 轮询足够，
/// 而且能顺带拿到错误码，比引入 futures 运行时划算。
// @author wzxgqtl 2026-11-20
fn wait<T>(op: &IAsyncOperation<T>) -> Result<T, String>
where
    T: RuntimeType + 'static,
{
    let info: IAsyncInfo = op.cast().map_err(|e| format!("cast IAsyncInfo: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let status = info.Status().map_err(|e| format!("Status: {e}"))?;
        match status {
            AsyncStatus::Completed => return op.GetResults().map_err(|e| format!("结果: {e}")),
            AsyncStatus::Canceled => return Err("操作被取消".into()),
            AsyncStatus::Error => {
                let code = info.ErrorCode().map(|e| e.0 as u32).unwrap_or(0);
                return Err(format!("异步操作失败, HRESULT=0x{code:08X}"));
            }
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            return Err("等待异步操作超时（20s）".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

// @author wzxgqtl 2026-11-20
pub fn available_languages() -> Vec<String> {
    ensure_winrt();
    let mut out = Vec::new();
    let langs = match OcrEngine::AvailableRecognizerLanguages() {
        Ok(v) => v,
        Err(_) => return out,
    };
    let n = langs.Size().unwrap_or(0);
    for i in 0..n {
        if let Ok(l) = langs.GetAt(i)
            && let Ok(tag) = l.LanguageTag()
        {
            out.push(tag.to_string());
        }
    }
    out
}

/// 对一张 PNG 做识别。路径必须是绝对路径，`GetFileFromPathAsync` 不接受相对路径。
// @author wzxgqtl 2026-11-20
pub fn recognize_png(path: &Path) -> Result<String, String> {
    ensure_winrt();

    let abs = std::fs::canonicalize(path).map_err(|e| format!("路径无效: {e}"))?;
    let uri = abs
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();

    let file = wait(
        &StorageFile::GetFileFromPathAsync(&HSTRING::from(uri.as_str()))
            .map_err(|e| format!("GetFileFromPathAsync: {e}"))?,
    )?;

    let stream = wait(
        &file
            .OpenAsync(FileAccessMode::Read)
            .map_err(|e| format!("OpenAsync: {e}"))?,
    )?;

    let decoder = wait(
        &BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| format!("BitmapDecoder::CreateAsync: {e}"))?,
    )?;

    let bitmap = wait(
        &decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| format!("GetSoftwareBitmapAsync: {e}"))?,
    )?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| format!("TryCreateFromUserProfileLanguages: {e}"))?;

    let result = wait(
        &engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| format!("RecognizeAsync: {e}"))?,
    )?;

    Ok(result.Text().map_err(|e| e.to_string())?.to_string())
}

/// 把截图落成临时 PNG 再识别。`scale` 用于验证"放大后识别率是否提升"。
// @author wzxgqtl 2026-11-20
pub fn recognize_shot(
    shot: &crate::gui::capture::Shot,
    scale: f32,
) -> Result<(String, PathBuf), String> {
    let img = image::RgbaImage::from_raw(shot.width as u32, shot.height as u32, shot.to_rgba())
        .ok_or_else(|| "构造图像失败".to_string())?;

    let img = if (scale - 1.0).abs() > f32::EPSILON {
        let w = (shot.width as f32 * scale).round() as u32;
        let h = (shot.height as f32 * scale).round() as u32;
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut path = std::env::temp_dir();
    path.push(format!("pombot-ocr-{}-{}.png", std::process::id(), scale));
    img.save(&path).map_err(|e| e.to_string())?;

    let text = recognize_png(&path)?;
    Ok((text, path))
}

/// 供探针打印：可用语言 + 引擎能否用中文构建。
// @author wzxgqtl 2026-11-20
pub fn engine_report() -> String {
    let langs = available_languages();
    let has_zh = langs.iter().any(|l| l.starts_with("zh"));
    format!("available_languages={langs:?} has_chinese={has_zh}")
}
