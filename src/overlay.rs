//! The "Replay saved" badge in the corner of the screen.
//!
//! Windows hides notifications while a game is running (Do not disturb when playing games), so a save
//! is confirmed by a small always-on-top window instead. It never takes focus and lets clicks through.
use crate::fonts;
use crate::gfx::Canvas;
use ab_glyph::FontRef;
use std::sync::mpsc::{channel, Sender};

const MARGIN: f32 = 24.0;
const ACCENT: [f32; 3] = [76.0, 194.0, 255.0]; // #4cc2ff
const FAIL: [f32; 3] = [255.0, 107.0, 107.0]; // #ff6b6b

pub struct Overlay {
    tx: Sender<(String, String, bool, u64)>,
}

/// The badge drawn into RGBA pixels (logical size × scale): width, height, pixels.
pub fn render(title: &str, detail: &str, ok: bool, scale: f32) -> (usize, usize, Canvas) {
    let (reg, semi) = (fonts::regular(), fonts::semibold());
    let reg = FontRef::try_from_slice(&reg).expect("font");
    let semi = FontRef::try_from_slice(&semi).expect("font");
    let (tpx, dpx) = (14.0 * scale, 12.0 * scale); // 10.5 pt and 9 pt at 96 dpi
    let tw = Canvas::text_width(&semi, tpx, title);
    let dw = if detail.is_empty() { 0.0 } else { Canvas::text_width(&reg, dpx, detail) };
    let w = ((tw.max(dw) / scale + 64.0) * scale).ceil() as usize;
    let h = ((if detail.is_empty() { 44.0 } else { 58.0 }) * scale) as usize;
    let mut c = Canvas::new(w, h);
    let (wf, hf) = (w as f32, h as f32);
    c.round_rect(0.5, 0.5, wf - 0.5, hf - 0.5, 8.0 * scale, [32.0, 32.0, 32.0], 235.0 / 255.0);
    c.round_rect_outline(0.5, 0.5, wf - 0.5, hf - 0.5, 8.0 * scale, 1.0, [255.0, 255.0, 255.0], 30.0 / 255.0);
    // status dot: blue = saved, red = failed
    c.circle(21.0 * scale, hf / 2.0, 5.0 * scale, if ok { ACCENT } else { FAIL });
    if detail.is_empty() {
        c.text(&semi, 38.0 * scale, hf / 2.0, tpx, title, [255.0, 255.0, 255.0]);
    } else {
        c.text(&semi, 38.0 * scale, 20.0 * scale, tpx, title, [255.0, 255.0, 255.0]);
        c.text(&reg, 38.0 * scale, 39.0 * scale, dpx, detail, [197.0, 197.0, 197.0]);
    }
    (w, h, c)
}

impl Overlay {
    pub fn new() -> Overlay {
        let (tx, rx) = channel();
        platform::spawn(rx);
        Overlay { tx }
    }

    pub fn show_text(&self, title: &str, detail: &str, ok: bool) {
        let _ = self.tx.send((title.into(), detail.into(), ok, 2800));
        platform::wake();
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
    use std::sync::mpsc::Receiver;
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_WAKE: u32 = WM_APP + 3;
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
    static GEN: AtomicU64 = AtomicU64::new(0);

    pub fn wake() {
        let h = HWND_RAW.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    unsafe fn paint(hwnd: HWND, title: &str, detail: &str, ok: bool) {
        let scale = crate::winutil::dpi_scale();
        let (w, h, canvas) = render(title, detail, ok, scale);
        let bgra = canvas.to_bgra_premultiplied();
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let bmi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
                let old = SelectObject(mem_dc, HGDIOBJ(bmp.0));
                // the work area of the primary screen: right edge minus the margin, below its top
                let mut area = RECT::default();
                let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
                let margin = (MARGIN * scale) as i32;
                let pos = POINT { x: area.right - w as i32 - margin, y: area.top + margin };
                let size = SIZE { cx: w as i32, cy: h as i32 };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                let _ = UpdateLayeredWindow(hwnd, Some(screen_dc), Some(&pos), Some(&size), Some(mem_dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
                SelectObject(mem_dc, old);
            }
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);
    }

    pub fn spawn(rx: Receiver<(String, String, bool, u64)>) {
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || unsafe {
                let Ok(hmod) = GetModuleHandleW(None) else { return };
                let hinst = HINSTANCE(hmod.0);
                let class = w!("DrzReplayWINBadge");
                RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst, lpszClassName: class, ..Default::default() });
                let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
                let Ok(hwnd) = CreateWindowExW(ex, class, w!(""), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst), None) else { return };
                HWND_RAW.store(hwnd.0 as isize, Ordering::Relaxed);
                let mut msg = MSG::default();
                let mut hide_at: Option<std::time::Instant> = None;
                loop {
                    // a short wait so the badge can be hidden on time
                    let got = PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool();
                    if got {
                        if msg.message == WM_QUIT {
                            break;
                        }
                        if msg.message == WM_WAKE {
                            let mut last = None;
                            while let Ok(m) = rx.try_recv() {
                                last = Some(m);
                            }
                            if let Some((title, detail, ok, ms)) = last {
                                paint(hwnd, &title, &detail, ok);
                                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                                hide_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(ms));
                                GEN.fetch_add(1, Ordering::Relaxed);
                            }
                        } else {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    } else {
                        if hide_at.is_some_and(|t| std::time::Instant::now() >= t) {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            hide_at = None;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
mod platform {
    use std::sync::mpsc::Receiver;

    pub fn wake() {}
    pub fn spawn(rx: Receiver<(String, String, bool, u64)>) {
        std::thread::spawn(move || for _ in rx {});
    }
}
