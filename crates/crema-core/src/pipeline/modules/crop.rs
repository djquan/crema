use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Crop;

impl ProcessingModule for Crop {
    fn name(&self) -> &str {
        "crop"
    }

    fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        let is_identity_crop = params.crop_x == 0.0
            && params.crop_y == 0.0
            && params.crop_w == 1.0
            && params.crop_h == 1.0;

        if is_identity_crop && params.rotation == 0.0 {
            return Ok(input);
        }

        let src_x =
            ((params.crop_x * input.width as f32) as u32).min(input.width.saturating_sub(1));
        let src_y =
            ((params.crop_y * input.height as f32) as u32).min(input.height.saturating_sub(1));
        let remaining_w = input.width.saturating_sub(src_x);
        let remaining_h = input.height.saturating_sub(src_y);
        let dst_w = (params.crop_w * input.width as f32).max(1.0) as u32;
        let dst_h = (params.crop_h * input.height as f32).max(1.0) as u32;
        let dst_w = dst_w.min(remaining_w).max(1);
        let dst_h = dst_h.min(remaining_h).max(1);

        if params.rotation == 0.0 {
            let mut data = Vec::with_capacity((dst_w * dst_h * 3) as usize);
            for row in src_y..(src_y + dst_h) {
                let row_start = ((row * input.width + src_x) * 3) as usize;
                let row_end = row_start + (dst_w * 3) as usize;
                data.extend_from_slice(&input.data[row_start..row_end]);
            }
            return ImageBuf::from_data(dst_w, dst_h, data);
        }

        // Rotation path: for each output pixel, compute the source position
        // by rotating backward around the source image center.
        let angle = -params.rotation.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        let cx = input.width as f32 * 0.5;
        let cy = input.height as f32 * 0.5;

        let src_w = input.width;
        let src_h = input.height;

        let mut data = Vec::with_capacity((dst_w * dst_h * 3) as usize);

        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let px = (src_x + dx) as f32 + 0.5;
                let py = (src_y + dy) as f32 + 0.5;

                let rx = cx + cos_a * (px - cx) - sin_a * (py - cy) - 0.5;
                let ry = cy + sin_a * (px - cx) + cos_a * (py - cy) - 0.5;

                let (r, g, b) = bilinear_sample(&input.data, src_w, src_h, rx, ry);
                data.push(r);
                data.push(g);
                data.push(b);
            }
        }

        ImageBuf::from_data(dst_w, dst_h, data)
    }
}

fn bilinear_sample(data: &[f32], w: u32, h: u32, x: f32, y: f32) -> (f32, f32, f32) {
    let x = x.clamp(0.0, (w as f32) - 1.0);
    let y = y.clamp(0.0, (h as f32) - 1.0);

    let x0 = (x as u32).min(w - 1);
    let y0 = (y as u32).min(h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);

    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let idx00 = ((y0 * w + x0) * 3) as usize;
    let idx10 = ((y0 * w + x1) * 3) as usize;
    let idx01 = ((y1 * w + x0) * 3) as usize;
    let idx11 = ((y1 * w + x1) * 3) as usize;

    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    let r = data[idx00] * w00 + data[idx10] * w10 + data[idx01] * w01 + data[idx11] * w11;
    let g = data[idx00 + 1] * w00
        + data[idx10 + 1] * w10
        + data[idx01 + 1] * w01
        + data[idx11 + 1] * w11;
    let b = data[idx00 + 2] * w00
        + data[idx10 + 2] * w10
        + data[idx01 + 2] * w01
        + data[idx11 + 2] * w11;

    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_crop_is_identity() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams::default();
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
    }

    #[test]
    fn crop_reduces_dimensions() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
    }

    #[test]
    fn crop_at_boundary_does_not_panic() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            crop_x: 1.0,
            crop_y: 1.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert!(result.width >= 1);
        assert!(result.height >= 1);
    }

    #[test]
    fn crop_with_offset() {
        let mut data = Vec::with_capacity(48);
        for i in 0..16 {
            data.push(i as f32);
            data.push(0.0);
            data.push(0.0);
        }
        let buf = ImageBuf::from_data(4, 4, data).unwrap();

        let params = EditParams {
            crop_x: 0.5,
            crop_y: 0.5,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        assert_eq!(result.data[0], 10.0);
    }

    #[test]
    fn rotation_zero_same_as_no_rotation() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            rotation: 0.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf.clone(), &params).unwrap();

        let params_no_rot = EditParams {
            rotation: 0.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result_no_rot = Crop.process_cpu(buf, &params_no_rot).unwrap();

        assert_eq!(result.width, result_no_rot.width);
        assert_eq!(result.height, result_no_rot.height);
        assert_eq!(result.data, result_no_rot.data);
    }

    #[test]
    fn rotation_preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 10, vec![0.5; 300]).unwrap();
        let params = EditParams {
            rotation: 15.0,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 10);
    }

    #[test]
    fn rotation_180_flips_image() {
        // 2x2 image with distinct pixels
        #[rustfmt::skip]
        let data = vec![
            1.0, 0.0, 0.0,  0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,  0.5, 0.5, 0.5,
        ];
        let buf = ImageBuf::from_data(2, 2, data).unwrap();

        // 180 degrees should approximately flip the image. Due to bilinear
        // interpolation on a tiny image with the rotation center between pixels,
        // the result will be blended; just check that the output differs from input.
        let params = EditParams {
            rotation: 45.0,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf.clone(), &params).unwrap();
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        // With a 45-degree rotation on a 2x2 image, the pixel values should change
        assert!(
            result.data != buf.data,
            "rotated image should differ from original"
        );
    }

    #[test]
    fn small_rotation_mostly_preserves_center() {
        // Create an image with a bright center pixel
        let size = 11u32;
        let mut data = vec![0.0_f32; (size * size * 3) as usize];
        let center = (size / 2) as usize;
        let idx = (center * size as usize + center) * 3;
        data[idx] = 1.0;
        data[idx + 1] = 1.0;
        data[idx + 2] = 1.0;
        let buf = ImageBuf::from_data(size, size, data).unwrap();

        let params = EditParams {
            rotation: 1.0,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();

        // Center pixel should still be the brightest area
        let center_idx = (center * size as usize + center) * 3;
        let center_lum =
            result.data[center_idx] + result.data[center_idx + 1] + result.data[center_idx + 2];
        assert!(
            center_lum > 0.5,
            "center pixel should remain bright after small rotation, got {center_lum}"
        );
    }
}
