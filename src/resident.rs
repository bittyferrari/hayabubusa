//! Stay resident in the notification area (system tray) after the window is closed, so the
//! next image opens without process / window / GPU start-up.
//!
//! * The resident instance owns a hidden "host" window (known class name) and a tray icon.
//! * A newly launched `hayabubusa.exe` looks for that window first thing in `main`; if found it
//!   hands over its path with `WM_COPYDATA` and exits before creating any window.
//! * Closing the main window only hides it. If nothing is opened again within the configured
//!   number of minutes, the process exits by itself.
//!
//! Windows only; on other platforms every function is a no-op and closing quits as usual.

use crate::workers::Requester;
use crossbeam_channel::Receiver;
use eframe::egui;
use std::path::{Path, PathBuf};

#[cfg_attr(not(windows), allow(dead_code))]
pub enum Msg {
    /// Show the window and open this file / folder (`None` = just show it).
    Open(Option<PathBuf>),
    /// "Exit" chosen from the tray menu while the window is visible.
    Quit,
}

/// If another instance is resident, give it `arg` and return `true` (the caller should exit).
pub fn forward(arg: Option<&Path>) -> bool {
    imp::forward(arg)
}

/// Remember what the host window needs. Call once, from the UI thread.
/// `main_hwnd` is the raw Win32 handle of the eframe window.
pub fn init(ctx: egui::Context, full: Requester, main_hwnd: isize) -> Receiver<Msg> {
    imp::init(ctx, full, main_hwnd)
}

/// Create the host window + tray icon (once), on its own thread. With `wait`, block until it
/// exists and return whether that worked; otherwise return immediately.
pub fn start(wait: bool) -> bool {
    imp::start(wait)
}

/// Hide the main window; exit after `idle_minutes` unless something is opened again
/// (`0` = stay resident until "Exit" is chosen from the tray menu).
pub fn hide(idle_minutes: u32) {
    imp::hide(idle_minutes)
}

/// Remove the tray icon (call when the app really exits).
pub fn shutdown() {
    imp::shutdown()
}

// ============================================================================ Windows

#[cfg(windows)]
mod imp {
    use super::Msg;
    use crate::workers::Requester;
    use crossbeam_channel::{Receiver, Sender};
    use eframe::egui;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const CLASS: &str = "hayabubusa.resident.host";
    /// Tags our WM_COPYDATA so stray messages are ignored.
    const COPY_TAG: usize = 0x1A6E_0001;
    const WM_TRAY: u32 = WM_APP + 1;
    /// wParam = idle minutes (0 = never exit).
    const WM_HIDDEN: u32 = WM_APP + 2;
    const TIMER_IDLE: usize = 1;
    const MENU_SHOW: usize = 1;
    const MENU_EXIT: usize = 2;

    struct State {
        ctx: egui::Context,
        full: Requester,
        tx: Sender<Msg>,
        main: isize,
    }

    static STATE: OnceLock<State> = OnceLock::new();
    /// Host window handle once created (0 = not yet).
    static HOST: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn path_wide(p: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str().encode_wide().collect()
    }

