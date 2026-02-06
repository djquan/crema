use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Linear f32 RGB image buffer.
///
/// All pixel data is stored as interleaved RGBRGBRGB... in linear light.
/// Values are scene-referred (unbounded above 1.0 before tone mapping).
#[derive(Clone, Debug)]
pub struct ImageBuf {
    pub width: u32,
    pub height: u32,
    /// Flat pixel data: [R, G, B, R, G, B, ...] in linear f32.
    pub data: Vec<f32>,
}

impl ImageBuf {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; (width * height * 3) as usize],
        }
    }

    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> anyhow::Result<Self> {
        let expected = (width * height * 3) as usize;
        anyhow::ensure!(
            data.len() == expected,
            "expected {expected} floats for {width}x{height} RGB, got {}",
            data.len()
        );
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Convert to RGBA f32 with alpha = 1.0 (for GPU upload as Rgba32Float).
    pub fn to_rgba_f32(&self) -> Vec<f32> {
        let pixel_count = (self.width * self.height) as usize;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for pixel in self.data.chunks_exact(3) {
            rgba.push(pixel[0]);
            rgba.push(pixel[1]);
            rgba.push(pixel[2]);
            rgba.push(1.0);
        }
        rgba
    }

    /// Convert to RGBA u8 with sRGB gamma for display/thumbnail use.
    pub fn to_rgba_u8_srgb(&self) -> Vec<u8> {
        let pixel_count = (self.width * self.height) as usize;
        let mut out = Vec::with_capacity(pixel_count * 4);
        for pixel in self.data.chunks_exact(3) {
            out.push(linear_to_srgb_u8(pixel[0]));
            out.push(linear_to_srgb_u8(pixel[1]));
            out.push(linear_to_srgb_u8(pixel[2]));
            out.push(255);
        }
        out
    }

    pub fn pixel_count(&self) -> usize {
        (self.width * self.height) as usize
    }

    /// Downsample so the longest edge fits within `max_edge` pixels.
    /// Uses box averaging for clean downscaling. Returns self if already small enough.
    pub fn downsample(&self, max_edge: u32) -> Self {
        let longest = self.width.max(self.height);
        if longest <= max_edge {
            return self.clone();
        }

        let scale = max_edge as f32 / longest as f32;
        let new_w = (self.width as f32 * scale).round().max(1.0) as u32;
        let new_h = (self.height as f32 * scale).round().max(1.0) as u32;

        let mut data = Vec::with_capacity((new_w * new_h * 3) as usize);

        for dst_y in 0..new_h {
            for dst_x in 0..new_w {
                let src_x0 = (dst_x as f32 / scale) as u32;
                let src_y0 = (dst_y as f32 / scale) as u32;
                let src_x1 = ((dst_x + 1) as f32 / scale).ceil() as u32;
                let src_y1 = ((dst_y + 1) as f32 / scale).ceil() as u32;
                let src_x1 = src_x1.min(self.width);
                let src_y1 = src_y1.min(self.height);

                let mut r = 0.0_f32;
                let mut g = 0.0_f32;
                let mut b = 0.0_f32;
                let mut count = 0u32;

                for sy in src_y0..src_y1 {
                    for sx in src_x0..src_x1 {
                        let idx = ((sy * self.width + sx) * 3) as usize;
                        r += self.data[idx];
                        g += self.data[idx + 1];
                        b += self.data[idx + 2];
                        count += 1;
                    }
                }

                if count > 0 {
                    let inv = 1.0 / count as f32;
                    data.push(r * inv);
                    data.push(g * inv);
                    data.push(b * inv);
                } else {
                    data.push(0.0);
                    data.push(0.0);
                    data.push(0.0);
                }
            }
        }

        Self {
            width: new_w,
            height: new_h,
            data,
        }
    }
}

const SRGB_LUT_SIZE: usize = 4096;

static SRGB_LUT: LazyLock<[u8; SRGB_LUT_SIZE]> = LazyLock::new(|| {
    let mut lut = [0u8; SRGB_LUT_SIZE];
    for (i, entry) in lut.iter_mut().enumerate() {
        let v = i as f32 / (SRGB_LUT_SIZE - 1) as f32;
        let srgb = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        *entry = (srgb * 255.0 + 0.5) as u8;
    }
    lut
});

fn linear_to_srgb_u8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let idx = (v * (SRGB_LUT_SIZE - 1) as f32) as usize;
    SRGB_LUT[idx]
}

/// Non-destructive edit parameters for a photo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditParams {
    /// Exposure compensation in EV stops.
    pub exposure: f32,
    /// White balance color temperature in Kelvin.
    pub wb_temp: f32,
    /// White balance green-magenta tint.
    pub wb_tint: f32,
    /// Crop region, normalized 0..1.
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
}

