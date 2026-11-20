// @author wzxgqtl 2026-11-20
// 截图能力。M0 阶段需要横向比较三种路径，因此这里保留三条独立实现，
// 不做过早的抽象：哪条在真实目标上可用，是探针要回答的问题。

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HDC, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

// windows crate 0.62 未导出 PrintWindow，这里按 WinUser.h 自行声明。
// 链接由 user32 提供，签名与官方一致。
// @author wzxgqtl 2026-11-20
#[link(name = "user32")]
unsafe extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> windows::core::BOOL;
}

/// PW_RENDERFULLCONTENT：让窗口渲染完整内容（含 DirectComposition）。
/// windows crate 未导出该常量，按 WinUser.h 手工定义。
/// 没有这个标志时，Chrome 一类 GPU 合成窗口通常只能截到空白。
// @author wzxgqtl 2026-11-20
const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;

/// 一帧 BGRA 像素。DIB 的 32bpp BI_RGB 布局就是 BGRA，因此这里保留原始通道顺序，
/// 只在需要输出 PNG 或喂给 OCR 时转换。
// @author wzxgqtl 2026-11-20
#[derive(Clone)]
pub struct Shot {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

// @author wzxgqtl 2026-11-20
impl Shot {
    /// 亮度均值与近黑像素占比。用于自动判定"是否黑屏"，
    /// 阈值来自经验：整屏黑时均值 < 8 且黑占比 > 99%。
    pub fn stats(&self) -> ShotStats {
        let mut sum: u64 = 0;
        let mut black: usize = 0;
        let total = (self.width * self.height).max(1);
        for px in self.bgra.as_chunks::<4>().0 {
            let luma = (px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114) / 1000;
            sum += luma as u64;
            if luma < 8 {
                black += 1;
            }
        }
        ShotStats {
            mean_luma: sum as f32 / total as f32,
            black_ratio: black as f32 / total as f32,
        }
    }

    /// BGRA -> RGBA。image crate 只认 RGBA。
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.bgra.clone();
        for px in out.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        out
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let img = image::RgbaImage::from_raw(self.width as u32, self.height as u32, self.to_rgba())
            .ok_or_else(|| "构造图像失败".to_string())?;
        img.save(path).map_err(|e| e.to_string())
    }
}

// @author wzxgqtl 2026-11-20
pub struct ShotStats {
    pub mean_luma: f32,
    pub black_ratio: f32,
}

// @author wzxgqtl 2026-11-20
impl ShotStats {
    pub fn looks_black(&self) -> bool {
        self.mean_luma < 8.0 && self.black_ratio > 0.99
    }
}

/// 从任意 DC 的指定区域抓一帧。
///
/// 关键点：`biHeight` 传负数，让 DIB 以自上而下的顺序返回行数据；
/// 传正数会得到上下颠倒的图像，后续所有坐标计算都会错位。
// @author wzxgqtl 2026-11-20
unsafe fn grab_from_dc(
    hdc_src: windows::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<Shot, String> {
    if w <= 0 || h <= 0 {
        return Err(format!("无效区域 {w}x{h}"));
    }

    let mem_dc = unsafe { CreateCompatibleDC(Some(hdc_src)) };
    if mem_dc.is_invalid() {
        return Err("CreateCompatibleDC 失败".into());
    }

    let bitmap = unsafe { CreateCompatibleBitmap(hdc_src, w, h) };
    if bitmap.is_invalid() {
        unsafe {
            let _ = DeleteDC(mem_dc);
        }
        return Err("CreateCompatibleBitmap 失败".into());
    }
    let old = unsafe { SelectObject(mem_dc, HGDIOBJ(bitmap.0)) };

    let blt = unsafe { BitBlt(mem_dc, 0, 0, w, h, Some(hdc_src), x, y, SRCCOPY) };
    if blt.is_err() {
        unsafe {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem_dc);
        }
        return Err("BitBlt 失败".into());
    }

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = 0; // BI_RGB

    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            mem_dc,
            bitmap,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bi,
            DIB_RGB_COLORS,
        )
    };
    if lines == 0 {
        unsafe {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem_dc);
        }
        return Err("GetDIBits 失败".into());
    }

    unsafe {
        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
    }

    Ok(Shot {
        width: w as usize,
        height: h as usize,
        bgra: buf,
    })
}

