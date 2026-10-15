//! Comic mode: every image of the folder stacked top to bottom in one long strip,
//! all at the same width. Wheel / keys scroll, Ctrl+wheel zooms, drag pans.
//!
//! The scroll position is kept as "image at the top edge + fraction of it" instead of a
//! pixel offset, so the view does not jump when the layout changes (zoom, window resize,
//! image sizes arriving from the background reader, files added or removed).

use super::{fill, rotated_mesh, tr, App, Mode, Thumb, TEX_OPTS};
use crate::decode;
use crate::util;
use eframe::egui::{self, pos2, vec2, Color32, Key, Modifiers, Rect, Sense, TextureHandle};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// Height / width used for images whose size is not known yet.
const DEFAULT_ASPECT: f32 = 1.414;
/// At most this many visible images are decoded at full size at once.
const MAX_FULL_VISIBLE: usize = 12;
/// New textures created per frame. Uploading several big images in one frame is what
/// makes scrolling hitch, so they are spread over frames (results wait in the channel).
const UPLOADS_PER_FRAME: usize = 1;
/// Decode this many screen heights ahead in the scroll direction (and a bit behind).
const AHEAD_SCREENS: f32 = 2.0;
const BEHIND_SCREENS: f32 = 0.75;
/// Textures further than this many screens away are freed.
const KEEP_SCREENS: f32 = 4.0;

/// A comic mode texture.
pub(super) enum ComicFull {
    Ready {
        tex: TextureHandle,
        /// Texture width in pixels.
        tex_w: u32,
        /// Already as sharp as it can get (see `workers::ComicImage::complete`).
        complete: bool,
    },
    Failed(String),
}

/// Vertical layout of the strip.
struct Strip {
    /// Top of each image; one extra entry at the end = total height.
    tops: Vec<f32>,
}

impl Strip {
    fn new(col_w: f32, aspects: &[f32]) -> Self {
        let mut tops = Vec::with_capacity(aspects.len() + 1);
        let mut y = 0.0f32;
        for a in aspects {
            tops.push(y);
            y += (col_w * a).max(1.0);
        }
        tops.push(y);
        Self { tops }
    }

    fn len(&self) -> usize {
        self.tops.len() - 1
    }

    fn total(&self) -> f32 {
        *self.tops.last().unwrap_or(&0.0)
    }

    fn height(&self, i: usize) -> f32 {
        self.tops[i + 1] - self.tops[i]
    }

    fn y_of(&self, i: usize, frac: f32) -> f32 {
        if self.len() == 0 {
            return 0.0;
        }
        let i = i.min(self.len() - 1);
        self.tops[i] + frac * self.height(i)
    }

    /// Image containing strip position `y`, and the fraction into it.
    fn locate(&self, y: f32) -> (usize, f32) {
        let n = self.len();
        if n == 0 {
            return (0, 0.0);
        }
        let y = y.clamp(0.0, self.total());
        let i = self.tops[..n]
            .partition_point(|&t| t <= y)
            .saturating_sub(1);
        let i = i.min(n - 1);
        (i, ((y - self.tops[i]) / self.height(i)).clamp(0.0, 1.0))
    }
}

impl App {
    /// Take finished comic images from the workers (a limited number per frame).
    pub(super) fn poll_comic(&mut self) {
        for _ in 0..UPLOADS_PER_FRAME {
            let Ok(r) = self.comic_pool.rx.try_recv() else {
                return;
            };
            let st = match r.result {
                Ok(img) => {
                    self.dims.insert(r.path.clone(), img.orig);
                    let tex = self
                        .ctx
                        .load_texture(r.path.to_string_lossy(), img.image, TEX_OPTS);
                    ComicFull::Ready {
                        tex,
                        tex_w: img.tex_w,
                        complete: img.complete,
                    }
                }
                Err(e) => ComicFull::Failed(e),
            };
            self.comic_full.insert(r.path, st);
        }
        if !self.comic_pool.rx.is_empty() {
            self.ctx.request_repaint();
        }
    }

    /// Drop comic textures and pending decodes (leaving comic mode / hiding).
    pub(super) fn free_comic(&mut self) {
        self.comic_full.clear();
        self.comic_pool.set_wanted([]);
        while self.comic_pool.rx.try_recv().is_ok() {}
    }

    /// Where the strip is scrolled to: (image index, fraction into it).
    fn comic_anchor(&self) -> Option<(usize, f32)> {
        if let Some((p, f)) = &self.comic_top {
            if let Some(i) = self.images.iter().position(|&e| self.entries[e].path == *p) {
                return Some((i, *f));
            }
        }
        self.view_idx.map(|i| (i, 0.0))
    }

