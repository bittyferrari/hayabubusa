//! File operations: rotate, convert, resize.

use crate::decode;
use crate::util;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ImageFormat};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutFormat {
    Keep,
    Jpg,
    Png,
    Webp,
}

impl OutFormat {
    pub fn label(self) -> &'static str {
        match self {
            OutFormat::Keep => crate::i18n::tr().keep_format,
            OutFormat::Jpg => "JPG",
            OutFormat::Png => "PNG",
            OutFormat::Webp => "WEBP",
        }
    }
    fn ext(self, src: &Path) -> String {
        match self {
            OutFormat::Keep => {
                let e = util::ext_lower(src);
                if e.is_empty() {
                    "png".into()
                } else {
                    e
                }
            }
            OutFormat::Jpg => "jpg".into(),
            OutFormat::Png => "png".into(),
            OutFormat::Webp => "webp".into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ResizeMode {
    None,
    Percent(f32),
    LongSide(u32),
    /// Fit inside box, keep ratio
    Fit(u32, u32),
    /// Exact size (may distort)
    Exact(u32, u32),
}

#[derive(Clone, Debug)]
pub struct ConvertOptions {
    pub format: OutFormat,
    pub quality: u8,
    pub resize: ResizeMode,
    pub no_upscale: bool,
    pub out_dir: Option<PathBuf>, // None = same folder as source
    pub overwrite: bool,          // write over source file when name matches
    pub rotate: u8,               // extra clockwise quarter turns
}

pub fn rotate(img: DynamicImage, quarter_turns_cw: u8) -> DynamicImage {
    match quarter_turns_cw % 4 {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    }
}

pub fn resize(img: DynamicImage, mode: ResizeMode, no_upscale: bool) -> DynamicImage {
    use image::imageops::FilterType::Lanczos3;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (tw, th) = match mode {
        ResizeMode::None => return img,
        ResizeMode::Percent(p) => (w * p / 100.0, h * p / 100.0),
        ResizeMode::LongSide(l) => {
            let s = l as f32 / w.max(h);
            (w * s, h * s)
        }
        ResizeMode::Fit(bw, bh) => {
            let s = (bw as f32 / w).min(bh as f32 / h);
            (w * s, h * s)
        }
        ResizeMode::Exact(bw, bh) => (bw as f32, bh as f32),
    };
    let (tw, th) = (tw.round().max(1.0) as u32, th.round().max(1.0) as u32);
    if no_upscale && (tw > img.width() || th > img.height()) {
        return img;
    }
    if tw == img.width() && th == img.height() {
        return img;
    }
    img.resize_exact(tw, th, Lanczos3)
}

/// Save, writing to a temp file first then renaming (safe for in-place overwrite).
pub fn save(img: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let ext = util::ext_lower(path);
    let data: Vec<u8> = match ext.as_str() {
        "jpg" | "jpeg" | "jfif" => {
            let rgb = img.to_rgb8();
            let mut buf = Vec::new();
            JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100))
                .encode_image(&rgb)
                .map_err(|e| e.to_string())?;
            buf
        }
        "webp" => {
            let (w, h) = (img.width(), img.height());
            let mem = if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                let enc = webp::Encoder::from_rgba(rgba.as_raw(), w, h);
                if quality >= 100 {
                    enc.encode_lossless()
                } else {
                    enc.encode(quality as f32)
                }
            } else {
                let rgb = img.to_rgb8();
                let enc = webp::Encoder::from_rgb(rgb.as_raw(), w, h);
                if quality >= 100 {
                    enc.encode_lossless()
                } else {
                    enc.encode(quality as f32)
                }
            };
            mem.to_vec()
        }
        _ => {
            let fmt = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Png);
            let mut buf = std::io::Cursor::new(Vec::new());
            // Formats without alpha / 16-bit support get a safe 8-bit image.
            let tmp;
            let img = match fmt {
                ImageFormat::Bmp | ImageFormat::Tga | ImageFormat::Ico | ImageFormat::Gif => {
                    tmp = DynamicImage::ImageRgba8(img.to_rgba8());
                    &tmp
                }
                _ => img,
            };
            img.write_to(&mut buf, fmt).map_err(|e| e.to_string())?;
            buf.into_inner()
        }
    };
    let tmp = path.with_extension(format!("{ext}.hayabubusa-tmp"));
    std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Rotate a file in place (keeps its format; JPEG saved at quality 95).
pub fn rotate_file(path: &Path, quarter_turns_cw: u8) -> Result<(), String> {
    let img = decode::load_full(path)?;
    let img = rotate(img, quarter_turns_cw);
    save(&img, path, 95)
}

