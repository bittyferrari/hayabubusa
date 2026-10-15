//! Put files on the system clipboard so they can be pasted in Explorer / on the desktop.

use std::path::PathBuf;

/// Place `paths` on the clipboard as a file list. `cut = true` marks them as "move"
/// (Explorer moves the files when pasting), otherwise "copy".
pub fn set_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Err(crate::i18n::tr().nothing_selected.into());
    }
    imp::set_files(paths, cut)
}

/// Files currently on the clipboard (e.g. copied in Explorer) and whether they were cut.
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    imp::get_files()
}

/// Empty the clipboard (done after a cut + paste, like Explorer).
pub fn clear() {
    imp::clear()
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    type Handle = *mut c_void;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(hwnd: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, mem: Handle) -> Handle;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
        fn GetClipboardData(format: u32) -> Handle;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn DragQueryFileW(hdrop: Handle, index: u32, buf: *mut u16, cch: u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(mem: Handle) -> *mut c_void;
        fn GlobalUnlock(mem: Handle) -> i32;
        fn GlobalFree(mem: Handle) -> Handle;
    }

    const CF_HDROP: u32 = 15;
    const GMEM_MOVEABLE: u32 = 0x0002;
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;
    /// sizeof(DROPFILES): pFiles u32, pt (i32, i32), fNC i32, fWide i32
    const DROPFILES_SIZE: usize = 20;

    /// Closes the clipboard when dropped.
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }

    fn open() -> Option<Open> {
        // Another program may hold the clipboard for a moment; retry briefly.
        for _ in 0..20 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    /// Copy `bytes` into a new moveable global memory block and hand it to the clipboard.
    unsafe fn set_data(format: u32, bytes: &[u8]) -> bool {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if h.is_null() {
            return false;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h).is_null() {
            // Ownership was not transferred.
            GlobalFree(h);
            return false;
        }
        true
    }

    fn drop_effect_format() -> u32 {
        let name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    }

    pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
        let _guard = open()?;
        unsafe {
            let h = GetClipboardData(CF_HDROP);
            if h.is_null() {
                return None;
            }
            let n = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
            let mut files = Vec::with_capacity(n as usize);
            for i in 0..n {
                let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0);
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; len as usize + 1];
                let got = DragQueryFileW(h, i, buf.as_mut_ptr(), len + 1);
                buf.truncate(got as usize);
                files.push(PathBuf::from(OsString::from_wide(&buf)));
            }
            let mut cut = false;
            let fmt = drop_effect_format();
            if fmt != 0 {
                let e = GetClipboardData(fmt);
                if !e.is_null() {
                    let p = GlobalLock(e) as *const u8;
                    if !p.is_null() {
                        let mut b = [0u8; 4];
                        std::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), 4);
                        GlobalUnlock(e);
                        let effect = u32::from_le_bytes(b);
                        cut = effect & DROPEFFECT_MOVE != 0 && effect & DROPEFFECT_COPY == 0;
                    }
                }
            }
            (!files.is_empty()).then_some((files, cut))
        }
    }

    pub fn clear() {
        if let Some(_guard) = open() {
            unsafe {
                EmptyClipboard();
            }
        }
    }

    pub fn set_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
        // DROPFILES header followed by NUL-separated UTF-16 paths and a final NUL.
        let mut buf: Vec<u8> = Vec::with_capacity(DROPFILES_SIZE + paths.len() * 128);
        buf.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
        buf.extend_from_slice(&0i32.to_le_bytes()); // pt.x
        buf.extend_from_slice(&0i32.to_le_bytes()); // pt.y
        buf.extend_from_slice(&0i32.to_le_bytes()); // fNC
        buf.extend_from_slice(&1i32.to_le_bytes()); // fWide
        for p in paths {
            for u in p.as_os_str().encode_wide() {
                buf.extend_from_slice(&u.to_le_bytes());
            }
            buf.extend_from_slice(&[0, 0]);
        }
        buf.extend_from_slice(&[0, 0]);

        let effect = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };

        let _guard = open().ok_or(crate::i18n::tr().clipboard_busy)?;
        unsafe {
            if EmptyClipboard() == 0 {
                return Err(crate::i18n::tr().clipboard_clear_failed.into());
            }
            if !set_data(CF_HDROP, &buf) {
                return Err(crate::i18n::tr().clipboard_write_failed.into());
            }
            let fmt = drop_effect_format();
            if fmt != 0 {
                set_data(fmt, &effect.to_le_bytes());
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    pub fn set_files(_paths: &[PathBuf], _cut: bool) -> Result<(), String> {
        Err(crate::i18n::tr().clipboard_unsupported.into())
    }

    pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
        None
    }

    pub fn clear() {}
}