impl Default for EditParams {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            wb_temp: 5500.0,
            wb_tint: 0.0,
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_buf_dimensions() {
        let buf = ImageBuf::new(100, 50);
        assert_eq!(buf.data.len(), 100 * 50 * 3);
        assert_eq!(buf.pixel_count(), 5000);
    }

    #[test]
    fn from_data_validates_length() {
        let ok = ImageBuf::from_data(2, 2, vec![0.0; 12]);
        assert!(ok.is_ok());

        let bad = ImageBuf::from_data(2, 2, vec![0.0; 10]);
        assert!(bad.is_err());
    }

    #[test]
    fn rgba_f32_conversion() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.25, 0.75]).unwrap();
        let rgba = buf.to_rgba_f32();
        assert_eq!(rgba.len(), 4);
        assert_eq!(rgba, vec![0.5, 0.25, 0.75, 1.0]);
    }

    #[test]
    fn srgb_gamma_black_white() {
        let buf = ImageBuf::from_data(1, 2, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]).unwrap();
        let srgb = buf.to_rgba_u8_srgb();
        assert_eq!(srgb[0..4], [0, 0, 0, 255]);
        assert_eq!(srgb[4..8], [255, 255, 255, 255]);
    }

    #[test]
    fn edit_params_default() {
        let p = EditParams::default();
        assert_eq!(p.exposure, 0.0);
        assert_eq!(p.wb_temp, 5500.0);
        assert_eq!(p.crop_w, 1.0);
        assert_eq!(p.crop_h, 1.0);
    }

    #[test]
    fn rgba_f32_multi_pixel() {
        let buf = ImageBuf::from_data(2, 1, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]).unwrap();
        let rgba = buf.to_rgba_f32();
        assert_eq!(rgba.len(), 8);
        assert_eq!(rgba[3], 1.0); // alpha of first pixel
        assert_eq!(rgba[7], 1.0); // alpha of second pixel
        assert_eq!(rgba[4], 0.4); // R of second pixel
    }

    #[test]
    fn srgb_mid_gray() {
        // Linear 0.214 should map to roughly sRGB 128 (mid-gray)
        let buf = ImageBuf::from_data(1, 1, vec![0.2140, 0.2140, 0.2140]).unwrap();
        let srgb = buf.to_rgba_u8_srgb();
        // sRGB value should be close to 128
        assert!((srgb[0] as i32 - 128).unsigned_abs() <= 2);
    }

    #[test]
    fn srgb_clamps_out_of_range() {
        let buf = ImageBuf::from_data(1, 1, vec![-0.5, 2.0, 0.5]).unwrap();
        let srgb = buf.to_rgba_u8_srgb();
        assert_eq!(srgb[0], 0); // clamped negative to 0
        assert_eq!(srgb[1], 255); // clamped >1.0 to 255
    }

    #[test]
    fn new_buffer_is_zeroed() {
        let buf = ImageBuf::new(10, 10);
        assert!(buf.data.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn from_data_zero_dimensions() {
        let buf = ImageBuf::from_data(0, 0, vec![]);
        assert!(buf.is_ok());
        assert_eq!(buf.unwrap().pixel_count(), 0);
    }

    #[test]
    fn downsample_noop_when_small() {
        let buf = ImageBuf::from_data(100, 50, vec![0.5; 100 * 50 * 3]).unwrap();
        let down = buf.downsample(200);
        assert_eq!(down.width, 100);
        assert_eq!(down.height, 50);
    }

    #[test]
    fn downsample_reduces_dimensions() {
        let buf = ImageBuf::from_data(1000, 500, vec![0.5; 1000 * 500 * 3]).unwrap();
        let down = buf.downsample(100);
        assert!(down.width <= 100);
        assert!(down.height <= 100);
        assert_eq!(down.data.len(), (down.width * down.height * 3) as usize);
    }

    #[test]
    fn downsample_preserves_average_color() {
        let buf = ImageBuf::from_data(400, 200, vec![0.7; 400 * 200 * 3]).unwrap();
        let down = buf.downsample(50);
        for &v in &down.data {
            assert!((v - 0.7).abs() < 1e-4);
        }
    }

    #[test]
    fn edit_params_serialization_roundtrip() {
        let params = EditParams {
            exposure: 1.5,
            wb_temp: 6500.0,
            wb_tint: -10.0,
            crop_x: 0.1,
            crop_y: 0.2,
            crop_w: 0.8,
            crop_h: 0.7,
        };
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: EditParams = serde_json::from_str(&json).unwrap();
        assert!((deserialized.exposure - 1.5).abs() < 1e-6);
        assert!((deserialized.wb_temp - 6500.0).abs() < 1e-6);
        assert!((deserialized.crop_w - 0.8).abs() < 1e-6);
    }
}
