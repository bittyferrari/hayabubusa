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
