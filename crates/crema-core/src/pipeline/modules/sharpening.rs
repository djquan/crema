use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Sharpening;

impl ProcessingModule for Sharpening {
    fn name(&self) -> &str {
        "sharpening"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.sharpen_amount == 0.0 {
            return Ok(input);
        }

        let w = input.width as usize;
        let h = input.height as usize;
        let amount = params.sharpen_amount / 100.0;
        let radius = params.sharpen_radius;

        let blurred = gaussian_blur_separable(&input.data, w, h, radius);

        for (orig, blur) in input.data.iter_mut().zip(blurred.iter()) {
            *orig = (*orig + amount * (*orig - blur)).max(0.0);
        }

        Ok(input)
    }
}

fn gaussian_kernel(radius: f32) -> Vec<f32> {
    let sigma = radius.max(0.1);
    let kernel_radius = (sigma * 3.0).ceil() as usize;
    let size = kernel_radius * 2 + 1;
    let mut kernel = Vec::with_capacity(size);
    let mut sum = 0.0;

    for i in 0..size {
        let x = i as f32 - kernel_radius as f32;
        let v = (-x * x / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        sum += v;
    }

    for v in &mut kernel {
        *v /= sum;
    }

    kernel
}

fn gaussian_blur_separable(data: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
    let kernel = gaussian_kernel(radius);
    let kr = kernel.len() / 2;

    // Horizontal pass
    let mut temp = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let base = (y * w + x) * 3;
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (ki, &kv) in kernel.iter().enumerate() {
                let sx = (x as isize + ki as isize - kr as isize).clamp(0, w as isize - 1) as usize;
                let si = (y * w + sx) * 3;
                r += data[si] * kv;
                g += data[si + 1] * kv;
                b += data[si + 2] * kv;
            }
            temp[base] = r;
            temp[base + 1] = g;
            temp[base + 2] = b;
        }
    }

    // Vertical pass
    let mut out = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let base = (y * w + x) * 3;
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (ki, &kv) in kernel.iter().enumerate() {
                let sy = (y as isize + ki as isize - kr as isize).clamp(0, h as isize - 1) as usize;
                let si = (sy * w + x) * 3;
                r += temp[si] * kv;
                g += temp[si + 1] * kv;
                b += temp[si + 2] * kv;
            }
            out[base] = r;
            out[base + 1] = g;
            out[base + 2] = b;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_noop() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams::default();
        let result = Sharpening.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn sharpening_increases_local_contrast() {
        // Create an image with a sharp edge: left half 0.2, right half 0.8
        let w = 20;
        let h = 4;
        let mut data = vec![0.0; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 0.2 } else { 0.8 };
                let i = (y * w + x) * 3;
                data[i] = v;
                data[i + 1] = v;
                data[i + 2] = v;
            }
        }
        let buf = ImageBuf::from_data(w as u32, h as u32, data.clone()).unwrap();
        let params = EditParams {
            sharpen_amount: 100.0,
            sharpen_radius: 1.0,
            ..Default::default()
        };
        let result = Sharpening.process_cpu(buf, &params).unwrap();

        // The pixel just right of the edge should be brighter than original (overshoot)
        let edge_right = (1 * w + w / 2) * 3;
        assert!(
            result.data[edge_right] > 0.8,
            "sharpening should overshoot at edge: got {}",
            result.data[edge_right]
        );
    }

    #[test]
    fn uniform_image_unchanged() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            sharpen_amount: 100.0,
            sharpen_radius: 1.5,
            ..Default::default()
        };
        let result = Sharpening.process_cpu(buf, &params).unwrap();
        for &v in &result.data {
            assert!(
                (v - 0.5).abs() < 1e-4,
                "uniform image should stay uniform after sharpening, got {v}"
            );
        }
    }

    #[test]
    fn radius_affects_spread() {
        let w = 40;
        let h = 1;
        let mut data = vec![0.0; w * h * 3];
        // Single bright pixel in center
        let center = w / 2;
        let ci = center * 3;
        data[ci] = 1.0;
        data[ci + 1] = 1.0;
        data[ci + 2] = 1.0;

        let buf1 = ImageBuf::from_data(w as u32, h as u32, data.clone()).unwrap();
        let buf2 = ImageBuf::from_data(w as u32, h as u32, data).unwrap();

        let params_small = EditParams {
            sharpen_amount: 100.0,
            sharpen_radius: 0.5,
            ..Default::default()
        };
        let params_large = EditParams {
            sharpen_amount: 100.0,
            sharpen_radius: 3.0,
            ..Default::default()
        };

        let r1 = Sharpening.process_cpu(buf1, &params_small).unwrap();
        let r2 = Sharpening.process_cpu(buf2, &params_large).unwrap();

        // With larger radius, the blur spreads more, so sharpened center should be different
        assert!(
            (r1.data[ci] - r2.data[ci]).abs() > 0.01,
            "different radii should produce different results"
        );
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            sharpen_amount: 150.0,
            sharpen_radius: 3.0,
            ..Default::default()
        };
        let result = Sharpening.process_cpu(buf, &params).unwrap();
        assert!(result.data.iter().all(|v| v.is_finite() && *v >= 0.0));
    }

    #[test]
    fn preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 5, vec![0.4; 150]).unwrap();
        let params = EditParams {
            sharpen_amount: 50.0,
            ..Default::default()
        };
        let result = Sharpening.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn gaussian_kernel_sums_to_one() {
        for &r in &[0.5, 1.0, 2.0, 3.0] {
            let k = gaussian_kernel(r);
            let sum: f32 = k.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "kernel for radius {r} should sum to 1.0, got {sum}"
            );
        }
    }
}
