//! Image decoding helpers: full-size load (for the viewer) and fast thumbnails.

use crate::util;
use eframe::egui::ColorImage;
use image::{metadata::Orientation, DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Longest side of cached thumbnails (pixels).
pub const THUMB_SIZE: u32 = 256;

/// Load a full image, applying the EXIF orientation.
pub fn load_full(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Convert to an egui image, downscaling if larger than the GPU texture limit.
pub fn to_color_image(img: &DynamicImage, max_side: u32) -> ColorImage {
    let scaled;
    let img = if img.width() > max_side || img.height() > max_side {
        scaled = img.resize(max_side, max_side, image::imageops::FilterType::Triangle);
        &scaled
    } else {
        img
    };
    let size = [img.width() as usize, img.height() as usize];
    match img {
        DynamicImage::ImageRgb8(rgb) => ColorImage::from_rgb(size, rgb.as_raw()),
        DynamicImage::ImageRgba8(rgba) => ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        other if !other.color().has_alpha() => ColorImage::from_rgb(size, other.to_rgb8().as_raw()),
        other => ColorImage::from_rgba_unmultiplied(size, other.to_rgba8().as_raw()),
    }
}

pub fn rgba_to_color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

fn exif_orientation(path: &Path) -> Orientation {
    let Ok(f) = File::open(path) else {
        return Orientation::NoTransforms;
    };
    let mut r = BufReader::new(f);
    exif::Reader::new()
        .read_from_container(&mut r)
        .ok()
        .and_then(|ex| {
            ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .and_then(|v| Orientation::from_exif(v as u8))
        .unwrap_or(Orientation::NoTransforms)
}

/// Fast JPEG thumbnail using DCT-domain downscaling (decodes at 1/2, 1/4 or 1/8 size).
fn jpeg_thumb(path: &Path, size: u32) -> Result<DynamicImage, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut dec = jpeg_decoder::Decoder::new(BufReader::new(f));
    dec.read_info().map_err(|e| e.to_string())?;
    let s = size.min(u16::MAX as u32) as u16;
    dec.scale(s, s).map_err(|e| e.to_string())?;
    let pixels = dec.decode().map_err(|e| e.to_string())?;
    let info = dec.info().ok_or("no jpeg info")?;
    let (w, h) = (info.width as u32, info.height as u32);
    let img = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels).ok_or("bad buffer")?)
        }
        jpeg_decoder::PixelFormat::L8 => {
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, pixels).ok_or("bad buffer")?)
        }
        jpeg_decoder::PixelFormat::L16 => {
            let v: Vec<u8> = pixels.chunks_exact(2).map(|c| c[0]).collect();
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, v).ok_or("bad buffer")?)
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            let v: Vec<u8> = pixels
                .chunks_exact(4)
                .flat_map(|c| {
                    let k = c[3] as u32;
                    [
                        (c[0] as u32 * k / 255) as u8,
                        (c[1] as u32 * k / 255) as u8,
                        (c[2] as u32 * k / 255) as u8,
                    ]
                })
                .collect();
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, v).ok_or("bad buffer")?)
        }
    };
    let mut img = img;
    img.apply_orientation(exif_orientation(path));
    Ok(img)
}

/// Build a thumbnail (longest side <= size).
pub fn make_thumb(path: &Path, size: u32) -> Result<RgbaImage, String> {
    let img = if util::is_jpeg(path) {
        match jpeg_thumb(path, size) {
            Ok(i) => i,
            Err(_) => load_full(path)?,
        }
    } else {
        load_full(path)?
    };
    let img = if img.width() > size || img.height() > size {
        img.resize(size, size, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Ok(img.to_rgba8())
}

/// Encode RGBA pixels to lossy WebP.
pub fn encode_webp_rgba(img: &RgbaImage, quality: f32) -> Vec<u8> {
    webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height())
        .encode(quality)
        .to_vec()
}

pub fn decode_webp_bytes(data: &[u8]) -> Option<RgbaImage> {
    let img = webp::Decoder::new(data).decode()?;
    let (w, h) = (img.width(), img.height());
    if img.is_alpha() {
        RgbaImage::from_raw(w, h, img.to_vec())
    } else {
        let rgb = image::RgbImage::from_raw(w, h, img.to_vec())?;
        Some(DynamicImage::ImageRgb8(rgb).to_rgba8())
    }
}

/// Image size (after EXIF orientation) read from the file header, without decoding pixels.
pub fn image_dims(path: &Path) -> Option<[u32; 2]> {
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    reader.no_limits();
    let mut dec = reader.into_decoder().ok()?;
    let (w, h) = dec.dimensions();
    let o = dec.orientation().unwrap_or(Orientation::NoTransforms);
    let swap = matches!(
        o,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    (w > 0 && h > 0).then_some(if swap { [h, w] } else { [w, h] })
}

/// Load an image for display `target_w` pixels wide (after EXIF orientation).
/// JPEGs are decoded at 1/2, 1/4 or 1/8 size in the DCT domain when that is still
/// at least `target_w` wide, which is much faster than decoding at full size.
/// Returns the image (not yet resized to `target_w`) and the original size.
pub fn load_for_width(path: &Path, target_w: u32) -> Result<(DynamicImage, [u32; 2]), String> {
    if util::is_jpeg(path) {
        if let Some(r) = jpeg_for_width(path, target_w) {
            return Ok(r);
        }
    }
    let img = load_full(path)?;
    let orig = [img.width(), img.height()];
    Ok((img, orig))
}

/// `None` = no DCT scaling possible (or any error): use the normal decoder.
fn jpeg_for_width(path: &Path, target_w: u32) -> Option<(DynamicImage, [u32; 2])> {
    let f = File::open(path).ok()?;
    let mut dec = jpeg_decoder::Decoder::new(BufReader::new(f));
    dec.read_info().ok()?;
    let info = dec.info()?;
    let (w, h) = (info.width as u32, info.height as u32);
    let orientation = exif_orientation(path);
    let swap = matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    let orig = if swap { [h, w] } else { [w, h] };
    // Displayed width is the stored height for rotated images.
    let stored_w = if swap { h } else { w };
    if target_w.saturating_mul(2) > stored_w {
        return None; // not even 1/2 scale is possible
    }
    // `scale` picks the smallest size that reaches the request in *either* dimension,
    // so ask for an unreachable height to make the width decide.
    let t = target_w.clamp(1, u16::MAX as u32) as u16;
    let (rw, rh) = if swap { (u16::MAX, t) } else { (t, u16::MAX) };
    dec.scale(rw, rh).ok()?;
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let (w, h) = (info.width as u32, info.height as u32);
    let img = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels)?)
        }
        jpeg_decoder::PixelFormat::L8 => {
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, pixels)?)
        }
        _ => return None, // CMYK / 16-bit: leave to the normal decoder
    };
    let mut img = img;
    img.apply_orientation(orientation);
    if img.width() < target_w {
        return None; // should not happen; never return something too small
    }
    Some((img, orig))
}
