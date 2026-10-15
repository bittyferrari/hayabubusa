//! Copy / move / delete through the Windows shell (SHFileOperation), so it behaves like
//! Explorer: progress dialog, "- 複製" names on collision, delete goes to the Recycle Bin.

use std::path::{Path, PathBuf};

/// Copy (or move) `sources` into `dest_dir`. Name collisions get a new name, nothing is
/// overwritten. Returns `Ok(false)` if the user cancelled.
pub fn copy_or_move(sources: &[PathBuf], dest_dir: &Path, mv: bool) -> Result<bool, String> {
    if sources.is_empty() {
        return Ok(true);
    }
    imp::copy_or_move(sources, dest_dir, mv)
}

/// Send `paths` to the Recycle Bin. Returns `Ok(false)` if the user cancelled.
pub fn recycle(paths: &[PathBuf]) -> Result<bool, String> {
    if paths.is_empty() {
        return Ok(true);
    }
    imp::recycle(paths)
}

#[cfg(windows)]
mod imp {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    #[cfg_attr(target_pointer_width = "32", repr(C, packed(1)))]
    #[cfg_attr(not(target_pointer_width = "32"), repr(C))]
    struct ShFileOpStructW {
        hwnd: *mut c_void,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        any_aborted: i32,
        name_mappings: *mut c_void,
        progress_title: *const u16,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
    }

    const FO_MOVE: u32 = 1;
    const FO_COPY: u32 = 2;
    const FO_DELETE: u32 = 3;
    const FOF_RENAMEONCOLLISION: u16 = 0x0008;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
    const FOF_WANTNUKEWARNING: u16 = 0x4000;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;

    /// NUL-separated list with a final extra NUL, as SHFileOperation wants.
    fn multi_sz<'a>(paths: impl IntoIterator<Item = &'a OsStr>) -> Vec<u16> {
        let mut v = Vec::new();
        for p in paths {
            v.extend(p.encode_wide());
            v.push(0);
        }
        v.push(0);
        v
    }

    fn run(func: u32, from: &[PathBuf], to: Option<&Path>, flags: u16) -> Result<bool, String> {
        let from = multi_sz(from.iter().map(|p| p.as_os_str()));
        let to = to.map(|t| multi_sz([t.as_os_str()]));
        let mut op = ShFileOpStructW {
            hwnd: std::ptr::null_mut(),
            func,
            from: from.as_ptr(),
            to: to.as_ref().map_or(std::ptr::null(), |t| t.as_ptr()),
            flags,
            any_aborted: 0,
            name_mappings: std::ptr::null_mut(),
            progress_title: std::ptr::null(),
        };
        let r = unsafe {
            // The shell uses COM internally; this is a worker thread of our own.
            CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
            SHFileOperationW(&mut op)
        };
        if op.any_aborted != 0 || r == 0x4C7 {
            // 0x4C7 = ERROR_CANCELLED
            return Ok(false);
        }
        if r != 0 {
            return Err(crate::i18n::fill(
                crate::i18n::tr().fileop_failed,
                &[&format!("0x{r:X}")],
            ));
        }
        Ok(true)
    }

    pub fn copy_or_move(sources: &[PathBuf], dest_dir: &Path, mv: bool) -> Result<bool, String> {
        run(
            if mv { FO_MOVE } else { FO_COPY },
            sources,
            Some(dest_dir),
            FOF_ALLOWUNDO | FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR,
        )
    }

    pub fn recycle(paths: &[PathBuf]) -> Result<bool, String> {
        // We ask for confirmation ourselves; still warn if something can't be recycled
        // (e.g. network drive) and would be deleted permanently.
        run(
            FO_DELETE,
            paths,
            None,
            FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING,
        )
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn copy_or_move(_s: &[PathBuf], _d: &Path, _mv: bool) -> Result<bool, String> {
        Err(crate::i18n::tr().paste_unsupported.into())
    }

    pub fn recycle(_p: &[PathBuf]) -> Result<bool, String> {
        Err(crate::i18n::tr().delete_unsupported.into())
    }
}
