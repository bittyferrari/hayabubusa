mod comic;

use crate::i18n::{self, fill, tr};
use crate::ops::{self, Batch, ConvertOptions, OutFormat, ResizeMode};
use crate::resident;
use crate::util;
use crate::workers::{self, ComicResult, DimsResult, FullResult, Pool, ThumbResult};
use crossbeam_channel::Receiver;
use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, Key, Modifiers, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Browser,
    Viewer,
}

enum Full {
    Ready(TextureHandle, [u32; 2]),
    Failed(String),
}

enum Thumb {
    Ready(TextureHandle, [usize; 2]),
    /// Decoding failed at time `at` (egui seconds) after `tries` attempts. A new file may
    /// still be locked (being copied / scanned by antivirus), so it is retried a few times.
    Failed { at: f64, tries: u8 },
}

const THUMB_RETRIES: u8 = 3;
const THUMB_RETRY_SECS: f64 = 1.5;

struct ConvertDialog {
    files: Vec<PathBuf>,
    rotate: u8,
    format: OutFormat,
    quality: u8,
    resize_kind: usize, // 0 none, 1 percent, 2 long side, 3 fit, 4 exact
    percent: f32,
    long_side: u32,
    w: u32,
    h: u32,
    no_upscale: bool,
    subfolder: bool,
    subfolder_name: String,
    overwrite: bool,
}

enum Action {
    Open(usize),
    Click { i: usize, ctrl: bool, shift: bool },
    Parent,
    Goto(PathBuf),
    Rotate(u8),
    Convert,
    /// Put the selection on the system clipboard (true = cut).
    Clipboard(bool),
    Paste,
    Delete,
    /// Open comic mode at the cursor (or the first image).
    Comic,
}

/// Result of a background copy / move / delete.
struct FsDone {
    result: Result<String, String>,
    /// Paths that no longer exist here (deleted / moved away).
    removed: Vec<PathBuf>,
    /// Entry to put the cursor on after the refresh.
    select: Option<PathBuf>,
    /// A cut was pasted: clear the clipboard and the faded items.
    was_cut: bool,
}

const TEX_OPTS: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

pub struct App {
    ctx: egui::Context,
    mode: Mode,
    dir: PathBuf,
    path_edit: String,
    entries: Vec<Entry>,
    images: Vec<usize>,
    scan_rx: Option<Receiver<(PathBuf, Vec<Entry>)>>,
    pending_select: Option<PathBuf>,

    // browser
    selected: HashSet<usize>,
    cursor: Option<usize>,
    anchor: Option<usize>,
    thumb_disp: f32,
    thumbs: HashMap<PathBuf, Thumb>,
    thumb_pool: Pool<ThumbResult>,
    scroll_to_cursor: bool,
    cols: usize,
    scroll_offset: f32,
    view_h: f32,

    // viewer
    view_path: Option<PathBuf>,
    view_idx: Option<usize>,
    full: HashMap<PathBuf, Full>,
    full_pool: Pool<FullResult>,
    max_tex: Arc<AtomicU32>,
    zoom: Option<f32>, // None = fit
    pan: Vec2,
    rot: u8,
    wheel_acc: f32,
    last_wheel_frame: u64,

    // comic mode: a viewer sub-mode showing all images stacked top to bottom
    comic: bool,
    /// Image at the top edge of the view and how far into it (0..1 of its height).
    /// Kept by path so it survives re-layout (zoom, sizes arriving) and rescans.
    comic_top: Option<(PathBuf, f32)>,
    comic_pan_x: f32,
    /// Wheel / key scrolling still to animate (points, + = down).
    comic_pending: f32,
    /// Image sizes from file headers, for laying out comic mode ([0, 0] = unreadable).
    dims: HashMap<PathBuf, [u32; 2]>,
    dims_pool: Pool<DimsResult>,
    /// Comic mode textures, decoded at the width they are shown at.
    comic_full: HashMap<PathBuf, comic::ComicFull>,
    comic_pool: Pool<ComicResult>,
    /// Pixel width the comic pool scales new images to.
    comic_target_w: Arc<AtomicU32>,

    convert: Option<ConvertDialog>,
    batch: Option<(Batch, bool)>, // bool = finish handled
    toast: Option<(String, f64)>,
    title: String,
    fonts_rx: Option<Receiver<egui::FontDefinitions>>,

    settings: util::Settings,
    settings_open: bool,
    /// Time (egui seconds) when settings changed and still need writing to disk.
    settings_dirty_at: Option<f64>,
    /// Smooth wheel-scroll target offset for the thumbnail grid.
    scroll_target: Option<f32>,
    /// Files last cut to the clipboard (drawn faded until pasted elsewhere).
    cut_paths: HashSet<PathBuf>,
    was_focused: bool,
    /// Running copy / move / delete.
    fs_rx: Option<Receiver<FsDone>>,
    /// Waiting for the user to confirm sending these to the Recycle Bin.
    delete_confirm: Option<Vec<PathBuf>>,
    /// Ctrl+V already handled through egui's Paste event; ignore the following V key-up.
    paste_seen: bool,
    /// Signature of the listing last loaded (to notice changes made by other programs).
    dir_sig: u64,
    watch_rx: Option<Receiver<(PathBuf, u64)>>,
    next_watch: f64,
    /// Messages from the tray / other instances (Windows resident mode).
    resident_rx: Receiver<resident::Msg>,
    /// Window close was turned into "hide to tray"; done on the next frame.
    hide_pending: bool,
    /// Really exit on the next close request (tray menu "結束").
    quitting: bool,
}

impl App {
    pub fn new(
        ctx: egui::Context,
        full_pool: Pool<FullResult>,
        max_tex: Arc<AtomicU32>,
        start_dir: PathBuf,
        start_file: Option<PathBuf>,
        settings: util::Settings,
        resident_rx: Receiver<resident::Msg>,
    ) -> Self {
        if settings.stay_resident {
            // Tray icon + single-instance window; created on its own thread.
            resident::start(false);
        }
        // Load a CJK-capable system font in the background (keeps startup fast).
        let frx = load_fonts_async(&ctx, i18n::current());
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_gray(32);
        ctx.set_visuals(visuals);

        let comic_target_w = Arc::new(AtomicU32::new(1024));
        let mut app = Self {
            thumb_pool: workers::thumb_pool(ctx.clone()),
            dims_pool: workers::dims_pool(ctx.clone()),
            comic_pool: workers::comic_pool(ctx.clone(), comic_target_w.clone(), max_tex.clone()),
            comic_target_w,
            comic_full: HashMap::new(),
            ctx,
            mode: if start_file.is_some() {
                Mode::Viewer
            } else {
                Mode::Browser
            },
            dir: PathBuf::new(),
            path_edit: String::new(),
            entries: Vec::new(),
            images: Vec::new(),
            scan_rx: None,
            pending_select: start_file.clone(),
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
            thumb_disp: settings.thumb_size,
            thumbs: HashMap::new(),
            scroll_to_cursor: true,
            cols: 1,
            scroll_offset: 0.0,
            view_h: 600.0,
            view_path: start_file,
            view_idx: None,
            full: HashMap::new(),
            full_pool,
            max_tex,
            zoom: None,
            pan: Vec2::ZERO,
            rot: 0,
            wheel_acc: 0.0,
            last_wheel_frame: u64::MAX,
            comic: false,
            comic_top: None,
            comic_pan_x: 0.0,
            comic_pending: 0.0,
            dims: HashMap::new(),
            convert: None,
            batch: None,
            toast: None,
            title: String::new(),
            fonts_rx: Some(frx),
            settings,
            settings_open: false,
            settings_dirty_at: None,
            scroll_target: None,
            cut_paths: HashSet::new(),
            was_focused: true,
            fs_rx: None,
            delete_confirm: None,
            paste_seen: false,
            dir_sig: 0,
            watch_rx: None,
            next_watch: 0.0,
            resident_rx,
            hide_pending: false,
            quitting: false,
        };
        app.set_dir(start_dir, app.pending_select.clone());
        app
    }

    // ------------------------------------------------------------------ directory