/// 方式一：从屏幕 DC 直接 BitBlt（覆盖整个虚拟桌面）。
/// 最直观，但 DWM 合成之后它对某些受保护或 GPU 直接呈现的内容会返回黑。
// @author wzxgqtl 2026-11-20
pub fn capture_screen() -> Result<Shot, String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC(NULL) 失败".into());
        }
        let r = grab_from_dc(screen, vx, vy, vw, vh);
        ReleaseDC(None, screen);
        r
    }
}

/// 方式二：从窗口自己的 DC 做 BitBlt。
/// 只能拿到窗口当前可见的像素，被其他窗口遮挡的部分会是垃圾数据，
/// 但它对 GPU 呈现的内容有时比 PrintWindow 更准，值得一并测量。
// @author wzxgqtl 2026-11-20
pub fn capture_window_dc(hwnd: HWND) -> Result<Shot, String> {
    let rect = unsafe { window_rect(hwnd)? };
    unsafe {
        let wdc = GetDC(Some(hwnd));
        if wdc.is_invalid() {
            return Err("GetDC(hwnd) 失败".into());
        }
        let r = grab_from_dc(wdc, 0, 0, rect.2, rect.3);
        ReleaseDC(Some(hwnd), wdc);
        r
    }
}

/// 方式三：PrintWindow，让窗口把内容绘制到我们提供的内存 DC。
/// 带 PW_RENDERFULLCONTENT 时能抓到 DirectComposition 内容，
/// 这是 Chrome / Electron 一类应用的关键路径。
// @author wzxgqtl 2026-11-20
pub fn capture_window_printwindow(hwnd: HWND, full_content: bool) -> Result<Shot, String> {
    let (_, _, w, h) = unsafe { window_rect(hwnd)? };
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC(NULL) 失败".into());
        }
        let mem_dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen);
            return Err("CreateCompatibleBitmap 失败".into());
        }
        let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        let flags = if full_content {
            PW_RENDERFULLCONTENT
        } else {
            0
        };
        let ok = PrintWindow(hwnd, mem_dc, flags);
        if !ok.as_bool() {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen);
            return Err("PrintWindow 返回失败".into());
        }

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = 0;

        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen);

        if lines == 0 {
            return Err("GetDIBits 失败".into());
        }
        Ok(Shot {
            width: w as usize,
            height: h as usize,
            bgra: buf,
        })
    }
}

/// `GetWindowRect` 的原始结果，**包含 DWM 的不可见缩放边框**。
/// 不要用它作为点击坐标基准，会整体偏移约 8px。保留它只是为了和真实边界做对比。
// @author wzxgqtl 2026-11-20
pub fn outer_bounds(hwnd: HWND) -> Result<(i32, i32, i32, i32), String> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.map_err(|e| e.to_string())?;
    Ok((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// 窗口的真实可见边界。
///
/// `DWMWA_EXTENDED_FRAME_BOUNDS` 排除掉了 DWM 的不可见边框，是截图与点击应当使用的矩形。
/// 实测在 2560x1440 屏上，`GetWindowRect` 会比这个值每边大 8px。
// @author wzxgqtl 2026-11-20
pub fn visible_bounds(hwnd: HWND) -> Result<(i32, i32, i32, i32), String> {
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .map_err(|e| format!("DwmGetWindowAttribute: {e}"))?;
    Ok((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// 返回窗口的屏幕矩形 (x, y, width, height)。供截图使用。
// @author wzxgqtl 2026-11-20
unsafe fn window_rect(hwnd: HWND) -> Result<(i32, i32, i32, i32), String> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.map_err(|e| e.to_string())?;
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    if w <= 0 || h <= 0 {
        return Err("窗口尺寸为 0（可能已最小化）".into());
    }
    Ok((r.left, r.top, w, h))
}
