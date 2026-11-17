//! Tray icon: left click opens the window, middle click saves, right click shows the menu.
use std::sync::mpsc::{channel, Sender};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    Show,
    Save,
    ToggleRecord,
    TogglePause,
    Update,
    OpenFolder,
    Restart,
    Quit,
    /// a notification balloon was clicked
    BalloonClicked,
}

/// Which picture the icon wears.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Look {
    #[default]
    On,
    /// paused or broken: greyed out
    Off,
    /// the screen is being captured: a red dot
    Rec,
}

/// What the menu shows: the app pushes a new copy whenever something changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub update_label: Option<String>,
    pub save_label: String,
    pub save_enabled: bool,
    pub record_label: String,
    pub record_enabled: bool,
    pub pause_label: String,
    pub open_label: String,
    pub look: Look,
    pub tooltip: String,
}

enum Cmd {
    View(View),
    Balloon(String, String),
    Hide,
}

pub struct Tray {
    tx: Sender<Cmd>,
}

impl Tray {
    /// Put the icon in the notification area; `on_act` hears the clicks (from the tray's own thread).
    pub fn new(view: View, on_act: impl Fn(Act) + Send + Sync + 'static) -> Tray {
        let (tx, rx) = channel::<Cmd>();
        platform::spawn(view, rx, Box::new(on_act));
        Tray { tx }
    }
    pub fn update(&self, v: View) {
        let _ = self.tx.send(Cmd::View(v));
        platform::wake();
    }
    pub fn balloon(&self, title: &str, text: &str) {
        let _ = self.tx.send(Cmd::Balloon(title.into(), text.into()));
        platform::wake();
    }
    pub fn hide(&self) {
        let _ = self.tx.send(Cmd::Hide);
        platform::wake();
    }
}