    fn set_dir(&mut self, dir: PathBuf, select: Option<PathBuf>) {
        let dir = util::canon(dir);
        if dir != self.dir {
            self.thumbs.clear();
            self.scroll_offset = 0.0;
            self.scroll_target = None;
            // A refresh of the same folder keeps the selection (restored by path in poll_scan).
            self.selected.clear();
            self.cursor = None;
            self.anchor = None;
        }
        self.path_edit = dir.to_string_lossy().to_string();
        self.dir = dir.clone();
        self.pending_select = select;
        self.watch_rx = None;
        util::save_last_dir(&dir);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let entries = scan_dir(&dir);
            let _ = tx.send((dir, entries));
            ctx.request_repaint();
        });
        self.scan_rx = Some(rx);
    }

    /// Reload the listing; cursor and selection stay on the same files.
    fn rescan(&mut self) {
        let sel = match self.mode {
            Mode::Viewer => self.view_path.clone(),
            Mode::Browser => None,
        };
        let dir = self.dir.clone();
        self.set_dir(dir, sel);
    }

    /// F5: reload the listing and all thumbnails / the current image.
    fn refresh_all(&mut self) {
        self.thumbs.clear();
        if let Some(v) = &self.view_path {
            self.full.remove(v);
        }
        self.rescan();
        self.toast(tr().refreshed);
    }

    /// Every second or so, re-read the folder in the background and reload the
    /// listing if files were added / removed / changed by another program.
    fn poll_watch(&mut self) {
        let now = self.ctx.input(|i| i.time);
        if let Some(rx) = &self.watch_rx {
            if let Ok((dir, sig)) = rx.try_recv() {
                self.watch_rx = None;
                if dir == self.dir && self.scan_rx.is_none() && sig != self.dir_sig {
                    self.rescan();
                }
            }
            return;
        }
        let focused = self.ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let interval = if focused { 1.0 } else { 2.5 };
        if now >= self.next_watch && self.scan_rx.is_none() && self.fs_rx.is_none() {
            self.next_watch = now + interval;
            let dir = self.dir.clone();
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let sig = dir_signature(&scan_dir(&dir));
                let _ = tx.send((dir, sig));
                ctx.request_repaint();
            });
            self.watch_rx = Some(rx);
        }
        self.ctx
            .request_repaint_after(std::time::Duration::from_secs_f64(interval));
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        let Ok((dir, entries)) = rx.try_recv() else {
            return;
        };
        self.scan_rx = None;
        if dir != self.dir {
            return;
        }
        self.dir_sig = dir_signature(&entries);

        // Remember the selection by path (indices change when files come and go).
        let path_of = |i: usize| self.entries.get(i).map(|e| e.path.clone());
        let old_cursor = self.cursor.and_then(path_of);
        let old_anchor = self.anchor.and_then(path_of);
        let old_sel: HashSet<PathBuf> =
            self.selected.iter().filter_map(|&i| path_of(i)).collect();

        // Files that changed on disk (or failed before, e.g. were still being copied)
        // get a fresh thumbnail / full image.
        let old: HashMap<&Path, (u64, Option<SystemTime>)> = self
            .entries
            .iter()
            .map(|e| (e.path.as_path(), (e.size, e.modified)))
            .collect();
        let mut stale: Vec<PathBuf> = entries
            .iter()
            .filter(|e| {
                old.get(e.path.as_path())
                    .is_some_and(|&(size, m)| size != e.size || m != e.modified)
            })
            .map(|e| e.path.clone())
            .collect();
        stale.extend(
            self.thumbs
                .iter()
                .filter(|(_, t)| matches!(t, Thumb::Failed { .. }))
                .map(|(p, _)| p.clone()),
        );
        self.invalidate(&stale);

        self.entries = entries;
        sort_entries(&mut self.entries, self.settings.sort_key, self.settings.sort_desc);
        self.images = (0..self.entries.len())
            .filter(|&i| !self.entries[i].is_dir)
            .collect();
        let idx: HashMap<&Path, usize> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.as_path(), i))
            .collect();
        self.selected = old_sel
            .iter()
            .filter_map(|p| idx.get(p.as_path()).copied())
            .collect();
        self.anchor = old_anchor.and_then(|p| idx.get(p.as_path()).copied());
        self.cursor = old_cursor.and_then(|p| idx.get(p.as_path()).copied());
        let mut jump = self.cursor.is_none();
        if let Some(sel) = self.pending_select.take() {
            let sel = util::canon(sel);
            if let Some(&i) = idx.get(sel.as_path()) {
                self.cursor = Some(i);
                self.selected.clear();
                self.selected.insert(i);
                self.anchor = Some(i);
                jump = true;
            }
        }
        if self.cursor.is_none() && !self.entries.is_empty() {
            self.cursor = Some(0);
            self.anchor = Some(0);
            self.selected.insert(0);
        }
        if self.selected.is_empty() {
            self.selected.extend(self.cursor);
        }
        // Background refreshes don't yank the scroll position.
        if jump {
            self.scroll_to_cursor = true;
        }
        self.update_view_idx();
    }

    /// Re-sort the current listing in place, keeping cursor / selection on the same files.
    fn apply_sort(&mut self) {
        self.mark_settings_dirty();
        if self.entries.is_empty() {
            return;
        }
        let path_of = |i: usize| self.entries.get(i).map(|e| e.path.clone());
        let cur = self.cursor.and_then(path_of);
        let anc = self.anchor.and_then(path_of);
        let sel: HashSet<PathBuf> = self.selected.iter().filter_map(|&i| path_of(i)).collect();

        sort_entries(&mut self.entries, self.settings.sort_key, self.settings.sort_desc);
        self.images = (0..self.entries.len())
            .filter(|&i| !self.entries[i].is_dir)
            .collect();

        let idx: HashMap<&Path, usize> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.as_path(), i))
            .collect();
        self.cursor = cur.and_then(|p| idx.get(p.as_path()).copied());
        self.anchor = anc.and_then(|p| idx.get(p.as_path()).copied());
        self.selected = sel
            .iter()
            .filter_map(|p| idx.get(p.as_path()).copied())
            .collect();
        self.scroll_to_cursor = true;
        self.update_view_idx();
    }

    fn update_view_idx(&mut self) {
        self.view_idx = self.view_path.as_ref().and_then(|vp| {
            let vp = util::canon(vp.clone());
            self.images.iter().position(|&e| self.entries[e].path == vp)
        });
        if let Some(i) = self.view_idx {
            self.view_path = Some(self.entries[self.images[i]].path.clone());
        }
    }

    fn go_parent(&mut self) {
        if let Some(p) = self.dir.parent() {
            let from = self.dir.clone();
            self.set_dir(p.to_path_buf(), Some(from));
        }
    }

    // ------------------------------------------------------------------ helpers

    fn toast(&mut self, s: impl Into<String>) {
        let t = self.ctx.input(|i| i.time);
        self.toast = Some((s.into(), t + 2.5));
    }

    fn selected_images(&self) -> Vec<PathBuf> {
        let mut v: Vec<usize> = self
            .selected
            .iter()
            .copied()
            .filter(|&i| !self.entries[i].is_dir)
            .collect();
        if v.is_empty() {
            if let Some(c) = self.cursor {
                if !self.entries[c].is_dir {
                    v.push(c);
                }
            }
        }
        v.sort_unstable();
        v.into_iter()
            .map(|i| self.entries[i].path.clone())
            .collect()
    }

    /// Selected entries (files and folders), or the cursor entry if nothing is selected.
    fn selected_paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<usize> = self.selected.iter().copied().collect();
        if v.is_empty() {
            v.extend(self.cursor);
        }
        v.sort_unstable();
        v.into_iter()
            .filter_map(|i| self.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    fn copy_to_clipboard(&mut self, paths: Vec<PathBuf>, cut: bool) {
        match crate::clipboard::set_files(&paths, cut) {
            Ok(()) => {
                let n = paths.len();
                self.cut_paths = if cut {
                    paths.into_iter().collect()
                } else {
                    HashSet::new()
                };
                self.toast(fill(if cut { tr().cut_n } else { tr().copied_n }, &[&n]));
            }
            Err(e) => self.toast(e),
        }
    }

    /// After files were cut and pasted elsewhere they no longer exist here: refresh
    /// the listing when the window gets focus back.
    fn check_cut_on_focus(&mut self) {
        let focused = self.ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.was_focused && !self.cut_paths.is_empty() {
            let before = self.cut_paths.len();
            self.cut_paths.retain(|p| p.exists());
            if self.cut_paths.len() != before {
                self.rescan();
            }
        }
        self.was_focused = focused;
    }

    fn fs_busy(&mut self) -> bool {
        if self.fs_rx.is_some() {
            self.toast(tr().fs_busy);
            return true;
        }
        false
    }

    fn spawn_fs(&mut self, job: impl FnOnce() -> FsDone + Send + 'static) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.fs_rx = Some(rx);
    }

    /// Paste files from the system clipboard into the current folder.
    fn paste(&mut self) {
        if self.fs_busy() {
            return;
        }
        let Some((files, cut)) = crate::clipboard::get_files() else {
            self.toast(tr().clipboard_no_files);
            return;
        };
        let dest = self.dir.clone();
        // Moving into the folder the files are already in does nothing.
        let files: Vec<PathBuf> = if cut {
            files
                .into_iter()
                .filter(|f| f.parent() != Some(dest.as_path()))
                .collect()
        } else {
            files
        };
        if files.is_empty() {
            self.toast(tr().already_here);
            return;
        }
        let n = files.len();
        let select = files
            .first()
            .and_then(|f| f.file_name())
            .map(|name| dest.join(name));
        self.spawn_fs(move || {
            let r = crate::fileops::copy_or_move(&files, &dest, cut);
            FsDone {
                result: match r {
                    Ok(true) => Ok(fill(if cut { tr().moved_n } else { tr().pasted_n }, &[&n])),
                    Ok(false) => Ok(tr().cancelled.into()),
                    Err(e) => Err(e),
                },
                removed: if cut { files } else { Vec::new() },
                select,
                was_cut: cut,
            }
        });
    }

    /// Ask (if enabled) and then send `paths` to the Recycle Bin.
    fn request_delete(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() || self.fs_busy() {
            return;
        }
        if self.settings.confirm_delete {
            self.delete_confirm = Some(paths);
        } else {
            self.do_delete(paths);
        }
    }

    /// Nearest entry (after the cursor, else before) that is not being removed.
    fn neighbor(&self, removed: &HashSet<&Path>, images_only: bool) -> Option<PathBuf> {
        let cur = match self.mode {
            Mode::Viewer => self
                .view_path
                .as_ref()
                .and_then(|v| self.entries.iter().position(|e| &e.path == v)),
            Mode::Browser => self.cursor,
        }?;
        let ok = |i: &usize| {
            let e = &self.entries[*i];
            !removed.contains(e.path.as_path()) && !(images_only && e.is_dir)
        };
        (cur..self.entries.len())
            .find(ok)
            .or_else(|| (0..cur).rev().find(ok))
            .map(|i| self.entries[i].path.clone())
    }

    fn do_delete(&mut self, paths: Vec<PathBuf>) {
        if self.fs_busy() {
            return;
        }
        let select = {
            let set: HashSet<&Path> = paths.iter().map(|p| p.as_path()).collect();
            self.neighbor(&set, self.mode == Mode::Viewer)
        };
        let n = paths.len();
        self.spawn_fs(move || {
            let r = crate::fileops::recycle(&paths);
            let ok = matches!(r, Ok(true));
            FsDone {
                result: match r {
                    Ok(true) => Ok(fill(tr().recycled_n, &[&n])),
                    Ok(false) => Ok(tr().cancelled.into()),
                    Err(e) => Err(e),
                },
                // Some files may still exist after a cancel / error; filtered on arrival.
                removed: if ok { paths } else { paths.into_iter().filter(|p| !p.exists()).collect() },
                select,
                was_cut: false,
            }
        });
    }

    fn poll_fs(&mut self) {
        let Some(rx) = &self.fs_rx else { return };
        let Ok(done) = rx.try_recv() else { return };
        self.fs_rx = None;
        match done.result {
            Ok(msg) => self.toast(msg),
            Err(e) => self.toast(e),
        }
        let removed: Vec<PathBuf> = done.removed.into_iter().filter(|p| !p.exists()).collect();
        if done.was_cut && !removed.is_empty() {
            crate::clipboard::clear();
            self.cut_paths.clear();
        }
        self.invalidate(&removed);
        // The image being viewed was deleted: show the neighbour instead.
        if self.mode == Mode::Viewer {
            if let Some(v) = self.view_path.clone() {
                if removed.contains(&v) {
                    match done.select.clone() {
                        Some(next) => {
                            self.view_path = Some(next);
                            self.reset_view();
                            self.update_view_idx();
                        }
                        None => {
                            self.view_path = None;
                            self.mode = Mode::Browser;
                        }
                    }
                }
            }
        }
        let keep_cursor = self
            .cursor
            .and_then(|c| self.entries.get(c))
            .map(|e| e.path.clone())
            .filter(|p| !removed.contains(p));
        // paste: the (first) pasted item; delete: the neighbour of the deleted ones
        let select = done.select.filter(|s| s.exists()).or(keep_cursor);
        let select = if self.mode == Mode::Viewer {
            self.view_path.clone()
        } else {
            select
        };
        let dir = self.dir.clone();
        self.set_dir(dir, select);
    }

    fn delete_confirm_window(&mut self) {
        let Some(paths) = &self.delete_confirm else { return };
        let n = paths.len();
        let names: Vec<String> = paths
            .iter()
            .take(5)
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
            .collect();
        let (enter, esc) = self
            .ctx
            .input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        let mut confirm = enter;
        let mut cancel = esc;
        let mut dont_ask = false;
        egui::Window::new(tr().delete_title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&self.ctx, |ui| {
                ui.label(fill(tr().delete_confirm_n, &[&n]));
                ui.add_space(4.0);
                for name in &names {
                    ui.label(egui::RichText::new(format!("  {name}")).weak());
                }
                if n > names.len() {
                    ui.label(
                        egui::RichText::new(format!(
                            "  {}",
                            fill(tr().and_more_n, &[&(n - names.len())])
                        ))
                        .weak(),
                    );
                }
                ui.add_space(8.0);
                ui.checkbox(&mut dont_ask, tr().dont_ask_again);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button(tr().delete_enter).clicked() {
                        confirm = true;
                    }
                    if ui.button(tr().cancel_esc).clicked() {
                        cancel = true;
                    }
                });
            });
        if dont_ask && self.settings.confirm_delete {
            self.settings.confirm_delete = false;
            self.mark_settings_dirty();
        }
        if cancel {
            self.delete_confirm = None;
        } else if confirm {
            if let Some(p) = self.delete_confirm.take() {
                self.do_delete(p);
            }
        }
    }

    /// Ctrl+V: egui only sends a Paste event when the clipboard holds text, and swallows
    /// the key-down otherwise, so also accept the V key-up while Ctrl is held.
    fn paste_pressed(&mut self, i: &egui::InputState) -> bool {
        let mut hit = false;
        for ev in &i.events {
            match ev {
                egui::Event::Paste(_) => {
                    self.paste_seen = true;
                    hit = true;
                }
                egui::Event::Key {
                    key: Key::V,
                    pressed: false,
                    modifiers,
                    ..
                } if modifiers.command => {
                    if std::mem::take(&mut self.paste_seen) {
                        continue;
                    }
                    hit = true;
                }
                _ => {}
            }
        }
        hit
    }

    fn image_path(&self, idx: usize) -> &Path {
        &self.entries[self.images[idx]].path
    }

    fn open_viewer(&mut self, path: PathBuf) {
        self.view_path = Some(path);
        self.update_view_idx();
        self.reset_view();
        self.mode = Mode::Viewer;
    }

    fn back_to_browser(&mut self) {
        if self.rot != 0 {
            self.toast(tr().rotation_discarded);
        }
        self.reset_view();
        if let Some(vp) = &self.view_path {
            if let Some(i) = self.entries.iter().position(|e| &e.path == vp) {
                self.cursor = Some(i);
                self.anchor = Some(i);
                self.selected.clear();
                self.selected.insert(i);
                self.scroll_to_cursor = true;
            } else if self.scan_rx.is_some() {
                self.pending_select = Some(vp.clone());
            }
        }
        if self.ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        self.mode = Mode::Browser;
    }

    fn reset_view(&mut self) {
        self.zoom = None;
        self.pan = Vec2::ZERO;
        self.rot = 0;
        self.comic_top = None;
        self.comic_pan_x = 0.0;
        self.comic_pending = 0.0;
    }

    fn view_step(&mut self, delta: i64) {
        let n = self.images.len() as i64;
        let Some(cur) = self.view_idx else { return };
        if n == 0 {
            return;
        }
        let next = (cur as i64 + delta).clamp(0, n - 1) as usize;
        if next != cur {
            if self.rot != 0 {
                self.toast(tr().rotation_discarded);
            }
            self.view_idx = Some(next);
            self.view_path = Some(self.image_path(next).to_path_buf());
            self.reset_view();
        }
    }

    fn invalidate(&mut self, paths: &[PathBuf]) {
        for p in paths {
            self.full.remove(p);
            self.thumbs.remove(p);
            self.dims.remove(p);
            self.comic_full.remove(p);
        }
    }

    fn start_rotate(&mut self, files: Vec<PathBuf>, turns: u8) {
        if files.is_empty() || self.batch.is_some() {
            return;
        }
        let b = Batch::spawn(
            fill(tr().rotating_n, &[&files.len()]),
            files.clone(),
            self.ctx.clone(),
            move |p| ops::rotate_file(p, turns).map(|_| Some(p.to_path_buf())),
        );
        self.batch = Some((b, false));
    }

    fn open_convert(&mut self, files: Vec<PathBuf>, rotate: u8) {
        if files.is_empty() {
            return;
        }
        self.convert = Some(ConvertDialog {
            files,
            rotate,
            format: OutFormat::Webp,
            quality: 90,
            resize_kind: 0,
            percent: 50.0,
            long_side: 1920,
            w: 1920,
            h: 1080,
            no_upscale: true,
            subfolder: true,
            subfolder_name: "converted".into(),
            overwrite: false,
        });
    }

    // ------------------------------------------------------------------ polling

    fn poll_workers(&mut self) {
        let max_side = self.ctx.input(|i| i.max_texture_side) as u32;
        self.max_tex.store(max_side.max(2048), Ordering::Relaxed);

        while let Ok(r) = self.full_pool.rx.try_recv() {
            let st = match r.result {
                Ok((img, orig)) => {
                    self.dims.insert(r.path.clone(), orig);
                    let tex = self
                        .ctx
                        .load_texture(r.path.to_string_lossy(), img, TEX_OPTS);
                    Full::Ready(tex, orig)
                }
                Err(e) => Full::Failed(e),
            };
            self.full.insert(r.path, st);
        }
        self.poll_comic();
        while let Ok(r) = self.dims_pool.rx.try_recv() {
            self.dims.entry(r.path).or_insert(r.dims.unwrap_or([0, 0]));
        }
        while let Ok(r) = self.thumb_pool.rx.try_recv() {
            let st = match r.image {
                Some(img) => {
                    let size = img.size;
                    Thumb::Ready(self.ctx.load_texture("thumb", img, TEX_OPTS), size)
                }
                None => {
                    let tries = match self.thumbs.get(&r.path) {
                        Some(Thumb::Failed { tries, .. }) => tries + 1,
                        _ => 1,
                    };
                    if tries < THUMB_RETRIES {
                        self.ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                            THUMB_RETRY_SECS + 0.05,
                        ));
                    }
                    Thumb::Failed {
                        at: self.ctx.input(|i| i.time),
                        tries,
                    }
                }
            };
            self.thumbs.insert(r.path, st);
        }
        if let Some(rx) = &self.fonts_rx {
            if let Ok(defs) = rx.try_recv() {
                self.ctx.set_fonts(defs);
                self.fonts_rx = None;
            }
        }
        // finished batch
        let mut finished_outputs = None;
        if let Some((b, handled)) = &mut self.batch {
            if !*handled && b.finished.load(Ordering::SeqCst) {
                *handled = true;
                finished_outputs = Some(b.outputs.lock().unwrap().clone());
                if b.errors.lock().unwrap().is_empty() {
                    let msg = fill(tr().batch_done, &[&b.title]);
                    self.batch = None;
                    self.toast(msg);
                }
            }
        }
        if let Some(outs) = finished_outputs {
            self.invalidate(&outs);
            if self.view_path.as_ref().is_some_and(|v| outs.contains(v)) {
                self.rot = 0;
            }
            if outs.iter().any(|o| o.parent() == Some(self.dir.as_path())) {
                self.rescan();
            }
        }
    }

    // ------------------------------------------------------------------ browser

    fn browser_keys(&mut self, actions: &mut Vec<Action>) {
        if self.ctx.egui_wants_keyboard_input()
            || self.convert.is_some()
            || self.delete_confirm.is_some()
        {
            return;
        }
        let ctx = self.ctx.clone();
        if ctx.input(|i| self.paste_pressed(i)) {
            actions.push(Action::Paste);
        }
        if ctx.input(|i| i.key_pressed(Key::Delete) && !i.modifiers.shift) {
            actions.push(Action::Delete);
        }
        let n = self.entries.len();
        let cols = self.cols.max(1) as i64;
        let page = cols * ((self.view_h / self.cell_size().y).floor().max(1.0) as i64);
        let mut clip: Option<bool> = None;
        let (mv, enter, back, all, shift) = self.ctx.input_mut(|i| {
            // egui turns Ctrl+C / Ctrl+X into these events (no Key event is sent).
            for ev in &i.events {
                match ev {
                    egui::Event::Copy => clip = Some(false),
                    egui::Event::Cut => clip = Some(true),
                    _ => {}
                }
            }
            let mut mv: i64 = 0;
            if i.key_pressed(Key::ArrowRight) {
                mv += 1;
            }
            if i.key_pressed(Key::ArrowLeft) {
                mv -= 1;
            }
            if i.key_pressed(Key::ArrowDown) {
                mv += cols;
            }
            if i.key_pressed(Key::ArrowUp) {
                mv -= cols;
            }
            if i.key_pressed(Key::PageDown) {
                mv += page;
            }
            if i.key_pressed(Key::PageUp) {
                mv -= page;
            }
            if i.key_pressed(Key::Home) {
                mv = i64::MIN / 2;
            }
            if i.key_pressed(Key::End) {
                mv = i64::MAX / 2;
            }
            (
                mv,
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Backspace),
                i.consume_key(Modifiers::COMMAND, Key::A),
                i.modifiers.shift,
            )
        });
        if n > 0 && mv != 0 {
            let cur = self.cursor.unwrap_or(0) as i64;
            let next = cur.saturating_add(mv).clamp(0, n as i64 - 1) as usize;
            self.cursor = Some(next);
            if shift {
                let a = self.anchor.unwrap_or(next);
                self.selected = (a.min(next)..=a.max(next)).collect();
            } else {
                self.selected.clear();
                self.selected.insert(next);
                self.anchor = Some(next);
            }
            self.scroll_to_cursor = true;
        }
        if enter {
            if let Some(c) = self.cursor {
                actions.push(Action::Open(c));
            }
        }
        if back {
            actions.push(Action::Parent);
        }
        if all {
            self.selected = (0..n).collect();
        }
        if let Some(cut) = clip {
            actions.push(Action::Clipboard(cut));
        }
    }

    fn cell_size(&self) -> Vec2 {
        vec2(self.thumb_disp + 14.0, self.thumb_disp + 14.0 + 20.0)
    }

    fn browser_ui(&mut self, ui: &mut egui::Ui) {
        let mut actions = Vec::new();
        self.browser_keys(&mut actions);

        egui::Panel::top("browser_top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(tr().up).on_hover_text("Backspace").clicked() {
                    actions.push(Action::Parent);
                }
                if ui.button("⟳").on_hover_text(tr().refresh_tip).clicked() {
                    self.refresh_all();
                }
                ui.separator();
                if ui.button(tr().rotate_left).clicked() {
                    actions.push(Action::Rotate(3));
                }
                if ui.button(tr().rotate_right).clicked() {
                    actions.push(Action::Rotate(1));
                }
                if ui.button(tr().convert_resize_menu).clicked() {
                    actions.push(Action::Convert);
                }
                ui.separator();
                if ui
                    .button(tr().comic_mode)
                    .on_hover_text(tr().comic_tip)
                    .clicked()
                {
                    actions.push(Action::Comic);
                }
                ui.separator();
                if ui.button(tr().paste).on_hover_text("Ctrl+V").clicked() {
                    actions.push(Action::Paste);
                }
                if ui
                    .button(tr().delete)
                    .on_hover_text(tr().delete_tip)
                    .clicked()
                {
                    actions.push(Action::Delete);
                }
                ui.separator();
                ui.label(tr().thumbs);
                ui.add(egui::Slider::new(&mut self.thumb_disp, 64.0..=256.0).show_value(false))
                    .on_hover_text(tr().thumbs_tip);
                ui.separator();
                ui.label(tr().sort);
                let mut key = self.settings.sort_key;
                egui::ComboBox::from_id_salt("sort_key")
                    .selected_text(key.label())
                    .show_ui(ui, |ui| {
                        for k in util::SortKey::ALL {
                            ui.selectable_value(&mut key, k, k.label());
                        }
                    });
                let desc = self.settings.sort_desc;
                let dir_btn = ui
                    .button(if desc {
                        tr().descending
                    } else {
                        tr().ascending
                    })
                    .on_hover_text(tr().sort_dir_tip);
                if key != self.settings.sort_key || dir_btn.clicked() {
                    self.settings.sort_key = key;
                    if dir_btn.clicked() {
                        self.settings.sort_desc = !desc;
                    }
                    self.apply_sort();
                }
                ui.separator();
                if ui.button(tr().settings_btn).clicked() {
                    self.settings_open = !self.settings_open;
                }
            });
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.path_edit)
                        .desired_width(ui.available_width()),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let p = PathBuf::from(self.path_edit.trim());
                    if p.is_dir() {
                        actions.push(Action::Goto(p));
                    } else if p.is_file() {
                        actions.push(Action::Goto(p.parent().unwrap_or(&p).to_path_buf()));
                        self.view_path = Some(p);
                    }
                }
            });
            ui.add_space(2.0);
        });

        egui::Panel::bottom("browser_status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let ndirs = self.entries.len() - self.images.len();
                ui.label(fill(tr().status_counts, &[&self.images.len(), &ndirs]));
                if self.selected.len() > 1 {
                    ui.separator();
                    ui.label(fill(tr().selected_n, &[&self.selected.len()]));
                }
                if let Some(e) = self.cursor.and_then(|c| self.entries.get(c)) {
                    ui.separator();
                    ui.label(&e.name);
                    if !e.is_dir {
                        ui.label(util::human_size(e.size));
                    }
                }
                if self.scan_rx.is_some() {
                    ui.separator();
                    ui.spinner();
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            let cell = self.cell_size();
            let avail_w = ui.available_width();
            let cols = ((avail_w / cell.x).floor() as usize).max(1);
            self.cols = cols;
            let n = self.entries.len();
            let rows = n.div_ceil(cols);

            // Mouse wheel is handled here (not by the ScrollArea) so one notch
            // scrolls a configurable number of thumbnail rows.
            let max_off = (rows as f32 * cell.y - self.view_h).max(0.0);
            let mut wheel = 0.0f32; // points, + = down
            let mut zoom_steps = 0.0f32;
            if ui.ui_contains_pointer() {
                let rows_per = self.settings.scroll_rows;
                let view_h = self.view_h;
                ui.input(|i| {
                    for ev in &i.events {
                        if let egui::Event::MouseWheel {
                            unit,
                            delta,
                            modifiers,
                            ..
                        } = ev
                        {
                            if modifiers.command {
                                zoom_steps += match unit {
                                    egui::MouseWheelUnit::Point => delta.y / 50.0,
                                    _ => delta.y,
                                };
                                continue;
                            }
                            // Shift+wheel on some platforms arrives as horizontal.
                            let d = if delta.y != 0.0 { delta.y } else { delta.x };
                            wheel -= match unit {
                                egui::MouseWheelUnit::Line => d * rows_per * cell.y,
                                egui::MouseWheelUnit::Point => d,
                                egui::MouseWheelUnit::Page => d * view_h,
                            };
                        }
                    }
                });
            }
            if zoom_steps != 0.0 {
                self.thumb_disp = (self.thumb_disp + zoom_steps * 16.0).clamp(64.0, 256.0);
            }

            let mut set_off: Option<f32> = None;
            if wheel != 0.0 {
                let base = self.scroll_target.unwrap_or(self.scroll_offset);
                let t = (base + wheel).clamp(0.0, max_off);
                if self.settings.smooth_scroll {
                    self.scroll_target = Some(t);
                } else {
                    self.scroll_target = None;
                    set_off = Some(t);
                }
            }
            if let Some(t) = self.scroll_target {
                let t = t.clamp(0.0, max_off);
                let cur = self.scroll_offset;
                let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
                let k = 1.0 - (-dt * 20.0).exp();
                let mut next = cur + (t - cur) * k;
                if (t - next).abs() < 0.5 {
                    next = t;
                    self.scroll_target = None;
                } else {
                    ui.ctx().request_repaint();
                }
                set_off = Some(next);
            }

            let mut sa = egui::ScrollArea::vertical()
                .auto_shrink(false)
                .scroll_source(egui::scroll_area::ScrollSource {
                    mouse_wheel: false,
                    ..Default::default()
                });
            if self.scroll_to_cursor {
                self.scroll_to_cursor = false;
                if let Some(c) = self.cursor {
                    let top = (c / cols) as f32 * cell.y;
                    let bottom = top + cell.y;
                    let mut off = self.scroll_offset;
                    if top < off {
                        off = top;
                    } else if bottom > off + self.view_h {
                        off = bottom - self.view_h;
                    }
                    if off != self.scroll_offset {
                        self.scroll_target = None;
                        set_off = Some(off.max(0.0));
                    }
                }
            }
            if let Some(off) = set_off {
                sa = sa.vertical_scroll_offset(off);
            }

            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let mut wanted: Vec<PathBuf> = Vec::new();
            let mut ahead: Vec<PathBuf> = Vec::new();
            let out = sa.show_rows(ui, cell.y, rows, |ui, range| {
                // prefetch two rows below the visible area
                for i in (range.end * cols)..((range.end + 2) * cols).min(n) {
                    let e = &self.entries[i];
                    if !e.is_dir && !self.thumbs.contains_key(&e.path) {
                        ahead.push(e.path.clone());
                    }
                }
                for row in range {
                    ui.horizontal(|ui| {
                        for col in 0..cols {
                            let i = row * cols + col;
                            if i >= n {
                                break;
                            }
                            self.draw_cell(ui, i, cell, &mut wanted, &mut actions);
                        }
                    });
                }
            });
            self.scroll_offset = out.state.offset.y;
            // scrollbar dragged / clamped while animating -> stop animation
            if let (Some(_), Some(off)) = (self.scroll_target, set_off) {
                if (out.state.offset.y - off).abs() > 1.0 {
                    self.scroll_target = None;
                }
            }
            self.view_h = out.inner_rect.height();
            wanted.extend(ahead);
            self.thumb_pool.set_wanted(wanted);

            // keep GPU memory bounded
            if self.thumbs.len() > 3000 {
                let first = (self.scroll_offset / cell.y) as usize * cols;
                let lo = first.saturating_sub(1000);
                let hi = (first + 1000).min(n);
                let keep: HashSet<&PathBuf> =
                    self.entries[lo..hi].iter().map(|e| &e.path).collect();
                self.thumbs.retain(|p, _| keep.contains(p));
            }
        });

        // Drag & drop / actions
        for a in actions {
            match a {
                Action::Open(i) => {
                    let e = &self.entries[i];
                    if e.is_dir {
                        let p = e.path.clone();
                        self.set_dir(p, None);
                    } else {
                        let p = e.path.clone();
                        self.open_viewer(p);
                    }
                }
                Action::Click { i, ctrl, shift } => {
                    if shift {
                        let a = self.anchor.unwrap_or(i);
                        self.selected = (a.min(i)..=a.max(i)).collect();
                    } else if ctrl {
                        if !self.selected.insert(i) {
                            self.selected.remove(&i);
                        }
                        self.anchor = Some(i);
                    } else {
                        self.selected.clear();
                        self.selected.insert(i);
                        self.anchor = Some(i);
                    }
                    self.cursor = Some(i);
                }
                Action::Parent => self.go_parent(),
                Action::Goto(p) => {
                    let sel = self
                        .view_path
                        .clone()
                        .filter(|v| v.parent() == Some(p.as_path()));
                    self.set_dir(p, sel);
                }
                Action::Rotate(t) => {
                    let files = self.selected_images();
                    self.start_rotate(files, t);
                }
                Action::Convert => {
                    let files = self.selected_images();
                    self.open_convert(files, 0);
                }
                Action::Clipboard(cut) => {
                    let paths = self.selected_paths();
                    self.copy_to_clipboard(paths, cut);
                }
                Action::Paste => self.paste(),
                Action::Delete => {
                    let paths = self.selected_paths();
                    self.request_delete(paths);
                }
                Action::Comic => {
                    let p = self
                        .cursor
                        .and_then(|c| self.entries.get(c))
                        .filter(|e| !e.is_dir)
                        .or_else(|| self.images.first().map(|&i| &self.entries[i]))
                        .map(|e| e.path.clone());
                    match p {
                        Some(p) => {
                            self.comic = true;
                            self.open_viewer(p);
                        }
                        None => self.toast(tr().no_images),
                    }
                }
            }
        }
    }

    fn draw_cell(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        cell: Vec2,
        wanted: &mut Vec<PathBuf>,
        actions: &mut Vec<Action>,
    ) {
        let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
        let e = &self.entries[i];
        if resp.double_clicked() {
            actions.push(Action::Open(i));
        } else if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            actions.push(Action::Click {
                i,
                ctrl: m.command,
                shift: m.shift,
            });
        }
        if resp.secondary_clicked() && !self.selected.contains(&i) {
            actions.push(Action::Click {
                i,
                ctrl: false,
                shift: false,
            });
        }
        resp.context_menu(|ui| {
            if ui.button(tr().open).clicked() {
                actions.push(Action::Open(i));
                ui.close();
            }
            ui.separator();
            if ui.button(tr().cut_menu).clicked() {
                actions.push(Action::Clipboard(true));
                ui.close();
            }
            if ui.button(tr().copy_menu).clicked() {
                actions.push(Action::Clipboard(false));
                ui.close();
            }
            if ui.button(tr().paste_here_menu).clicked() {
                actions.push(Action::Paste);
                ui.close();
            }
            if ui.button(tr().delete_menu).clicked() {
                actions.push(Action::Delete);
                ui.close();
            }
            if !e.is_dir {
                ui.separator();
                if ui.button(tr().rotate_left).clicked() {
                    actions.push(Action::Rotate(3));
                    ui.close();
                }
                if ui.button(tr().rotate_right).clicked() {
                    actions.push(Action::Rotate(1));
                    ui.close();
                }
                if ui.button(tr().convert_resize_menu).clicked() {
                    actions.push(Action::Convert);
                    ui.close();
                }
            }
        });
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        let inner = rect.shrink(3.0);
        let selected = self.selected.contains(&i);
        if selected {
            painter.rect_filled(inner, CornerRadius::same(4), Color32::from_rgb(45, 75, 120));
        } else if resp.hovered() {
            painter.rect_filled(inner, CornerRadius::same(4), Color32::from_gray(50));
        }
        if self.cursor == Some(i) {
            painter.rect_stroke(
                inner,
                CornerRadius::same(4),
                Stroke::new(1.5, Color32::from_rgb(90, 150, 230)),
                StrokeKind::Inside,
            );
        }
        let img_box = Rect::from_min_size(rect.min + vec2(7.0, 7.0), Vec2::splat(self.thumb_disp));
        let is_cut = self.cut_paths.contains(&e.path);
        if e.is_dir {
            draw_folder(painter, img_box, if is_cut { 0.45 } else { 1.0 });
        } else {
            match self.thumbs.get(&e.path) {
                Some(Thumb::Ready(tex, size)) => {
                    let r = fit_rect(img_box, vec2(size[0] as f32, size[1] as f32), false);
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        if is_cut {
                            Color32::from_white_alpha(115)
                        } else {
                            Color32::WHITE
                        },
                    );
                }
                Some(Thumb::Failed { at, tries }) => {
                    if *tries < THUMB_RETRIES && ui.input(|i| i.time) - at > THUMB_RETRY_SECS {
                        wanted.push(e.path.clone());
                    }
                    painter.text(
                        img_box.center(),
                        egui::Align2::CENTER_CENTER,
                        "✖",
                        egui::FontId::proportional(24.0),
                        Color32::from_gray(120),
                    );
                }
                None => {
                    painter.rect_filled(
                        img_box.shrink(img_box.width() * 0.3),
                        CornerRadius::same(4),
                        Color32::from_gray(44),
                    );
                    wanted.push(e.path.clone());
                }
            }
        }
        // file name (single line, elided)
        let mut job = egui::text::LayoutJob::single_section(
            e.name.clone(),
            egui::TextFormat::simple(egui::FontId::proportional(12.5), Color32::from_gray(210)),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(cell.x - 10.0);
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let text_pos = pos2(rect.center().x, rect.bottom() - 20.0);
        painter.galley(text_pos, galley, Color32::from_gray(210));
    }

    // ------------------------------------------------------------------ viewer

    fn viewer_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = self.ctx.clone();
        let Some(path) = self.view_path.clone() else {
            self.mode = Mode::Browser;
            return;
        };

        // Preload current, next, previous.
        let mut wanted = vec![path.clone()];
        if let Some(i) = self.view_idx {
            for j in [i as i64 + 1, i as i64 - 1, i as i64 + 2] {
                if j >= 0 && (j as usize) < self.images.len() {
                    wanted.push(self.image_path(j as usize).to_path_buf());
                }
            }
        }
        let keep: HashSet<PathBuf> = {
            let mut k: HashSet<PathBuf> = wanted.iter().cloned().collect();
            if let Some(i) = self.view_idx {
                if i >= 2 {
                    k.insert(self.image_path(i - 2).to_path_buf());
                }
            }
            k
        };
        self.full.retain(|p, _| keep.contains(p));
        self.full_pool
            .set_wanted(wanted.into_iter().filter(|p| !self.full.contains_key(p)));

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));

        // ---- keys
        let mut step: i64 = 0;
        let mut back = false;
        let mut save_now = false;
        let mut copy_now = false;
        let mut delete_now = false;
        let mut to_comic = false;
        if !ctx.egui_wants_keyboard_input()
            && self.convert.is_none()
            && self.delete_confirm.is_none()
        {
            ctx.input_mut(|i| {
                if i.key_pressed(Key::ArrowRight)
                    || i.key_pressed(Key::ArrowDown)
                    || i.key_pressed(Key::PageDown)
                    || i.key_pressed(Key::Space)
                {
                    step += 1;
                }
                if i.key_pressed(Key::ArrowLeft)
                    || i.key_pressed(Key::ArrowUp)
                    || i.key_pressed(Key::PageUp)
                    || i.key_pressed(Key::Backspace)
                {
                    step -= 1;
                }
                if i.key_pressed(Key::Home) {
                    step = i64::MIN / 2;
                }
                if i.key_pressed(Key::End) {
                    step = i64::MAX / 2;
                }
                if i.key_pressed(Key::Enter) {
                    back = true;
                }
                if i.key_pressed(Key::Escape) {
                    if fullscreen {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    } else {
                        back = true;
                    }
                }
                if i.key_pressed(Key::F11) || (i.key_pressed(Key::F) && !i.modifiers.command) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
                }
                if i.consume_key(Modifiers::COMMAND, Key::S) {
                    save_now = true;
                }
                if i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
                    copy_now = true;
                }
                if i.key_pressed(Key::Delete) && !i.modifiers.shift {
                    delete_now = true;
                }
                if i.key_pressed(Key::E) && !i.modifiers.command {
                    self.rot = (self.rot + 1) % 4;
                }
                if (i.key_pressed(Key::Q) || i.key_pressed(Key::L)) && !i.modifiers.command {
                    self.rot = (self.rot + 3) % 4;
                }
                if i.key_pressed(Key::Num0) || i.key_pressed(Key::Z) {
                    self.zoom = None;
                    self.pan = Vec2::ZERO;
                }
                if i.key_pressed(Key::Num1) {
                    self.zoom = Some(1.0);
                    self.pan = Vec2::ZERO;
                }
                if i.key_pressed(Key::C) && !i.modifiers.command {
                    to_comic = true;
                }
            });
        }
        if copy_now {
            self.copy_to_clipboard(vec![path.clone()], false);
        }
        if delete_now {
            self.request_delete(vec![path.clone()]);
        }
        let key_r = !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(Key::R) && !i.modifiers.command);

        // ---- top bar
        let mut do_save = save_now;
        let mut do_convert = key_r;
        if !fullscreen {
            egui::Panel::top("viewer_top").show(ui, |ui| {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    if ui.button(tr().back_enter).clicked() {
                        back = true;
                    }
                    if ui.button("⏴").on_hover_text(tr().prev_tip).clicked() {
                        step -= 1;
                    }
                    if ui.button("⏵").on_hover_text(tr().next_tip).clicked() {
                        step += 1;
                    }
                    ui.separator();
                    if ui.button("↺").on_hover_text(tr().rotate_left_tip).clicked() {
                        self.rot = (self.rot + 3) % 4;
                    }
                    if ui
                        .button("↻")
                        .on_hover_text(tr().rotate_right_tip)
                        .clicked()
                    {
                        self.rot = (self.rot + 1) % 4;
                    }
                    if self.rot != 0
                        && ui
                            .button(
                                egui::RichText::new(tr().save_rotation)
                                    .color(Color32::from_rgb(255, 200, 90)),
                            )
                            .clicked()
                    {
                        do_save = true;
                    }
                    if ui
                        .button(format!("{} (R)", tr().convert_resize_menu))
                        .clicked()
                    {
                        do_convert = true;
                    }
                    ui.separator();
                    if ui
                        .button(tr().comic_mode)
                        .on_hover_text(tr().comic_tip)
                        .clicked()
                    {
                        to_comic = true;
                    }
                    ui.separator();
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let pos = match self.view_idx {
                        Some(i) => format!("{}/{}", i + 1, self.images.len()),
                        None => "-".into(),
                    };
                    let mut info = format!("{pos}  {name}");
                    if let Some(Full::Ready(_, [w, h])) = self.full.get(&path) {
                        info += &format!("  {w}×{h}");
                    }
                    ui.label(info);
                });
                ui.add_space(3.0);
            });
        }

        // ---- image area
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            let resp = ui.allocate_rect(rect, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, Color32::from_gray(18));

            // wheel: navigate; Ctrl+wheel / pinch: zoom
            let mut zoom_factor = 1.0f32;
            let pointer = ctx.input(|i| i.pointer.hover_pos());
            // Line-based wheels: at most one step per frame (some platforms, e.g. X11,
            // deliver duplicate wheel events for one notch).
            let frame_nr = ctx.cumulative_frame_nr();
            if resp.hovered() && self.last_wheel_frame != frame_nr {
                self.last_wheel_frame = frame_nr;
                let (mut line_nav, mut line_zoom) = (0.0f32, 0.0f32);
                ctx.input(|i| {
                    for ev in &i.events {
                        match ev {
                            egui::Event::MouseWheel {
                                unit,
                                delta,
                                modifiers,
                                ..
                            } => {
                                let line = !matches!(unit, egui::MouseWheelUnit::Point);
                                match (modifiers.command, line) {
                                    (true, true) => line_zoom += delta.y,
                                    (true, false) => zoom_factor *= 1.15f32.powf(delta.y / 100.0),
                                    (false, true) => line_nav += delta.y,
                                    (false, false) => {
                                        self.wheel_acc += delta.y;
                                        if self.wheel_acc > 80.0 {
                                            step -= 1;
                                            self.wheel_acc = 0.0;
                                        } else if self.wheel_acc < -80.0 {
                                            step += 1;
                                            self.wheel_acc = 0.0;
                                        }
                                    }
                                }
                            }
                            egui::Event::Zoom(f) => zoom_factor *= *f,
                            _ => {}
                        }
                    }
                });
                if line_nav != 0.0 {
                    step -= line_nav.signum() as i64;
                }
                if line_zoom != 0.0 {
                    zoom_factor *= 1.2f32.powf(line_zoom.signum());
                }
            }
            ctx.input(|i| {
                if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                    zoom_factor *= 1.25;
                }
                if i.key_pressed(Key::Minus) {
                    zoom_factor /= 1.25;
                }
            });
            if resp.double_clicked() {
                back = true;
            }
            if resp.middle_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }

            match self.full.get(&path) {
                Some(Full::Ready(tex, orig)) => {
                    let (mut w, mut h) = (orig[0] as f32, orig[1] as f32);
                    if self.rot % 2 == 1 {
                        std::mem::swap(&mut w, &mut h);
                    }
                    let fit = (rect.width() / w).min(rect.height() / h).min(1.0);
                    let scale = self.zoom.unwrap_or(fit);
                    if zoom_factor != 1.0 {
                        let new_scale = (scale * zoom_factor).clamp(0.02, 32.0);
                        let anchor = pointer
                            .filter(|p| rect.contains(*p))
                            .unwrap_or(rect.center());
                        let center = rect.center() + self.pan;
                        let new_center = anchor - (anchor - center) * (new_scale / scale);
                        self.pan = new_center - rect.center();
                        self.zoom = Some(new_scale);
                    }
                    let scale = self.zoom.unwrap_or(fit);
                    if resp.dragged() {
                        self.pan += resp.drag_delta();
                    }
                    let size = vec2(w * scale, h * scale);
                    // keep the image from being dragged completely away
                    let max_pan = ((size - rect.size()) * 0.5).max(Vec2::ZERO);
                    self.pan = self.pan.clamp(-max_pan, max_pan);
                    let img_rect = Rect::from_center_size(rect.center() + self.pan, size);
                    let img_rect = Rect::from_min_size(snap(img_rect.min), img_rect.size());
                    painter.add(rotated_mesh(tex.id(), img_rect, self.rot));

                    if self.zoom.is_some() || fullscreen {
                        painter.text(
                            rect.right_bottom() - vec2(10.0, 8.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("{:.0}%", scale * 100.0),
                            egui::FontId::proportional(13.0),
                            Color32::from_gray(160),
                        );
                    }
                }
                Some(Full::Failed(e)) => {
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        fill(tr().cannot_open, &[e]),
                        egui::FontId::proportional(16.0),
                        Color32::from_rgb(230, 120, 120),
                    );
                }
                None => {
                    // Show the thumbnail (if we have it) while the full image decodes.
                    if let Some(Thumb::Ready(tex, s)) = self.thumbs.get(&path) {
                        let r = fit_rect(rect, vec2(s[0] as f32, s[1] as f32), true);
                        painter.add(rotated_mesh(tex.id(), r, 0));
                    }
                }
            }
        });

        if step != 0 {
            self.view_step(step);
        }
        if do_save && self.rot != 0 {
            let r = self.rot;
            self.start_rotate(vec![path.clone()], r);
        }
        if do_convert {
            let r = self.rot;
            self.open_convert(vec![path.clone()], r);
        }
        if back {
            self.back_to_browser();
        } else if to_comic {
            if self.rot != 0 {
                self.toast(tr().rotation_discarded);
            }
            self.reset_view();
            self.comic = true;
        }
    }

    // ------------------------------------------------------------------ dialogs

    fn convert_window(&mut self) {
        let Some(d) = &mut self.convert else { return };
        let mut start = false;
        let mut cancel = false;
        let ctx = self.ctx.clone();
        egui::Window::new(tr().convert_title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                ui.label(fill(tr().n_files, &[&d.files.len()]));
                if d.rotate != 0 {
                    ui.label(fill(tr().includes_rotation, &[&(d.rotate as u32 * 90)]));
                }
                ui.add_space(6.0);
                egui::Grid::new("conv_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(tr().format);
                        ui.horizontal(|ui| {
                            for f in [
                                OutFormat::Keep,
                                OutFormat::Jpg,
                                OutFormat::Png,
                                OutFormat::Webp,
                            ] {
                                ui.radio_value(&mut d.format, f, f.label());
                            }
                        });
                        ui.end_row();

                        ui.label(tr().quality);
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut d.quality, 1..=100));
                            if d.format == OutFormat::Webp && d.quality >= 100 {
                                ui.label(tr().lossless);
                            }
                            if d.format == OutFormat::Png {
                                ui.label(tr().png_lossless);
                            }
                        });
                        ui.end_row();

                        ui.label(tr().resize);
                        ui.vertical(|ui| {
                            let kinds = tr().resize_kinds;
                            egui::ComboBox::from_id_salt("resize_kind")
                                .selected_text(kinds[d.resize_kind])
                                .show_ui(ui, |ui| {
                                    for (i, k) in kinds.iter().enumerate() {
                                        ui.selectable_value(&mut d.resize_kind, i, *k);
                                    }
                                });
                            match d.resize_kind {
                                1 => {
                                    ui.add(
                                        egui::Slider::new(&mut d.percent, 1.0..=400.0).suffix(" %"),
                                    );
                                }
                                2 => {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut d.long_side)
                                                .range(1..=65535)
                                                .suffix(" px"),
                                        );
                                        for v in [1024u32, 1920, 2560, 3840] {
                                            if ui.small_button(v.to_string()).clicked() {
                                                d.long_side = v;
                                            }
                                        }
                                    });
                                }
                                3 | 4 => {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut d.w)
                                                .range(1..=65535)
                                                .prefix(tr().width_prefix),
                                        );
                                        ui.label("×");
                                        ui.add(
                                            egui::DragValue::new(&mut d.h)
                                                .range(1..=65535)
                                                .prefix(tr().height_prefix),
                                        );
                                    });
                                }
                                _ => {}
                            }
                            if d.resize_kind != 0 {
                                ui.checkbox(&mut d.no_upscale, tr().no_upscale);
                            }
                        });
                        ui.end_row();

                        ui.label(tr().output);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut d.subfolder, false, tr().same_folder);
                                ui.radio_value(&mut d.subfolder, true, tr().subfolder);
                                if d.subfolder {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut d.subfolder_name)
                                            .desired_width(110.0),
                                    );
                                }
                            });
                            ui.checkbox(&mut d.overwrite, tr().overwrite);
                        });
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(format!("  {}  ", tr().start)).clicked() {
                        start = true;
                    }
                    if ui.button(tr().cancel).clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
        if start && self.batch.is_none() {
            let d = self.convert.take().unwrap();
            let resize = match d.resize_kind {
                1 => ResizeMode::Percent(d.percent),
                2 => ResizeMode::LongSide(d.long_side),
                3 => ResizeMode::Fit(d.w, d.h),
                4 => ResizeMode::Exact(d.w, d.h),
                _ => ResizeMode::None,
            };
            let sub = d.subfolder_name.trim().to_string();
            let opt = ConvertOptions {
                format: d.format,
                quality: d.quality,
                resize,
                no_upscale: d.no_upscale,
                out_dir: None,
                overwrite: d.overwrite,
                rotate: d.rotate,
            };
            let subfolder = d.subfolder && !sub.is_empty();
            let files = d.files;
            let b = Batch::spawn(
                fill(tr().converting_n, &[&files.len()]),
                files,
                ctx.clone(),
                move |p| {
                    let mut o = opt.clone();
                    if subfolder {
                        o.out_dir = p.parent().map(|d| d.join(&sub));
                    }
                    ops::convert_file(p, &o).map(Some)
                },
            );
            self.batch = Some((b, false));
            if d.rotate != 0 {
                self.rot = 0;
            }
        } else if cancel {
            self.convert = None;
        }
    }

    fn batch_window(&mut self) {
        let Some((b, _)) = &self.batch else { return };
        let done = b.done.load(Ordering::SeqCst);
        let finished = b.finished.load(Ordering::SeqCst);
        let mut close = false;
        let ctx = self.ctx.clone();
        egui::Window::new(&b.title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                ui.add(
                    egui::ProgressBar::new(done as f32 / b.total.max(1) as f32)
                        .text(format!("{done} / {}", b.total))
                        .desired_width(320.0),
                );
                if finished {
                    let errs = b.errors.lock().unwrap();
                    ui.label(fill(tr().done_errors_n, &[&errs.len()]));
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for e in errs.iter() {
                                ui.colored_label(Color32::from_rgb(230, 120, 120), e);
                            }
                        });
                    if ui.button(tr().close).clicked() {
                        close = true;
                    }
                }
            });
        if close {
            self.batch = None;
        }
    }

    fn settings_window(&mut self) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut s = self.settings.clone();
        egui::Window::new(tr().settings_title)
            .id(egui::Id::new("settings_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(&self.ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(tr().language);
                        let lang_text = |l: Option<i18n::Lang>| match l {
                            Some(l) => l.native_name().to_string(),
                            None => {
                                format!("{} ({})", tr().lang_system, i18n::system().native_name())
                            }
                        };
                        egui::ComboBox::from_id_salt("lang")
                            .selected_text(lang_text(s.lang))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut s.lang, None, lang_text(None));
                                for l in i18n::Lang::ALL {
                                    ui.selectable_value(&mut s.lang, Some(l), lang_text(Some(l)));
                                }
                            });
                        ui.end_row();

                        ui.label(tr().scroll_per_notch);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut s.scroll_rows)
                                    .range(0.25..=10.0)
                                    .speed(0.05)
                                    .fixed_decimals(2),
                            );
                            ui.label(tr().thumb_rows);
                            for v in [0.5f32, 1.0, 2.0, 3.0] {
                                if ui.small_button(format!("{v}")).clicked() {
                                    s.scroll_rows = v;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(tr().comic_scroll_per_notch);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut s.comic_scroll_step)
                                    .range(util::COMIC_STEP_MIN..=util::COMIC_STEP_MAX)
                                    .speed(2.0)
                                    .fixed_decimals(0)
                                    .suffix(" px"),
                            );
                            for v in [80.0f32, 150.0, 300.0, 600.0] {
                                if ui.small_button(format!("{v}")).clicked() {
                                    s.comic_scroll_step = v;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(tr().smooth_scroll);
                        ui.checkbox(&mut s.smooth_scroll, "");
                        ui.end_row();

                        ui.label(tr().confirm_delete);
                        ui.checkbox(&mut s.confirm_delete, "");
                        ui.end_row();

                        ui.label(tr().window_on_start);
                        ui.vertical(|ui| {
                            ui.radio_value(
                                &mut s.window_mode,
                                util::WindowMode::Remember,
                                tr().remember_window,
                            );
                            ui.radio_value(
                                &mut s.window_mode,
                                util::WindowMode::Maximized,
                                tr().always_maximized,
                            );
                        });
                        ui.end_row();

                        if cfg!(windows) {
                            ui.label(tr().after_close);
                            ui.vertical(|ui| {
                                ui.checkbox(&mut s.stay_resident, tr().stay_resident);
                                ui.add_enabled_ui(s.stay_resident, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut s.idle_exit_min)
                                                .range(0..=1440)
                                                .speed(0.2),
                                        );
                                        ui.label(tr().idle_exit);
                                    });
                                });
                            });
                            ui.end_row();
                        }
                    });
            });
        self.settings_open = open;
        if s.lang != self.settings.lang {
            let lang = i18n::resolve(s.lang);
            if lang != i18n::current() {
                i18n::set(lang);
                // Primary CJK font follows the language (Han glyph shapes differ).
                self.fonts_rx = Some(load_fonts_async(&self.ctx, lang));
            }
        }
        if s != self.settings {
            self.settings = s;
            self.mark_settings_dirty();
        }
    }

    fn mark_settings_dirty(&mut self) {
        if self.settings_dirty_at.is_none() {
            self.settings_dirty_at = Some(self.ctx.input(|i| i.time));
        }
    }

    /// Track window state / thumbnail size and write settings to disk (debounced).
    fn track_settings(&mut self) {
        let (now, max, full, min, inner, outer, closing) = self.ctx.input(|i| {
            let v = i.viewport();
            (
                i.time,
                v.maximized,
                v.fullscreen,
                v.minimized,
                v.inner_rect,
                v.outer_rect,
                v.close_requested(),
            )
        });
        let mut s = self.settings.clone();
        s.thumb_size = self.thumb_disp;
        // Skip the first moment after startup: the window may not have reached its
        // initial (e.g. maximized) state yet.
        if now > 1.0 && full != Some(true) && min != Some(true) {
            if let Some(m) = max {
                s.win_maximized = Some(m);
                if !m {
                    if let Some(r) = inner.filter(|r| r.width() >= 200.0 && r.height() >= 150.0) {
                        s.win_size = Some([r.width().round(), r.height().round()]);
                    }
                    if let Some(r) = outer {
                        s.win_pos = Some([r.min.x.round(), r.min.y.round()]);
                    }
                }
            }
        }
        if s != self.settings {
            self.settings = s;
            self.mark_settings_dirty();
        }
        if let Some(t) = self.settings_dirty_at {
            if closing || now - t > 0.8 {
                self.settings.save();
                self.settings_dirty_at = None;
            } else {
                self.ctx
                    .request_repaint_after(std::time::Duration::from_millis(900));
            }
        }
    }

    fn update_title(&mut self) {
        let t = match self.mode {
            Mode::Viewer => self
                .view_path
                .as_ref()
                .map(|p| {
                    format!(
                        "{} - Hayabubusa",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
                .unwrap_or_default(),
            Mode::Browser => format!("{} - Hayabubusa", self.dir.to_string_lossy()),
        };
        if t != self.title {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.title = t;
        }
    }

    /// Open a file or folder handed over by another launch / the tray.
    fn open_path(&mut self, p: PathBuf) {
        if p.is_dir() {
            self.set_dir(p, None);
            self.mode = Mode::Browser;
        } else if p.is_file() {
            let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
            self.view_path = Some(p.clone());
            self.set_dir(dir, Some(p));
            self.reset_view();
            self.view_idx = None;
            self.mode = Mode::Viewer;
        }
    }

    /// Tray / single-instance messages, and turning "close" into "hide to tray".
    /// Returns `true` if this frame should draw nothing (window being hidden).
    fn resident_frame(&mut self) -> bool {
        while let Ok(m) = self.resident_rx.try_recv() {
            match m {
                resident::Msg::Open(Some(p)) => {
                    self.hide_pending = false;
                    self.open_path(p);
                }
                resident::Msg::Open(None) => self.hide_pending = false,
                resident::Msg::Quit => {
                    self.quitting = true;
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        if self.hide_pending {
            self.hide_pending = false;
            // Free what is only needed while visible (GPU textures, pending decodes).
            self.full.clear();
            self.thumbs.clear();
            self.free_comic();
            self.full_pool.set_wanted([]);
            self.thumb_pool.set_wanted([]);
            while self.full_pool.rx.try_recv().is_ok() {}
            while self.thumb_pool.rx.try_recv().is_ok() {}
            self.reset_view();
            resident::hide(self.settings.idle_exit_min);
            return true;
        }

        let closing = self.ctx.input(|i| i.viewport().close_requested());
        if closing
            && !self.quitting
            && self.settings.stay_resident
            && resident::start(true)
        {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            self.convert = None;
            self.delete_confirm = None;
            self.settings_open = false;
            self.hide_pending = true;
            self.ctx.request_repaint();
        }
        false
    }

    fn handle_drop(&mut self) {
        let dropped: Vec<PathBuf> = self.ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            if p.is_dir() {
                self.set_dir(p, None);
                self.mode = Mode::Browser;
            } else if util::is_image(&p) {
                let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
                self.view_path = Some(p.clone());
                self.set_dir(dir, Some(p));
                self.reset_view();
                self.view_idx = None;
                self.mode = Mode::Viewer;
            }
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        resident::shutdown();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.resident_frame() {
            return;
        }
        self.poll_scan();
        self.poll_workers();
        self.handle_drop();
        self.check_cut_on_focus();
        self.poll_fs();
        self.poll_watch();
        // F5 works in both modes, even while the path box has focus.
        if self.convert.is_none()
            && self.delete_confirm.is_none()
            && self.ctx.input(|i| i.key_pressed(Key::F5))
        {
            self.refresh_all();
        }

        match self.mode {
            Mode::Browser => self.browser_ui(ui),
            Mode::Viewer if self.comic => self.comic_ui(ui),
            Mode::Viewer => self.viewer_ui(ui),
        }
        self.convert_window();
        self.batch_window();
        self.settings_window();
        self.delete_confirm_window();
        self.track_settings();

        if let Some((msg, until)) = &self.toast {
            let now = self.ctx.input(|i| i.time);
            if now > *until {
                self.toast = None;
            } else {
                let msg = msg.clone();
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -40.0))
                    .show(&self.ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(msg);
                        });
                    });
                self.ctx
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        self.update_title();
    }
}

// ---------------------------------------------------------------------- helpers

fn snap(p: Pos2) -> Pos2 {
    pos2(p.x.round(), p.y.round())
}

/// Fit `size` into `bounds` keeping aspect ratio (optionally allowing upscale).
fn fit_rect(bounds: Rect, size: Vec2, upscale: bool) -> Rect {
    let mut s = (bounds.width() / size.x).min(bounds.height() / size.y);
    if !upscale {
        s = s.min(1.0);
    }
    Rect::from_center_size(bounds.center(), size * s)
}

fn rotated_mesh(tex: egui::TextureId, r: Rect, rot: u8) -> Shape {
    let mut mesh = egui::Mesh::with_texture(tex);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ];
    let uvs = [
        pos2(0.0, 0.0),
        pos2(1.0, 0.0),
        pos2(1.0, 1.0),
        pos2(0.0, 1.0),
    ];
    for k in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: corners[k],
            uv: uvs[(k + 4 - rot as usize % 4) % 4],
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

fn draw_folder(p: &egui::Painter, b: Rect, fade: f32) {
    let w = b.width() * 0.62;
    let h = w * 0.72;
    let body = Rect::from_center_size(b.center() + vec2(0.0, w * 0.05), vec2(w, h));
    let tab = Rect::from_min_size(body.min - vec2(0.0, h * 0.14), vec2(w * 0.4, h * 0.2));
    let c = Color32::from_rgb(225, 180, 80).gamma_multiply(fade);
    p.rect_filled(tab, CornerRadius::same(3), c.gamma_multiply(0.8));
    p.rect_filled(body, CornerRadius::same(4), c);
}

fn scan_dir(dir: &Path) -> Vec<Entry> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for de in rd.flatten() {
            let name = de.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || name == "$RECYCLE.BIN"
                || name == "System Volume Information"
            {
                continue;
            }
            let path = de.path();
            let Ok(ft) = de.file_type() else { continue };
            let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
            if is_dir {
                let md = de.metadata().ok();
                dirs.push(Entry {
                    path,
                    name,
                    is_dir: true,
                    size: 0,
                    modified: md.as_ref().and_then(|m| m.modified().ok()),
                    created: md.as_ref().and_then(|m| m.created().ok()),
                });
            } else if util::is_image(&path) {
                let md = de.metadata().ok();
                let modified = md.as_ref().and_then(|m| m.modified().ok());
                files.push(Entry {
                    path,
                    name,
                    is_dir: false,
                    size: md.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified,
                    // Some filesystems (e.g. older Linux) have no birth time.
                    created: md.as_ref().and_then(|m| m.created().ok()).or(modified),
                });
            }
        }
    }
    dirs.extend(files);
    dirs
}