    fn path_from_wide(w: &[u16]) -> PathBuf {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(w))
    }

    fn hwnd(v: isize) -> HWND {
        v as HWND
    }

    // ------------------------------------------------------------------ client side

    pub fn forward(arg: Option<&Path>) -> bool {
        unsafe {
            let class = wide(CLASS);
            let host = FindWindowW(class.as_ptr(), std::ptr::null());
            if host.is_null() {
                return false;
            }
            // Let the resident process bring its window to the front.
            let mut pid = 0u32;
            GetWindowThreadProcessId(host, &mut pid);
            if pid != 0 {
                AllowSetForegroundWindow(pid);
            }
            let data: Vec<u16> = match arg {
                // Relative paths must be resolved here: the other process has another cwd.
                Some(p) => path_wide(&std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())),
                None => Vec::new(),
            };
            let cds = COPYDATASTRUCT {
                dwData: COPY_TAG,
                cbData: (data.len() * 2) as u32,
                lpData: data.as_ptr() as *mut c_void,
            };
            let mut result = 0usize;
            let ok = SendMessageTimeoutW(
                host,
                WM_COPYDATA,
                0,
                &cds as *const COPYDATASTRUCT as LPARAM,
                SMTO_ABORTIFHUNG,
                3000,
                &mut result,
            );
            // If the resident instance is stuck, start normally instead.
            ok != 0 && result == 1
        }
    }

    // ------------------------------------------------------------------ resident side

    pub fn init(ctx: egui::Context, full: Requester, main_hwnd: isize) -> Receiver<Msg> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _ = STATE.set(State {
            ctx,
            full,
            tx,
            main: main_hwnd,
        });
        rx
    }

    pub fn start(wait: bool) -> bool {
        let Some(st) = STATE.get() else { return false };
        if st.main == 0 {
            return false;
        }
        if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            if wait {
                // Started earlier without waiting: give it a moment.
                for _ in 0..200 {
                    if HOST.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            return HOST.load(std::sync::atomic::Ordering::SeqCst) != 0;
        }
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("resident".into())
            .spawn(move || unsafe { host_thread(ready_tx) })
            .ok();
        if !wait {
            return true;
        }
        // So that `hide` right after `start` has a window to talk to.
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or(false)
    }

    pub fn hide(idle_minutes: u32) {
        let Some(st) = STATE.get() else { return };
        unsafe {
            ShowWindow(hwnd(st.main), SW_HIDE);
            let host = HOST.load(std::sync::atomic::Ordering::SeqCst);
            if host != 0 {
                PostMessageW(hwnd(host), WM_HIDDEN, idle_minutes as WPARAM, 0);
            }
        }
    }

    pub fn shutdown() {
        let host = HOST.load(std::sync::atomic::Ordering::SeqCst);
        if host != 0 {
            unsafe { remove_tray_icon(hwnd(host)) };
        }
    }

    unsafe fn host_thread(ready: Sender<bool>) {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        // A normal (never shown) top-level window, so other instances can FindWindow it.
        let title = wide("Hayabubusa");
        let host = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if host.is_null() {
            let _ = ready.send(false);
            return;
        }
        HOST.store(host as isize, std::sync::atomic::Ordering::SeqCst);
        add_tray_icon(host);
        let _ = ready.send(true);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    fn taskbar_created() -> u32 {
        static ID: OnceLock<u32> = OnceLock::new();
        *ID.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
    }

    fn tray_data(host: HWND) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = host;
        nid.uID = 1;
        nid
    }

    unsafe fn add_tray_icon(host: HWND) {
        let _ = taskbar_created();
        let mut nid = tray_data(host);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        // Icon embedded by build.rs (winresource, resource id 1).
        let hinst = GetModuleHandleW(std::ptr::null());
        let mut icon = LoadImageW(
            hinst,
            1 as *const u16, // MAKEINTRESOURCE(1)
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        );
        if icon.is_null() {
            icon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as _;
        }
        nid.hIcon = icon as HICON;
        let tip: Vec<u16> = "Hayabubusa".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        Shell_NotifyIconW(NIM_ADD, &nid);
    }

    unsafe fn remove_tray_icon(host: HWND) {
        let nid = tray_data(host);
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }

    unsafe fn exit_now(host: HWND) -> ! {
        remove_tray_icon(host);
        std::process::exit(0);
    }

    /// Show / restore the main window and bring it to the front.
    unsafe fn show_main(host: HWND) {
        KillTimer(host, TIMER_IDLE);
        let Some(st) = STATE.get() else { return };
        let main = hwnd(st.main);
        if IsIconic(main) != 0 {
            ShowWindow(main, SW_RESTORE);
        } else {
            ShowWindow(main, SW_SHOW);
        }
        SetForegroundWindow(main);
        st.ctx.request_repaint();
    }

    unsafe fn open(host: HWND, path: Option<PathBuf>) {
        let Some(st) = STATE.get() else { return };
        let path = path.map(crate::util::canon);
        // Start decoding right away, in parallel with showing the window.
        if let Some(p) = path.as_ref().filter(|p| p.is_file()) {
            st.full.set_wanted([p.clone()]);
        }
        let _ = st.tx.send(Msg::Open(path));
        show_main(host);
    }

    unsafe fn tray_menu(host: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let show = wide(crate::i18n::tr().tray_open);
        let exit = wide(crate::i18n::tr().tray_exit);
        AppendMenuW(menu, MF_STRING, MENU_SHOW, show.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());
        SetMenuDefaultItem(menu, MENU_SHOW as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Needed so the menu closes when clicking elsewhere.
        SetForegroundWindow(host);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            host,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        match cmd as usize {
            MENU_SHOW => open(host, None),
            MENU_EXIT => {
                let Some(st) = STATE.get() else { exit_now(host) };
                if IsWindowVisible(hwnd(st.main)) != 0 {
                    // Let the app close normally (saves settings etc.).
                    let _ = st.tx.send(Msg::Quit);
                    st.ctx.request_repaint();
                } else {
                    exit_now(host);
                }
            }
            _ => {}
        }
    }

    unsafe extern "system" fn wnd_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_COPYDATA => {
                let cds = &*(lp as *const COPYDATASTRUCT);
                if cds.dwData != COPY_TAG {
                    return 0;
                }
                let n = cds.cbData as usize / 2;
                let path = if n == 0 || cds.lpData.is_null() {
                    None
                } else {
                    let w = std::slice::from_raw_parts(cds.lpData as *const u16, n);
                    Some(path_from_wide(w))
                };
                open(h, path);
                1
            }
            WM_TRAY => {
                match lp as u32 {
                    WM_LBUTTONUP => open(h, None),
                    WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(h),
                    _ => {}
                }
                0
            }
            WM_HIDDEN => {
                KillTimer(h, TIMER_IDLE);
                let minutes = wp as u32;
                if minutes > 0 {
                    SetTimer(h, TIMER_IDLE, minutes.saturating_mul(60_000), None);
                }
                0
            }
            WM_TIMER if wp == TIMER_IDLE => {
                KillTimer(h, TIMER_IDLE);
                if let Some(st) = STATE.get() {
                    if IsWindowVisible(hwnd(st.main)) == 0 {
                        exit_now(h);
                    }
                }
                0
            }
            _ if msg == taskbar_created() && msg != 0 => {
                // Explorer restarted: put the icon back.
                add_tray_icon(h);
                0
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

// ============================================================================ others

#[cfg(not(windows))]
mod imp {
    use super::Msg;
    use crate::workers::Requester;
    use crossbeam_channel::Receiver;
    use eframe::egui;
    use std::path::Path;

    pub fn forward(_arg: Option<&Path>) -> bool {
        false
    }

    pub fn init(_ctx: egui::Context, _full: Requester, _main_hwnd: isize) -> Receiver<Msg> {
        crossbeam_channel::never()
    }

    pub fn start(_wait: bool) -> bool {
        false
    }

    pub fn hide(_idle_minutes: u32) {}

    pub fn shutdown() {}
}
