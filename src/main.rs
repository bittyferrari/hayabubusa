#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod clipboard;
mod decode;
mod fileops;
mod fonts;
mod i18n;
mod ops;
mod resident;
mod util;
mod workers;

use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

fn main() -> eframe::Result {
    let arg = std::env::args_os().nth(1).map(PathBuf::from);

    // An instance is already resident in the tray: hand the path over and quit
    // before paying for any window / GPU start-up.
    if resident::forward(arg.as_deref()) {
        return Ok(());
    }

    // Resolve relative paths while the launch cwd is still in effect, then leave it:
    // Explorer starts us with cwd = the image's folder, and a (resident) process whose
    // cwd is a folder keeps that folder from being deleted / renamed.
    let arg = arg.map(|p| std::path::absolute(&p).unwrap_or(p));
    release_cwd();

    let (start_dir, start_file) = match arg {
        Some(p) if p.is_file() => {
            let p = util::canon(p);
            let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            (dir, Some(p))
        }
        Some(p) if p.is_dir() => (util::canon(p), None),
        _ => (
            util::load_last_dir()
                .or_else(dirs::picture_dir)
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from(".")),
            None,
        ),
    };

    // Create the egui context up-front so we can start decoding the image
    // in parallel with window / GPU initialisation.
    let ctx = egui::Context::default();
    let max_tex = Arc::new(AtomicU32::new(8192));
    let full_pool = workers::full_pool(ctx.clone(), max_tex.clone());
    if let Some(f) = &start_file {
        full_pool.set_wanted([f.clone()]);
    }

    let title = start_file
        .as_ref()
        .and_then(|f| f.file_name())
        .map(|n| format!("{} - Hayabubusa", n.to_string_lossy()))
        .unwrap_or_else(|| "Hayabubusa".into());

    let settings = util::Settings::load();
    i18n::set(i18n::resolve(settings.lang));
    let mut vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(settings.win_size.unwrap_or([1280.0, 860.0]))
        .with_min_inner_size([400.0, 300.0])
        .with_drag_and_drop(true)
        .with_icon(app_icon());
    // Default (first run) and "always maximized" mode start maximized;
    // otherwise restore the last saved state.
    let maximize = match settings.window_mode {
        util::WindowMode::Maximized => true,
        util::WindowMode::Remember => settings.win_maximized.unwrap_or(true),
    };
    let mut centered = true;
    if maximize {
        vb = vb.with_maximized(true);
    } else if let Some(pos) = settings.win_pos {
        vb = vb.with_position(pos);
        centered = false;
    }

    let options = eframe::NativeOptions {
        viewport: vb,
        centered,
        ..Default::default()
    };

    eframe::run_native_ext(
        "hayabubusa",
        options,
        Some(ctx),
        Box::new(move |cc| {
            let resident_rx = resident::init(
                cc.egui_ctx.clone(),
                full_pool.requester(),
                main_hwnd(cc),
            );
            Ok(Box::new(app::App::new(
                cc.egui_ctx.clone(),
                full_pool,
                max_tex,
                start_dir,
                start_file,
                settings,
                resident_rx,
            )))
        }),
    )
}

/// Move the working directory to the executable's folder (which is in use anyway),
/// so no image folder stays locked while we run / stay resident.
fn release_cwd() {
    let target = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::var_os("SystemRoot").map(PathBuf::from));
    if let Some(d) = target {
        let _ = std::env::set_current_dir(d);
    }
}

/// Raw Win32 handle of the main window (0 elsewhere / if unavailable).
fn main_hwnd(cc: &eframe::CreationContext<'_>) -> isize {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                return w.hwnd.get();
            }
        }
    }
    let _ = cc;
    0
}

/// Window / taskbar icon, embedded at compile time.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("embedded icon")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}