/// Convert one file; returns the written path.
pub fn convert_file(src: &Path, opt: &ConvertOptions) -> Result<PathBuf, String> {
    let img = decode::load_full(src)?;
    let img = rotate(img, opt.rotate);
    let img = resize(img, opt.resize, opt.no_upscale);
    let dir = opt
        .out_dir
        .clone()
        .unwrap_or_else(|| src.parent().map(Path::to_path_buf).unwrap_or_default());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or("image".into());
    let ext = opt.format.ext(src);
    let direct = dir.join(format!("{stem}.{ext}"));
    let dst = if opt.overwrite || !direct.exists() {
        direct
    } else {
        util::unique_path(&dir, &stem, &ext)
    };
    save(&img, &dst, opt.quality)?;
    Ok(dst)
}

/// Progress of a background batch job.
pub struct Batch {
    pub title: String,
    pub total: usize,
    pub done: Arc<AtomicUsize>,
    pub finished: Arc<AtomicBool>,
    pub errors: Arc<Mutex<Vec<String>>>,
    pub outputs: Arc<Mutex<Vec<PathBuf>>>,
}

impl Batch {
    /// Run `f` over `files` on several threads.
    pub fn spawn<F>(title: String, files: Vec<PathBuf>, ctx: eframe::egui::Context, f: F) -> Self
    where
        F: Fn(&Path) -> Result<Option<PathBuf>, String> + Send + Sync + 'static,
    {
        let total = files.len();
        let done = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let (d, fin, er, out) = (
            done.clone(),
            finished.clone(),
            errors.clone(),
            outputs.clone(),
        );
        std::thread::spawn(move || {
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8);
            let next = AtomicUsize::new(0);
            let f = &f;
            std::thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some(p) = files.get(i) else { break };
                        match f(p) {
                            Ok(Some(o)) => out.lock().unwrap().push(o),
                            Ok(None) => {}
                            Err(e) => er.lock().unwrap().push(format!(
                                "{}: {e}",
                                p.file_name().unwrap_or_default().to_string_lossy()
                            )),
                        }
                        d.fetch_add(1, Ordering::SeqCst);
                        ctx.request_repaint();
                    });
                }
            });
            fin.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
        Self {
            title,
            total,
            done,
            finished,
            errors,
            outputs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> PathBuf {
        let mut img = image::RgbImage::new(400, 200);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        let p = dir.join("sample.jpg");
        save(&DynamicImage::ImageRgb8(img), &p, 90).unwrap();
        p
    }

    #[test]
    fn convert_rotate_resize() {
        let dir = std::env::temp_dir().join("hayabubusa_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = sample(&dir);

        for (fmt, ext) in [
            (OutFormat::Png, "png"),
            (OutFormat::Webp, "webp"),
            (OutFormat::Jpg, "jpg"),
        ] {
            let opt = ConvertOptions {
                format: fmt,
                quality: 85,
                resize: ResizeMode::LongSide(100),
                no_upscale: true,
                out_dir: Some(dir.join("out")),
                overwrite: false,
                rotate: 1,
            };
            let out = convert_file(&src, &opt).unwrap();
            assert_eq!(util::ext_lower(&out), ext);
            let img = decode::load_full(&out).unwrap();
            // 400x200 rotated -> 200x400, long side 100 -> 50x100
            assert_eq!((img.width(), img.height()), (50, 100), "{ext}");
        }
        // unique naming
        let opt = ConvertOptions {
            format: OutFormat::Jpg,
            quality: 80,
            resize: ResizeMode::None,
            no_upscale: true,
            out_dir: Some(dir.join("out")),
            overwrite: false,
            rotate: 0,
        };
        let out = convert_file(&src, &opt).unwrap();
        assert!(out
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("sample_1.jpg"));

        // rotate in place
        rotate_file(&src, 3).unwrap();
        let img = decode::load_full(&src).unwrap();
        assert_eq!((img.width(), img.height()), (200, 400));

        // thumbnail + webp cache roundtrip
        let t = decode::make_thumb(&src, 64).unwrap();
        assert!(t.width() <= 64 && t.height() <= 64 && t.height() == 64);
        let bytes = decode::encode_webp_rgba(&t, 80.0);
        let back = decode::decode_webp_bytes(&bytes).unwrap();
        assert_eq!(back.dimensions(), t.dimensions());
        println!("thumb webp bytes: {}", bytes.len());
    }
}
