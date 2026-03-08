use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct LensCorrection;

impl ProcessingModule for LensCorrection {
    fn name(&self) -> &str {
        "lens_correction"
    }

    fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.vignette_amount == 0.0 && params.distortion == 0.0 {
            return Ok(input);
        }

        let w = input.width;
        let h = input.height;
        let wf = w as f32;
        let hf = h as f32;

        let cx = wf * 0.5;
        let cy = hf * 0.5;
        let r_max = cx.max(cy);
        let k = params.distortion / 100.0;
        let vignette_strength = -params.vignette_amount / 100.0;
        let has_distortion = params.distortion != 0.0;
        let has_vignette = params.vignette_amount != 0.0;

        let mut out = Vec::with_capacity(input.data.len());

        for y in 0..h {
            for x in 0..w {
                let (src_x, src_y) = if has_distortion {
                    let nx = (x as f32 - cx) / r_max;
                    let ny = (y as f32 - cy) / r_max;
                    let r = (nx * nx + ny * ny).sqrt();
                    if r < 1e-8 {
                        (x as f32, y as f32)
                    } else {
                        let r_corrected = r * (1.0 + k * r * r);
                        let scale = r_corrected / r;
                        (cx + nx * scale * r_max, cy + ny * scale * r_max)
                    }
                } else {
                    (x as f32, y as f32)
                };

                let (r, g, b) = bilinear_sample(&input, src_x, src_y);

                let (r, g, b) = if has_vignette {
                    let dx = x as f32 / wf - 0.5;
                    let dy = y as f32 / hf - 0.5;
                    let d2 = dx * dx + dy * dy;
                    let factor = 1.0 + vignette_strength * d2;
                    (r * factor, g * factor, b * factor)
                } else {
                    (r, g, b)
                };

                out.push(r);
                out.push(g);
                out.push(b);
            }
        }

        ImageBuf::from_data(w, h, out)
    }
}

