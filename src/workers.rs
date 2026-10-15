//! Background worker pools with a "latest wins" queue: the UI replaces the wanted list
//! every frame, so stale requests (images scrolled away / skipped) are dropped.

use crate::decode;
use crate::util;
use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui::{self, ColorImage};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};

struct Queue {
    jobs: VecDeque<PathBuf>,
    inflight: HashSet<PathBuf>,
}

struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
}

pub struct Pool<R: Send + 'static> {
    shared: Arc<Shared>,
    pub rx: Receiver<R>,
}

impl<R: Send + 'static> Pool<R> {
    fn new<F>(threads: usize, ctx: egui::Context, work: F) -> Self
    where
        F: Fn(PathBuf) -> R + Send + Sync + 'static,
    {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue {
                jobs: VecDeque::new(),
                inflight: HashSet::new(),
            }),
            cv: Condvar::new(),
        });
        let (tx, rx): (Sender<R>, Receiver<R>) = unbounded();
        let work = Arc::new(work);
        for _ in 0..threads.max(1) {
            let shared = shared.clone();
            let tx = tx.clone();
            let work = work.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let path = {
                    let mut q = shared.q.lock().unwrap();
                    loop {
                        if let Some(p) = q.jobs.pop_front() {
                            q.inflight.insert(p.clone());
                            break p;
                        }
                        q = shared.cv.wait(q).unwrap();
                    }
                };
                let r = work(path.clone());
                shared.q.lock().unwrap().inflight.remove(&path);
                if tx.send(r).is_err() {
                    return;
                }
                ctx.request_repaint();
            });
        }
        Self { shared, rx }
    }

    /// Replace the pending queue with `wanted` (in priority order).
    pub fn set_wanted(&self, wanted: impl IntoIterator<Item = PathBuf>) {
        self.requester().set_wanted(wanted)
    }

    /// A cloneable handle that can queue work from another thread.
    pub fn requester(&self) -> Requester {
        Requester(self.shared.clone())
    }
}

#[derive(Clone)]
pub struct Requester(Arc<Shared>);

impl Requester {
    /// Replace the pending queue with `wanted` (in priority order).
    pub fn set_wanted(&self, wanted: impl IntoIterator<Item = PathBuf>) {
        let shared = &self.0;
        let mut q = shared.q.lock().unwrap();
        q.jobs.clear();
        let mut seen = HashSet::new();
        for p in wanted {
            if !q.inflight.contains(&p) && seen.insert(p.clone()) {
                q.jobs.push_back(p);
            }
        }
        let n = q.jobs.len();
        drop(q);
        if n > 0 {
            shared.cv.notify_all();
        }
    }
}

// ---------------------------------------------------------------- full images

pub struct FullResult {
    pub path: PathBuf,
    /// (display image, original size w/h)
    pub result: Result<(ColorImage, [u32; 2]), String>,
}

pub fn full_pool(ctx: egui::Context, max_tex: Arc<AtomicU32>) -> Pool<FullResult> {
    Pool::new(2, ctx, move |path: PathBuf| {
        let result = decode::load_full(&path).map(|img| {
            let orig = [img.width(), img.height()];
            (
                decode::to_color_image(&img, max_tex.load(Ordering::Relaxed)),
                orig,
            )
        });
        FullResult { path, result }
    })
}

// ---------------------------------------------------------------- thumbnails

pub struct ThumbResult {
    pub path: PathBuf,
    pub image: Option<ColorImage>,
}

fn cache_path(path: &std::path::Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = format!(
        "{}|{}|{}|{}",
        path.to_string_lossy(),
        meta.len(),
        mtime,
        decode::THUMB_SIZE
    );
    let h = util::fnv64(key.as_bytes());
    let hex = format!("{h:016x}");
    Some(
        thumb_cache_dir()
            .join(&hex[..2])
            .join(format!("{hex}.webp")),
    )
}

pub fn thumb_cache_dir() -> PathBuf {
    util::app_dir(dirs::cache_dir).join("thumbs")
}

fn thumb_work(path: &std::path::Path) -> Option<ColorImage> {
    let cache = cache_path(path);
    if let Some(c) = &cache {
        if let Ok(bytes) = std::fs::read(c) {
            if let Some(img) = decode::decode_webp_bytes(&bytes) {
                return Some(decode::rgba_to_color_image(&img));
            }
        }
    }
    let img = decode::make_thumb(path, decode::THUMB_SIZE).ok()?;
    if let Some(c) = &cache {
        let data = decode::encode_webp_rgba(&img, 80.0);
        if let Some(dir) = c.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(c, data);
    }
    Some(decode::rgba_to_color_image(&img))
}

pub fn thumb_pool(ctx: egui::Context) -> Pool<ThumbResult> {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let n = n.saturating_sub(1).clamp(2, 8);
    Pool::new(n, ctx, |path: PathBuf| {
        let image = thumb_work(&path);
        ThumbResult { path, image }
    })
}
