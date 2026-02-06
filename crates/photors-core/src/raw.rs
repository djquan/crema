use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use tracing::{debug, info};

use crate::image_buf::ImageBuf;

pub const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "rw2", "orf", "pef", "dng",
    "3fr", "ari", "bay", "cap", "dcr", "erf", "fff", "iiq", "k25", "kdc", "mef", "mos", "mrw",
    "raw", "rwl", "srw", "x3f",
];

pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tiff", "tif"];

pub fn is_supported_extension(ext: &str) -> bool {
    let lower = ext.to_ascii_lowercase();
    RAW_EXTENSIONS.contains(&lower.as_str()) || IMAGE_EXTENSIONS.contains(&lower.as_str())
}

pub fn is_raw_extension(ext: &str) -> bool {
    RAW_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Decode a RAW file to a linear f32 RGB ImageBuf.
///
/// rawler's default develop pipeline applies demosaic, white balance,
/// color calibration, and sRGB gamma. We undo the sRGB gamma to get
/// linear light for our own pipeline.
pub fn decode_raw(path: &Path) -> Result<ImageBuf> {
    info!(?path, "decoding RAW file");
    let t0 = std::time::Instant::now();

    let raw_image = rawler::decode_file(path)
        .with_context(|| format!("failed to decode RAW: {}", path.display()))?;
    debug!(elapsed_ms = t0.elapsed().as_millis(), "rawler decode_file");

    let t1 = std::time::Instant::now();
    let develop = rawler::imgop::develop::RawDevelop::default();
    let intermediate = develop
        .develop_intermediate(&raw_image)
        .with_context(|| format!("development failed: {}", path.display()))?;
    debug!(
        elapsed_ms = t1.elapsed().as_millis(),
        "rawler develop_intermediate"
    );

    match intermediate {
        rawler::imgop::develop::Intermediate::ThreeColor(rgb) => {
            let t2 = std::time::Instant::now();
            let width = rgb.width as u32;
            let height = rgb.height as u32;
            let mut data = Vec::with_capacity(rgb.data.len() * 3);
            for pixel in &rgb.data {
                data.push(srgb_f32_to_linear(pixel[0]));
                data.push(srgb_f32_to_linear(pixel[1]));
                data.push(srgb_f32_to_linear(pixel[2]));
            }
            debug!(
                elapsed_ms = t2.elapsed().as_millis(),
                "srgb_to_linear conversion"
            );
            debug!(elapsed_ms = t0.elapsed().as_millis(), "total decode_raw");
            ImageBuf::from_data(width, height, data)
        }
        _ => bail!("unexpected intermediate format (expected RGB)"),
    }
}

/// Load a standard image (JPEG, PNG, TIFF) to a linear f32 RGB ImageBuf.
pub fn load_image(path: &Path) -> Result<ImageBuf> {
    load_image_scaled(path, None)
}

/// Load a standard image, optionally resizing so the longest edge
/// fits within `max_edge` pixels. Resizing happens in u8/sRGB space
/// (before the linear conversion) so we avoid converting millions of
/// pixels we'd immediately throw away.
pub fn load_image_scaled(path: &Path, max_edge: Option<u32>) -> Result<ImageBuf> {
    info!(?path, "loading image file");
    let t0 = std::time::Instant::now();

    let img =
        image::open(path).with_context(|| format!("failed to open image: {}", path.display()))?;
    debug!(
        elapsed_ms = t0.elapsed().as_millis(),
        width = img.width(),
        height = img.height(),
        "image decode"
    );

    let img = match max_edge {
        Some(max) if img.width().max(img.height()) > max => {
            let t1 = std::time::Instant::now();
            let resized = img.resize(max, max, image::imageops::FilterType::Triangle);
            debug!(
                elapsed_ms = t1.elapsed().as_millis(),
                width = resized.width(),
                height = resized.height(),
                "u8 resize"
            );
            resized.into_rgb8()
        }
        _ => img.into_rgb8(),
    };

    let width = img.width();
    let height = img.height();
    let pixel_count = (width * height) as usize;
    let lut = &*SRGB_U8_TO_LINEAR;
    let mut data = Vec::with_capacity(pixel_count * 3);

    for pixel in img.pixels() {
        data.push(lut[pixel.0[0] as usize]);
        data.push(lut[pixel.0[1] as usize]);
        data.push(lut[pixel.0[2] as usize]);
    }
    debug!(elapsed_ms = t0.elapsed().as_millis(), "total load_image");

    ImageBuf::from_data(width, height, data)
}

/// Load any supported image file (RAW or standard).
pub fn load_any(path: &Path) -> Result<ImageBuf> {
    load_any_scaled(path, None)
}

/// Load any supported image file, optionally limiting the longest edge
/// to `max_edge` pixels. For standard images this resizes in u8 space
/// before converting to linear, avoiding work on pixels we'd discard.
/// For RAW files, we must decode at full resolution (rawler doesn't
/// support partial decode), then downsample in linear space.
pub fn load_any_scaled(path: &Path, max_edge: Option<u32>) -> Result<ImageBuf> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if is_raw_extension(ext) {
        let buf = decode_raw(path)?;
        match max_edge {
            Some(max) if buf.width.max(buf.height) > max => Ok(buf.downsample(max)),
            _ => Ok(buf),
        }
    } else {
        load_image_scaled(path, max_edge)
    }
}

/// Perfect 256-entry LUT for u8 sRGB -> linear f32 (used by load_image).
static SRGB_U8_TO_LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut lut = [0.0f32; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let v = i as f32 / 255.0;
        *entry = srgb_to_linear_exact(v);
    }
    lut
});

const SRGB_F32_LUT_SIZE: usize = 4096;

/// 4096-entry LUT for f32 sRGB -> linear f32 (used by decode_raw).
static SRGB_F32_TO_LINEAR: LazyLock<[f32; SRGB_F32_LUT_SIZE]> = LazyLock::new(|| {
    let mut lut = [0.0f32; SRGB_F32_LUT_SIZE];
    for (i, entry) in lut.iter_mut().enumerate() {
        let v = i as f32 / (SRGB_F32_LUT_SIZE - 1) as f32;
        *entry = srgb_to_linear_exact(v);
    }
    lut
});

fn srgb_f32_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let idx = (v * (SRGB_F32_LUT_SIZE - 1) as f32) as usize;
    SRGB_F32_TO_LINEAR[idx]
}

fn srgb_to_linear_exact(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_detection() {
        assert!(is_supported_extension("CR2"));
        assert!(is_supported_extension("jpg"));
        assert!(is_supported_extension("PNG"));
        assert!(!is_supported_extension("mp4"));
        assert!(is_raw_extension("nef"));
        assert!(!is_raw_extension("jpeg"));
    }

    #[test]
    fn srgb_linear_roundtrip() {
        let linear = srgb_f32_to_linear(0.5);
        assert!((linear - 0.214).abs() < 0.01);

        let black = srgb_f32_to_linear(0.0);
        assert_eq!(black, 0.0);

        let white = srgb_f32_to_linear(1.0);
        assert!((white - 1.0).abs() < 0.001);
    }

    #[test]
    fn u8_lut_matches_exact() {
        let lut = &*SRGB_U8_TO_LINEAR;
        assert_eq!(lut[0], 0.0);
        assert!((lut[255] - 1.0).abs() < 0.001);
        assert!((lut[128] - srgb_to_linear_exact(128.0 / 255.0)).abs() < 1e-6);
    }
}