fn bilinear_sample(img: &ImageBuf, x: f32, y: f32) -> (f32, f32, f32) {
    let w = img.width as f32;
    let h = img.height as f32;

    let x = x.clamp(0.0, w - 1.0);
    let y = y.clamp(0.0, h - 1.0);

    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);

    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let stride = img.width;

    let idx00 = ((y0 * stride + x0) * 3) as usize;
    let idx10 = ((y0 * stride + x1) * 3) as usize;
    let idx01 = ((y1 * stride + x0) * 3) as usize;
    let idx11 = ((y1 * stride + x1) * 3) as usize;

    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    let r = img.data[idx00] * w00
        + img.data[idx10] * w10
        + img.data[idx01] * w01
        + img.data[idx11] * w11;
    let g = img.data[idx00 + 1] * w00
        + img.data[idx10 + 1] * w10
        + img.data[idx01 + 1] * w01
        + img.data[idx11 + 1] * w11;
    let b = img.data[idx00 + 2] * w00
        + img.data[idx10 + 2] * w10
        + img.data[idx01 + 2] * w01
        + img.data[idx11 + 2] * w11;

    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_image(w: u32, h: u32, val: f32) -> ImageBuf {
        ImageBuf::from_data(w, h, vec![val; (w * h * 3) as usize]).unwrap()
    }

    #[test]
    fn identity_noop() {
        let buf = uniform_image(10, 10, 0.5);
        let expected = buf.data.clone();
        let params = EditParams::default();
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn vignette_darkens_corners() {
        let buf = uniform_image(10, 10, 0.5);
        let params = EditParams {
            vignette_amount: 50.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        // Corner pixel (0,0) should be darker than center pixel (5,5)
        let corner_idx = 0;
        let center_idx = ((5 * 10 + 5) * 3) as usize;
        assert!(
            result.data[corner_idx] < result.data[center_idx],
            "corner {} should be darker than center {}",
            result.data[corner_idx],
            result.data[center_idx]
        );
    }

    #[test]
    fn vignette_brightens_corners() {
        let buf = uniform_image(10, 10, 0.5);
        let params = EditParams {
            vignette_amount: -50.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        let corner_idx = 0;
        let center_idx = ((5 * 10 + 5) * 3) as usize;
        assert!(
            result.data[corner_idx] > result.data[center_idx],
            "corner {} should be brighter than center {}",
            result.data[corner_idx],
            result.data[center_idx]
        );
    }

    #[test]
    fn vignette_center_unchanged() {
        let buf = uniform_image(11, 11, 0.5);
        let params = EditParams {
            vignette_amount: 80.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        // Center pixel at (5,5) with normalized coords (0.4545, 0.4545)
        // dx = 0.4545 - 0.5 = -0.0455, dy same; d^2 is very small
        let center_idx = ((5 * 11 + 5) * 3) as usize;
        assert!(
            (result.data[center_idx] - 0.5).abs() < 0.01,
            "center pixel should be nearly unchanged, got {}",
            result.data[center_idx]
        );
    }

    #[test]
    fn distortion_preserves_center_pixel() {
        let w = 11u32;
        let h = 11u32;
        let mut data = vec![0.0f32; (w * h * 3) as usize];
        // Put a bright pixel at center
        let ci = ((5 * w + 5) * 3) as usize;
        data[ci] = 1.0;
        data[ci + 1] = 1.0;
        data[ci + 2] = 1.0;
        let buf = ImageBuf::from_data(w, h, data).unwrap();
        let params = EditParams {
            distortion: 50.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        // Center should still be bright (distortion maps center to itself)
        let ri = ((5 * w + 5) * 3) as usize;
        assert!(
            result.data[ri] > 0.5,
            "center should remain bright, got {}",
            result.data[ri]
        );
    }

    #[test]
    fn distortion_barrel_shifts_content() {
        // Barrel distortion (positive k): for each output pixel, source is sampled
        // from further away. A ring at source radius 7 should appear at a smaller
        // radius in the output.
        let w = 21u32;
        let h = 21u32;
        let mut data = vec![0.0f32; (w * h * 3) as usize];
        // Draw a ring of bright pixels at radius ~7
        let cx = 10.0f32;
        let cy = 10.0f32;
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let r = (dx * dx + dy * dy).sqrt();
                if (r - 7.0).abs() < 1.5 {
                    let i = ((y * w + x) * 3) as usize;
                    data[i] = 1.0;
                    data[i + 1] = 1.0;
                    data[i + 2] = 1.0;
                }
            }
        }
        let buf = ImageBuf::from_data(w, h, data).unwrap();
        let params = EditParams {
            distortion: 80.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        // Output pixel at radius ~5 from center along x-axis should pick up ring content
        // because it samples from further away in the source.
        let test_x = 15u32;
        let test_y = 10u32;
        let ti = ((test_y * w + test_x) * 3) as usize;
        assert!(
            result.data[ti] > 0.1,
            "barrel distortion should shift outer content inward in output, got {} at ({}, {})",
            result.data[ti],
            test_x,
            test_y
        );
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = uniform_image(10, 10, 0.5);
        let params = EditParams {
            vignette_amount: 100.0,
            distortion: 100.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        assert!(result.data.iter().all(|v| v.is_finite()));

        let buf2 = uniform_image(10, 10, 0.5);
        let params2 = EditParams {
            vignette_amount: -100.0,
            distortion: -100.0,
            ..Default::default()
        };
        let result2 = LensCorrection.process_cpu(buf2, &params2).unwrap();
        assert!(result2.data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn preserves_dimensions() {
        let buf = uniform_image(20, 15, 0.4);
        let params = EditParams {
            vignette_amount: 30.0,
            distortion: -20.0,
            ..Default::default()
        };
        let result = LensCorrection.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 20);
        assert_eq!(result.height, 15);
    }
}