    pub(super) fn comic_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = self.ctx.clone();
        if self.view_path.is_none() {
            self.mode = Mode::Browser;
            return;
        }
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let smooth = self.settings.smooth_scroll;
        let line_step = self.settings.comic_scroll_step;

        // ---- keys
        let mut scroll = 0.0f32; // points, + = down (lines / pages are converted later)
        let mut pages = 0.0f32; // view heights
        let mut jump: i64 = 0; // whole images
        let mut home = false;
        let mut end = false;
        let mut back = false;
        let mut to_single = false;
        let mut zoom_factor = 1.0f32;
        let mut set_zoom: Option<ZoomTo> = None;
        let mut copy_now = false;
        let mut delete_now = false;
        let mut convert_now = false;
        if !ctx.egui_wants_keyboard_input()
            && self.convert.is_none()
            && self.delete_confirm.is_none()
        {
            ctx.input_mut(|i| {
                if i.key_pressed(Key::ArrowDown) {
                    scroll += line_step;
                }
                if i.key_pressed(Key::ArrowUp) {
                    scroll -= line_step;
                }
                if i.key_pressed(Key::PageDown) || (i.key_pressed(Key::Space) && !i.modifiers.shift)
                {
                    pages += 0.9;
                }
                if i.key_pressed(Key::PageUp) || (i.key_pressed(Key::Space) && i.modifiers.shift) {
                    pages -= 0.9;
                }
                if i.key_pressed(Key::ArrowRight) {
                    jump += 1;
                }
                if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::Backspace) {
                    jump -= 1;
                }
                if i.key_pressed(Key::Home) {
                    home = true;
                }
                if i.key_pressed(Key::End) {
                    end = true;
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
                if i.key_pressed(Key::C) && !i.modifiers.command {
                    to_single = true;
                }
                if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                    zoom_factor *= 1.25;
                }
                if i.key_pressed(Key::Minus) {
                    zoom_factor /= 1.25;
                }
                if i.key_pressed(Key::Num0) || i.key_pressed(Key::Z) {
                    set_zoom = Some(ZoomTo::FitWidth);
                }
                if i.key_pressed(Key::Num1) {
                    set_zoom = Some(ZoomTo::Actual);
                }
                if i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
                    copy_now = true;
                }
                if i.key_pressed(Key::Delete) && !i.modifiers.shift {
                    delete_now = true;
                }
                if i.key_pressed(Key::R) && !i.modifiers.command {
                    convert_now = true;
                }
                // Ctrl+S has no meaning here; keep it from reaching anything else.
                i.consume_key(Modifiers::COMMAND, Key::S);
            });
        }

        // ---- top bar
        let cur_path = self.view_path.clone().unwrap_or_default();
        if !fullscreen {
            egui::Panel::top("comic_top").show(ui, |ui| {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    if ui.button(tr().back_enter).clicked() {
                        back = true;
                    }
                    if ui
                        .button(tr().single_view)
                        .on_hover_text(tr().single_view_tip)
                        .clicked()
                    {
                        to_single = true;
                    }
                    ui.separator();
                    if ui.button("⏶").on_hover_text(tr().prev_tip).clicked() {
                        jump -= 1;
                    }
                    if ui.button("⏷").on_hover_text(tr().next_tip).clicked() {
                        jump += 1;
                    }
                    ui.separator();
                    if ui.button(" − ").on_hover_text(tr().zoom_out_tip).clicked() {
                        zoom_factor /= 1.25;
                    }
                    // Actual scale of the current image (like the single view), not the
                    // fraction of the window width.
                    let col_w = ctx.content_rect().width() * self.settings.comic_zoom;
                    let pct = match self.dims.get(&cur_path) {
                        Some(&[w, _]) if w > 0 => format!("{:.0}%", col_w / w as f32 * 100.0),
                        _ => format!("{:.0}%", self.settings.comic_zoom * 100.0),
                    };
                    ui.label(pct);
                    if ui.button(" + ").on_hover_text(tr().zoom_in_tip).clicked() {
                        zoom_factor *= 1.25;
                    }
                    if ui.button(tr().fit_width).clicked() {
                        set_zoom = Some(ZoomTo::FitWidth);
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} (R)", tr().convert_resize_menu))
                        .clicked()
                    {
                        convert_now = true;
                    }
                    if ui.button(tr().settings_btn).clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    ui.separator();
                    let name = cur_path.file_name().unwrap_or_default().to_string_lossy();
                    let pos = match self.view_idx {
                        Some(i) => format!("{}/{}", i + 1, self.images.len()),
                        None => "-".into(),
                    };
                    let mut info = format!("{pos}  {name}");
                    if let Some([w, h]) = self.dims.get(&cur_path).filter(|d| d[0] > 0) {
                        info += &format!("  {w}×{h}");
                    }
                    ui.label(info);
                });
                ui.add_space(3.0);
            });
        }

        // ---- strip
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            let resp = ui.allocate_rect(rect, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, Color32::from_gray(18));
            let ppp = ctx.pixels_per_point();
            let view_h = rect.height().max(1.0);
            let pointer = ctx.input(|i| i.pointer.hover_pos());

            // wheel: scroll; Ctrl+wheel / pinch: zoom
            if resp.hovered() {
                ctx.input(|i| {
                    for ev in &i.events {
                        match ev {
                            egui::Event::MouseWheel {
                                unit,
                                delta,
                                modifiers,
                                ..
                            } => {
                                if modifiers.command {
                                    zoom_factor *= match unit {
                                        egui::MouseWheelUnit::Point => {
                                            1.15f32.powf(delta.y / 100.0)
                                        }
                                        _ => 1.2f32.powf(delta.y.signum()),
                                    };
                                    continue;
                                }
                                let d = if delta.y != 0.0 { delta.y } else { delta.x };
                                match unit {
                                    egui::MouseWheelUnit::Line => scroll -= d * line_step,
                                    egui::MouseWheelUnit::Point => scroll -= d,
                                    egui::MouseWheelUnit::Page => pages -= d,
                                }
                            }
                            egui::Event::Zoom(f) => zoom_factor *= *f,
                            _ => {}
                        }
                    }
                });
            }
            if resp.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }

            let n = self.images.len();
            let Some((ti, tf)) = self.comic_anchor().filter(|_| n > 0) else {
                let msg = if self.scan_rx.is_some() {
                    "…".to_string()
                } else {
                    tr().no_images.to_string()
                };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(16.0),
                    Color32::from_gray(160),
                );
                return;
            };

            // Layout: every image at the same width; unknown sizes use the thumbnail's
            // shape, else a typical page shape, and are corrected once read.
            let aspects: Vec<f32> = self
                .images
                .iter()
                .map(|&e| {
                    let p = &self.entries[e].path;
                    match self.dims.get(p) {
                        Some(&[w, h]) if w > 0 && h > 0 => h as f32 / w as f32,
                        _ => match self.thumbs.get(p) {
                            Some(Thumb::Ready(_, [w, h])) if *w > 0 && *h > 0 => {
                                *h as f32 / *w as f32
                            }
                            _ => DEFAULT_ASPECT,
                        },
                    }
                })
                .collect();
            let mut col_w = (rect.width() * self.settings.comic_zoom).max(16.0);
            let mut strip = Strip::new(col_w, &aspects);
            let mut off = strip.y_of(ti, tf);

            // ---- zoom (keeps the point under the mouse in place)
            let old_zoom = self.settings.comic_zoom;
            let mut new_zoom = old_zoom * zoom_factor;
            let mut anchor_pt = pointer
                .filter(|p| rect.contains(*p))
                .unwrap_or(rect.center());
            if let Some(z) = set_zoom {
                anchor_pt = pos2(rect.center().x, rect.top());
                new_zoom = match z {
                    ZoomTo::FitWidth => 1.0,
                    ZoomTo::Actual => {
                        let (ci, _) = strip.locate(off + view_h / 3.0);
                        let p = self.image_path(ci);
                        match self.dims.get(p) {
                            Some(&[w, _]) if w > 0 => w as f32 / rect.width(),
                            _ => old_zoom,
                        }
                    }
                };
            }
            let new_zoom = new_zoom.clamp(util::COMIC_ZOOM_MIN, util::COMIC_ZOOM_MAX);
            if (new_zoom - old_zoom).abs() > 1e-4 {
                let ay = anchor_pt.y - rect.top();
                let ax = anchor_pt.x - rect.center().x;
                let (mi, mf) = strip.locate(off + ay);
                let k = new_zoom / old_zoom;
                self.comic_pan_x = if matches!(set_zoom, Some(ZoomTo::FitWidth)) {
                    0.0
                } else {
                    ax - (ax - self.comic_pan_x) * k
                };
                self.settings.comic_zoom = new_zoom;
                self.mark_settings_dirty();
                col_w = (rect.width() * new_zoom).max(16.0);
                strip = Strip::new(col_w, &aspects);
                off = strip.y_of(mi, mf) - ay;
                self.comic_pending *= k;
            }

            // ---- scrolling
            let max_off = (strip.total() - view_h).max(0.0);
            let mut target: Option<f32> = None; // absolute position (jumps)
            if home {
                target = Some(0.0);
            }
            if end {
                target = Some(max_off);
            }
            if jump != 0 {
                // From where the view is heading, so repeated presses keep going.
                let from = off + self.comic_pending;
                let (i, f) = strip.locate(from + 0.5);
                let i = if jump > 0 {
                    (i as i64 + jump).min(n as i64 - 1) as usize
                } else if f * strip.height(i) > 1.0 {
                    // Inside an image: first go back to its top.
                    (i as i64 + jump + 1).max(0) as usize
                } else {
                    (i as i64 + jump).max(0) as usize
                };
                target = Some(strip.tops[i].min(max_off));
            }
            let mut delta = scroll + pages * view_h;
            if let Some(t) = target {
                delta = t - (off + self.comic_pending);
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                self.comic_pan_x += d.x;
                off -= d.y;
                self.comic_pending = 0.0;
            }
            self.comic_pending += delta;
            if self.comic_pending != 0.0 {
                if smooth {
                    let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.05);
                    let k = 1.0 - (-dt * 18.0).exp();
                    let step = if self.comic_pending.abs() < 0.5 {
                        self.comic_pending
                    } else {
                        self.comic_pending * k
                    };
                    off += step;
                    self.comic_pending -= step;
                    ctx.request_repaint();
                } else {
                    off += self.comic_pending;
                    self.comic_pending = 0.0;
                }
            }
            let clamped = off.clamp(0.0, max_off);
            if clamped != off {
                self.comic_pending = 0.0;
            }
            off = (clamped * ppp).round() / ppp;

            let max_pan = ((col_w - rect.width()) * 0.5).max(0.0);
            self.comic_pan_x = self.comic_pan_x.clamp(-max_pan, max_pan);

            // Remember the position, and which image is "current" (the one a third of
            // the way down the view, i.e. the one being read).
            let (ti, tf) = strip.locate(off);
            self.comic_top = Some((self.image_path(ti).to_path_buf(), tf));
            let (ci, _) = strip.locate(off + view_h / 3.0);
            if self.view_idx != Some(ci) {
                self.view_idx = Some(ci);
                self.view_path = Some(self.image_path(ci).to_path_buf());
            }

            // ---- draw visible images
            let snap = |v: f32| (v * ppp).round() / ppp;
            let x0 = snap(rect.center().x - col_w * 0.5 + self.comic_pan_x);
            let x1 = snap(x0 + col_w);
            let use_full = col_w * ppp > decode::THUMB_SIZE as f32;
            let mut visible: Vec<usize> = Vec::new();
            let mut i = ti;
            while i < n && strip.tops[i] < off + view_h {
                visible.push(i);
                let y0 = snap(rect.top() + strip.tops[i] - off);
                let y1 = snap(rect.top() + strip.tops[i + 1] - off);
                let r = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
                let p = self.image_path(i);
                match (
                    self.comic_full.get(p).filter(|_| use_full),
                    self.thumbs.get(p),
                ) {
                    (Some(ComicFull::Ready { tex, .. }), _) => {
                        painter.add(rotated_mesh(tex.id(), r, 0));
                    }
                    (_, Some(Thumb::Ready(tex, _))) => {
                        painter.add(rotated_mesh(tex.id(), r, 0));
                    }
                    (Some(ComicFull::Failed(e)), _) => {
                        painter.rect_filled(r, 0.0, Color32::from_gray(28));
                        painter.text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            fill(tr().cannot_open, &[e]),
                            egui::FontId::proportional(14.0),
                            Color32::from_rgb(230, 120, 120),
                        );
                    }
                    _ => {
                        painter.rect_filled(r, 0.0, Color32::from_gray(28));
                        painter.text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            p.file_name().unwrap_or_default().to_string_lossy(),
                            egui::FontId::proportional(13.0),
                            Color32::from_gray(110),
                        );
                    }
                }
                i += 1;
            }
            let last = visible.last().copied().unwrap_or(ti);

            // scroll position indicator
            if strip.total() > view_h {
                let bar_h = (view_h * view_h / strip.total()).max(24.0).min(view_h);
                let y = rect.top() + (view_h - bar_h) * (off / max_off.max(1.0));
                let bar = Rect::from_min_size(pos2(rect.right() - 7.0, y), vec2(4.0, bar_h));
                painter.rect_filled(bar, 2.0, Color32::from_white_alpha(70));
            }
            if fullscreen {
                let pos = format!("{}/{}", ci + 1, n);
                let pct = match self.dims.get(self.image_path(ci)) {
                    Some(&[w, _]) if w > 0 => format!("{:.0}%", col_w / w as f32 * 100.0),
                    _ => String::new(),
                };
                painter.text(
                    rect.right_bottom() - vec2(14.0, 8.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{pos}  {pct}"),
                    egui::FontId::proportional(13.0),
                    Color32::from_gray(160),
                );
            }

            // ---- background work: images on screen first (middle out), then ahead in
            // the scroll direction; thumbnails as placeholders; header sizes for layout.
            let (images, entries) = (&self.images, &self.entries);
            let path_at = |i: usize| entries[images[i]].path.clone();
            // Index range of images overlapping strip positions y0..y1.
            let span = |y0: f32, y1: f32| {
                let a = strip.locate(y0.max(0.0)).0;
                let b = strip.locate(y1.max(0.0)).0;
                a..(b + 1).min(n)
            };
            let up = self.comic_pending < -0.5;
            let (ahead, behind) = if up {
                (
                    span(off - view_h * AHEAD_SCREENS, off),
                    span(off + view_h, off + view_h * (1.0 + BEHIND_SCREENS)),
                )
            } else {
                (
                    span(off + view_h, off + view_h * (1.0 + AHEAD_SCREENS)),
                    span(off - view_h * BEHIND_SCREENS, off),
                )
            };
            // nearest first
            let ahead: Vec<usize> = if up {
                ahead.rev().collect()
            } else {
                ahead.collect()
            };
            let behind: Vec<usize> = if up {
                behind.collect()
            } else {
                behind.rev().collect()
            };

            // Decode at the shown width (+ some headroom, rounded so small zoom
            // changes do not re-decode everything).
            let need_px = col_w * ppp;
            let target = ((need_px * 1.1 / 128.0).ceil() * 128.0) as u32;
            self.comic_target_w.store(target, Ordering::Relaxed);
            let comic_full = &self.comic_full;
            let needs = |p: &PathBuf| match comic_full.get(p) {
                None => true,
                Some(ComicFull::Ready {
                    tex_w, complete, ..
                }) => !*complete && (*tex_w as f32) < need_px * 0.95,
                Some(ComicFull::Failed(_)) => false,
            };
            let mut wanted_full: Vec<PathBuf> = Vec::new();
            if use_full {
                let mut vis = visible.clone();
                vis.sort_by_key(|&i| (i as i64 - ci as i64).abs());
                wanted_full.extend(
                    vis.into_iter()
                        .take(MAX_FULL_VISIBLE)
                        .chain(ahead.iter().copied())
                        .chain(behind.iter().copied())
                        .map(path_at)
                        .filter(|p| needs(p)),
                );
            }
            self.comic_pool.set_wanted(wanted_full);
            let keep_range = span(
                off - view_h * KEEP_SCREENS,
                off + view_h * (1.0 + KEEP_SCREENS),
            );
            let keep: HashSet<PathBuf> = keep_range.clone().map(path_at).collect();
            self.comic_full.retain(|p, _| keep.contains(p));
            // The single view's full-size images are not used here.
            self.full.clear();

            let wanted_thumbs: Vec<PathBuf> = visible
                .iter()
                .chain(ahead.iter())
                .chain(behind.iter())
                .copied()
                .map(path_at)
                .filter(|p| !self.thumbs.contains_key(p))
                .collect();
            self.thumb_pool.set_wanted(wanted_thumbs);

            let lo = ti.saturating_sub(30);
            let hi = (last + 150).min(n);
            let wanted_dims: Vec<PathBuf> = (ti..hi)
                .chain((lo..ti).rev())
                .map(path_at)
                .filter(|p| !self.dims.contains_key(p))
                .collect();
            self.dims_pool.set_wanted(wanted_dims);
        });

        let cur = self.view_path.clone();
        if let Some(cur) = cur {
            if copy_now {
                self.copy_to_clipboard(vec![cur.clone()], false);
            }
            if delete_now {
                self.request_delete(vec![cur.clone()]);
            }
            if convert_now {
                self.open_convert(vec![cur], 0);
            }
        }
        if back {
            self.free_comic();
            self.back_to_browser();
        } else if to_single {
            self.free_comic();
            self.comic = false;
            self.reset_view();
        }
    }
}

#[derive(Clone, Copy)]
enum ZoomTo {
    FitWidth,
    /// 100% for the image in the middle of the view.
    Actual,
}
