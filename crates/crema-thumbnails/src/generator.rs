use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use tracing::debug;

use crema_core::image_buf::ImageBuf;

const THUMBNAIL_LONGEST_EDGE: u32 = 512;

/// Generate a thumbnail from an ImageBuf at reduced resolution.
pub fn generate_thumbnail(buf: &ImageBuf) -> Result<Vec<u8>> {
    let rgba_bytes = buf.to_rgba_u8_srgb();
    let img = RgbaImage::from_raw(buf.width, buf.height, rgba_bytes)
        .context("failed to create image from buffer")?;

    let dynamic = DynamicImage::ImageRgba8(img);
    let thumb = dynamic.resize(
        THUMBNAIL_LONGEST_EDGE,
        THUMBNAIL_LONGEST_EDGE,
        FilterType::Lanczos3,
    );

    let mut jpeg_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut jpeg_bytes);
    thumb
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .context("encode thumbnail as JPEG")?;

    debug!(size = jpeg_bytes.len(), "generated thumbnail");

    Ok(jpeg_bytes)
}

/// Try to load and generate a thumbnail for any supported image file.
pub fn thumbnail_for_file(path: &Path) -> Result<Vec<u8>> {
    let buf = crema_core::raw::load_any(path)?;
    generate_thumbnail(&buf)
}

/// Try to extract the embedded JPEG thumbnail from a RAW file.
/// Falls back to full decode if no embedded thumbnail is available.
pub fn fast_thumbnail(path: &Path) -> Result<Vec<u8>> {
    // For now, just do a full decode. Extracting embedded JPEGs from RAW
    // is format-specific and can be added later as an optimization.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if crema_core::raw::is_raw_extension(ext) {
        debug!(?path, "generating thumbnail via full RAW decode");
    }
    thumbnail_for_file(path)
}