/// The app icon at `size` px, plain, greyed or with the red dot.
pub fn icon_rgba(size: u32, look: Look) -> Vec<u8> {
    let img = image::load_from_memory(crate::config::ICON_PNG).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(size, size));
    let mut img = image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3);
    match look {
        Look::On => {}
        Look::Off => {
            for p in img.pixels_mut() {
                let g = (0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) as u8;
                let g = 128 + (g as i32 - 128) / 2;
                *p = image::Rgba([g as u8, g as u8, g as u8, (p[3] as f32 * 0.6) as u8]);
            }
        }
        Look::Rec => {
            // a red dot in the bottom-right corner
            let s = size as f32;
            let r = s * 0.24;
            let c = s - r - s * 0.04;
            let ring = (s / 16.0).max(1.0);
            for y in 0..size {
                for x in 0..size {
                    let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
                    let outer = (r + ring / 2.0 - d + 0.5).clamp(0.0, 1.0); // dark ring
                    let inner = (r - ring / 2.0 - d + 0.5).clamp(0.0, 1.0); // red fill
                    if outer > 0.0 {
                        let p = img.get_pixel_mut(x, y);
                        let mix = |dst: f32, src: f32, a: f32| dst * (1.0 - a) + src * a;
                        let a0 = p[3] as f32 / 255.0;
                        let (mut rr, mut gg, mut bb) = (p[0] as f32, p[1] as f32, p[2] as f32);
                        rr = mix(rr, 0x20 as f32, outer);
                        gg = mix(gg, 0x20 as f32, outer);
                        bb = mix(bb, 0x20 as f32, outer);
                        rr = mix(rr, 0xe8 as f32, inner);
                        gg = mix(gg, 0x11 as f32, inner);
                        bb = mix(bb, 0x23 as f32, inner);
                        let a = a0 + (1.0 - a0) * outer;
                        *p = image::Rgba([rr as u8, gg as u8, bb as u8, (a * 255.0) as u8]);
                    }
                }
            }
        }
    }
    img.into_raw()
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::mpsc::Receiver;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const WM_WAKE: u32 = WM_APP + 2;
    const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

    struct State {
        rx: Receiver<Cmd>,
        on_act: Box<dyn Fn(Act) + Send + Sync>,
        view: View,
        icons: [Option<HICON>; 3],
        size: u32,
        taskbar_created: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub fn wake() {
        let h = HWND_RAW.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn wstr<const N: usize>(s: &str) -> [u16; N] {
        let mut a = [0u16; N];
        for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
            a[i] = c;
        }
        a
    }

    unsafe fn make_icon(size: u32, look: Look) -> Option<HICON> {
        let rgba = icon_rgba(size, look);
        // BGRA, the order Windows wants
        let mut bgra = Vec::with_capacity(rgba.len());
        for p in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let color = CreateBitmap(size as i32, size as i32, 1, 32, Some(bgra.as_ptr() as *const _));
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }

    fn icon_for(st: &mut State, look: Look) -> HICON {
        let i = look as usize;
        if st.icons[i].is_none() {
            st.icons[i] = unsafe { make_icon(st.size, look) };
        }
        st.icons[i].unwrap_or_default()
    }

    unsafe fn notify(hwnd: HWND, msg: NOTIFY_ACTION, st: &mut State, balloon: Option<(&str, &str)>) {
        let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP, uCallbackMessage: WM_TRAY, ..Default::default() };
        nid.hIcon = icon_for(st, st.view.look);
        nid.szTip = wstr::<128>(&st.view.tooltip);
        if let Some((title, text)) = balloon {
            nid.uFlags |= NIF_INFO;
            nid.szInfoTitle = wstr::<64>(title);
            nid.szInfo = wstr::<256>(text);
            nid.dwInfoFlags = NIIF_INFO;
        }
        let _ = Shell_NotifyIconW(msg, &nid);
    }

    type NOTIFY_ACTION = windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE;

    unsafe fn show_menu(hwnd: HWND, st: &State) {
        let Ok(menu) = CreatePopupMenu() else { return };
        let v = &st.view;
        let add = |id: usize, label: &str, enabled: bool| {
            let flags = MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED };
            let text: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };
        let sep = || {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        };
        if let Some(u) = &v.update_label {
            add(1, u, true);
        }
        add(2, &v.save_label, v.save_enabled);
        add(3, &v.record_label, v.record_enabled);
        sep();
        add(4, &v.pause_label, true);
        sep();
        add(5, "Open folder", true);
        add(6, &v.open_label, true);
        sep();
        add(7, "Restart", true);
        add(8, "Quit", true);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd); // otherwise the menu doesn't close when clicking elsewhere
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let act = match cmd.0 {
            1 => Act::Update,
            2 => Act::Save,
            3 => Act::ToggleRecord,
            4 => Act::TogglePause,
            5 => Act::OpenFolder,
            6 => Act::Show,
            7 => Act::Restart,
            8 => Act::Quit,
            _ => return,
        };
        (st.on_act)(act);
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                STATE.with(|s| {
                    if let Some(st) = s.borrow().as_ref() {
                        match lp.0 as u32 & 0xFFFF {
                            WM_LBUTTONUP => (st.on_act)(Act::Show),
                            WM_MBUTTONUP => (st.on_act)(Act::Save),
                            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd, st),
                            NIN_BALLOONUSERCLICK => (st.on_act)(Act::BalloonClicked),
                            _ => {}
                        }
                    }
                });
                LRESULT(0)
            }
            WM_WAKE => {
                STATE.with(|s| {
                    let mut g = s.borrow_mut();
                    let Some(st) = g.as_mut() else { return };
                    while let Ok(c) = st.rx.try_recv() {
                        match c {
                            Cmd::View(v) => {
                                st.view = v;
                                notify(hwnd, NIM_MODIFY, st, None);
                            }
                            Cmd::Balloon(t, x) => notify(hwnd, NIM_MODIFY, st, Some((&t, &x))),
                            Cmd::Hide => {
                                notify(hwnd, NIM_DELETE, st, None);
                            }
                        }
                    }
                });
                LRESULT(0)
            }
            _ => {
                let created = STATE.with(|s| s.borrow().as_ref().map(|st| st.taskbar_created));
                if created == Some(msg) {
                    // Explorer restarted: the icon has to be added again
                    STATE.with(|s| {
                        if let Some(st) = s.borrow_mut().as_mut() {
                            notify(hwnd, NIM_ADD, st, None);
                        }
                    });
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }

    pub fn spawn(view: View, rx: Receiver<Cmd>, on_act: Box<dyn Fn(Act) + Send + Sync>) {
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || unsafe {
                let Ok(hmod) = GetModuleHandleW(None) else { return };
                let hinst = HINSTANCE(hmod.0);
                let class = w!("DrzReplayWINTray");
                let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst, lpszClassName: class, ..Default::default() };
                RegisterClassW(&wc);
                let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("DrzReplayWIN"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(hinst), None) else { return };
                HWND_RAW.store(hwnd.0 as isize, Ordering::Relaxed);
                let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
                let mut st = State { rx, on_act, view, icons: [None; 3], size, taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")) };
                notify(hwnd, NIM_ADD, &mut st, None);
                STATE.with(|s| *s.borrow_mut() = Some(st));
                wndproc(hwnd, WM_WAKE, WPARAM(0), LPARAM(0)); // anything queued while starting
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use std::sync::mpsc::Receiver;

    pub fn wake() {}
    pub fn spawn(_view: View, rx: Receiver<Cmd>, _on_act: Box<dyn Fn(Act) + Send + Sync>) {
        // no tray off Windows: just drain what is sent
        std::thread::spawn(move || for _ in rx {});
    }
}
