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

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageReader;
    use std::io::Cursor;

    fn make_solid_image(w: u32, h: u32, r: f32, g: f32, b: f32) -> ImageBuf {
        let pixel_count = (w * h) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);
        for _ in 0..pixel_count {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        ImageBuf::from_data(w, h, data).unwrap()
    }

    fn decode_jpeg_dimensions(jpeg_bytes: &[u8]) -> (u32, u32) {
        let reader = ImageReader::new(Cursor::new(jpeg_bytes))
            .with_guessed_format()
            .unwrap();
        let img = reader.decode().unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn thumbnail_produces_valid_jpeg() {
        let buf = make_solid_image(1024, 768, 0.5, 0.3, 0.1);
        let jpeg = generate_thumbnail(&buf).unwrap();
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8]);
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert!(w > 0 && h > 0);
    }

    #[test]
    fn thumbnail_longest_edge_is_512_for_landscape() {
        let buf = make_solid_image(2000, 1000, 0.5, 0.5, 0.5);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(w, 512);
        assert!(h <= 512);
        assert!((h as f64 - 256.0).abs() <= 1.0);
    }

    #[test]
    fn thumbnail_longest_edge_is_512_for_portrait() {
        let buf = make_solid_image(800, 1600, 0.2, 0.4, 0.6);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(h, 512);
        assert!(w <= 512);
        assert!((w as f64 - 256.0).abs() <= 1.0);
    }

    #[test]
    fn thumbnail_preserves_aspect_ratio_square() {
        let buf = make_solid_image(2048, 2048, 0.5, 0.5, 0.5);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(w, 512);
        assert_eq!(h, 512);
    }

    #[test]
    fn thumbnail_preserves_aspect_ratio_ultrawide() {
        let buf = make_solid_image(4000, 500, 0.1, 0.1, 0.1);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(w, 512);
        assert!((h as f64 - 64.0).abs() <= 1.0);
    }

    #[test]
    fn thumbnail_preserves_aspect_ratio_ultratall() {
        let buf = make_solid_image(300, 6000, 0.9, 0.9, 0.9);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(h, 512);
        assert!((w as f64 - 26.0).abs() <= 1.0);
    }

    #[test]
    fn thumbnail_handles_1x1_image() {
        let buf = make_solid_image(1, 1, 1.0, 0.0, 0.0);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert!(w >= 1 && h >= 1);
        assert!(w <= 512 && h <= 512);
    }

    #[test]
    fn thumbnail_small_image_is_upscaled_to_512() {
        // DynamicImage::resize upscales when image is smaller than target
        let buf = make_solid_image(100, 50, 0.5, 0.5, 0.5);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert_eq!(w, 512);
        assert_eq!(h, 256);
    }

    #[test]
    fn thumbnail_handles_hdr_values() {
        let buf = make_solid_image(800, 600, 5.0, 10.0, 0.0);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert!(w > 0 && h > 0);
        assert!(w <= 512 && h <= 512);
    }

    #[test]
    fn thumbnail_handles_negative_values() {
        let buf = make_solid_image(640, 480, -0.1, 0.5, 1.5);
        let jpeg = generate_thumbnail(&buf).unwrap();
        let (w, h) = decode_jpeg_dimensions(&jpeg);
        assert!(w > 0 && h > 0);
    }

    #[test]
    fn thumbnail_output_size_reasonable() {
        let buf = make_solid_image(2000, 2000, 0.5, 0.5, 0.5);
        let jpeg = generate_thumbnail(&buf).unwrap();
        assert!(jpeg.len() > 100);
        assert!(jpeg.len() < 1_000_000);
    }

    #[test]
    fn thumbnail_varied_content_produces_larger_jpeg() {
        let w = 1024u32;
        let h = 1024u32;
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                data.push(x as f32 / w as f32);
                data.push(y as f32 / h as f32);
                data.push(0.5);
            }
        }
        let gradient = ImageBuf::from_data(w, h, data).unwrap();
        let gradient_jpeg = generate_thumbnail(&gradient).unwrap();

        let solid = make_solid_image(1024, 1024, 0.5, 0.5, 0.5);
        let solid_jpeg = generate_thumbnail(&solid).unwrap();

        assert!(gradient_jpeg.len() > solid_jpeg.len());
    }
}