/// Order-independent fingerprint of a listing (names, sizes, modification times).
fn dir_signature(entries: &[Entry]) -> u64 {
    let mut sig = entries.len() as u64;
    for e in entries {
        let mt = e
            .modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        let mut key = e.path.to_string_lossy().into_owned().into_bytes();
        key.extend_from_slice(&e.size.to_le_bytes());
        key.extend_from_slice(&mt.to_le_bytes());
        key.push(e.is_dir as u8);
        sig = sig.wrapping_add(util::fnv64(&key));
    }
    sig
}

/// Folders always come first; within each group sort by `key`, ties broken by name.
fn sort_entries(v: &mut [Entry], key: util::SortKey, desc: bool) {
    use std::cmp::Ordering;
    use util::SortKey;
    v.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return if a.is_dir { Ordering::Less } else { Ordering::Greater };
        }
        let by_name = || util::natural_cmp(&a.name, &b.name);
        let ord = match key {
            SortKey::Name => by_name(),
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(by_name),
            SortKey::Created => a.created.cmp(&b.created).then_with(by_name),
            SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
            SortKey::Type => util::ext_lower(&a.path)
                .cmp(&util::ext_lower(&b.path))
                .then_with(by_name),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

/// Load CJK system fonts for `lang` on a background thread.
fn load_fonts_async(ctx: &egui::Context, lang: i18n::Lang) -> Receiver<egui::FontDefinitions> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let Some(defs) = crate::fonts::cjk_font_definitions(lang) {
            let _ = tx.send(defs);
            ctx.request_repaint();
        }
    });
    rx
}
