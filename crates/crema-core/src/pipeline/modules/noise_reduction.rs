use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct NoiseReduction;

impl ProcessingModule for NoiseReduction {
    fn name(&self) -> &str {
        "noise_reduction"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.nr_luminance == 0.0 && params.nr_color == 0.0 {
            return Ok(input);
        }

        let w = input.width as usize;
        let h = input.height as usize;
        if w == 0 || h == 0 {
            return Ok(input);
        }

        if params.nr_luminance > 0.0 {
            bilateral_filter(&mut input.data, w, h, params.nr_luminance);
        }

        if params.nr_color > 0.0 {
            chroma_blur(&mut input.data, w, h, params.nr_color);
        }

        Ok(input)
    }
}

/// Bilateral filter that uses luminance for range weighting.
///
/// For each pixel, neighbors within the spatial radius contribute
/// based on both spatial proximity (Gaussian) and luminance similarity
/// (Gaussian on luma difference). This smooths noise while preserving edges.
fn bilateral_filter(data: &mut [f32], w: usize, h: usize, strength: f32) {
    let spatial_sigma = strength / 10.0;
    let range_sigma: f32 = 0.1;
    let radius = (spatial_sigma * 2.0).ceil().min(5.0) as usize;

    if radius == 0 {
        return;
    }

    let inv_spatial_2sq = -1.0 / (2.0 * spatial_sigma * spatial_sigma);
    let inv_range_2sq = -1.0 / (2.0 * range_sigma * range_sigma);

    let src = data.to_vec();
    let pixel_count = w * h;

    for i in 0..pixel_count {
        let px = i % w;
        let py = i / w;
        let idx = i * 3;

        let luma_center = 0.2126 * src[idx] + 0.7152 * src[idx + 1] + 0.0722 * src[idx + 2];

        let mut sum_r = 0.0_f32;
        let mut sum_g = 0.0_f32;
        let mut sum_b = 0.0_f32;
        let mut sum_w = 0.0_f32;

        let y_start = py.saturating_sub(radius);
        let y_end = (py + radius + 1).min(h);
        let x_start = px.saturating_sub(radius);
        let x_end = (px + radius + 1).min(w);

        for ny in y_start..y_end {
            for nx in x_start..x_end {
                let ni = (ny * w + nx) * 3;

                let dx = px as f32 - nx as f32;
                let dy = py as f32 - ny as f32;
                let spatial_dist_sq = dx * dx + dy * dy;

                let luma_n = 0.2126 * src[ni] + 0.7152 * src[ni + 1] + 0.0722 * src[ni + 2];
                let luma_diff = luma_center - luma_n;

                let weight = (spatial_dist_sq * inv_spatial_2sq
                    + luma_diff * luma_diff * inv_range_2sq)
                    .exp();

                sum_r += src[ni] * weight;
                sum_g += src[ni + 1] * weight;
                sum_b += src[ni + 2] * weight;
                sum_w += weight;
            }
        }

        if sum_w > 0.0 {
            let inv = 1.0 / sum_w;
            data[idx] = sum_r * inv;
            data[idx + 1] = sum_g * inv;
            data[idx + 2] = sum_b * inv;
        }
    }
}

/// Blur chroma (Cb, Cr) while preserving luminance.
///
/// Converts to Y/Cb/Cr, applies separable Gaussian blur on Cb and Cr,
/// then reconstructs RGB.
fn chroma_blur(data: &mut [f32], w: usize, h: usize, strength: f32) {
    let sigma = strength / 10.0;
    let radius = (sigma * 2.0).ceil().min(7.0) as usize;

    if radius == 0 {
        return;
    }

    let pixel_count = w * h;

    let mut y_ch = Vec::with_capacity(pixel_count);
    let mut cb_ch = Vec::with_capacity(pixel_count);
    let mut cr_ch = Vec::with_capacity(pixel_count);

    for i in 0..pixel_count {
        let idx = i * 3;
        let r = data[idx];
        let g = data[idx + 1];
        let b = data[idx + 2];
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        y_ch.push(y);
        cb_ch.push(b - y);
        cr_ch.push(r - y);
    }

    let kernel = gaussian_kernel(sigma, radius);
    cb_ch = separable_blur_1ch(&cb_ch, w, h, &kernel);
    cr_ch = separable_blur_1ch(&cr_ch, w, h, &kernel);

    // Reconstruct: R = Y + Cr, B = Y + Cb, G = Y - (0.2126/0.7152)*Cr - (0.0722/0.7152)*Cb
    let cr_coeff = 0.2126 / 0.7152;
    let cb_coeff = 0.0722 / 0.7152;
    for i in 0..pixel_count {
        let idx = i * 3;
        let y = y_ch[i];
        let cb = cb_ch[i];
        let cr = cr_ch[i];
        data[idx] = y + cr;
        data[idx + 1] = y - cr_coeff * cr - cb_coeff * cb;
        data[idx + 2] = y + cb;
    }
}

fn gaussian_kernel(sigma: f32, radius: usize) -> Vec<f32> {
    let sigma = sigma.max(0.1);
    let size = radius * 2 + 1;
    let mut kernel = Vec::with_capacity(size);
    let mut sum = 0.0;

    for i in 0..size {
        let x = i as f32 - radius as f32;
        let v = (-x * x / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        sum += v;
    }

    for v in &mut kernel {
        *v /= sum;
    }

    kernel
}

fn separable_blur_1ch(data: &[f32], w: usize, h: usize, kernel: &[f32]) -> Vec<f32> {
    let kr = kernel.len() / 2;

    // Horizontal pass
    let mut temp = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sx = (x as isize + ki as isize - kr as isize).clamp(0, w as isize - 1) as usize;
                acc += data[y * w + sx] * kv;
            }
            temp[y * w + x] = acc;
        }
    }

    // Vertical pass
    let mut out = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sy = (y as isize + ki as isize - kr as isize).clamp(0, h as isize - 1) as usize;
                acc += temp[sy * w + x] * kv;
            }
            out[y * w + x] = acc;
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
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn luminance_reduces_noise() {
        let w = 20_u32;
        let h = 20_u32;
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        let mut rng_state: u32 = 12345;
        for _ in 0..(w * h) {
            // Simple PRNG for deterministic noise
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let noise = (rng_state as f32 / u32::MAX as f32) * 0.2 - 0.1;
            let v = (0.5 + noise).clamp(0.0, 1.0);
            data.push(v);
            data.push(v);
            data.push(v);
        }

        let variance_before: f64 = data.iter().map(|&v| ((v - 0.5) as f64).powi(2)).sum();

        let buf = ImageBuf::from_data(w, h, data).unwrap();
        let params = EditParams {
            nr_luminance: 50.0,
            ..Default::default()
        };
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();

        let variance_after: f64 = result
            .data
            .iter()
            .map(|&v| ((v - 0.5) as f64).powi(2))
            .sum();
        assert!(
            variance_after < variance_before * 0.5,
            "variance should decrease: before={variance_before}, after={variance_after}"
        );
    }

    #[test]
    fn color_reduces_chroma_noise() {
        let w = 20_u32;
        let h = 20_u32;
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        let mut rng_state: u32 = 67890;
        for _ in 0..(w * h) {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let noise_r = (rng_state as f32 / u32::MAX as f32) * 0.2 - 0.1;
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let noise_b = (rng_state as f32 / u32::MAX as f32) * 0.2 - 0.1;
            data.push((0.5 + noise_r).clamp(0.0, 1.0));
            data.push(0.5);
            data.push((0.5 + noise_b).clamp(0.0, 1.0));
        }

        // Measure chroma variance before
        let chroma_var_before: f64 = data
            .chunks_exact(3)
            .map(|p| {
                let y = 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
                let cb = p[2] as f64 - y;
                let cr = p[0] as f64 - y;
                cb * cb + cr * cr
            })
            .sum();

        let buf = ImageBuf::from_data(w, h, data).unwrap();
        let params = EditParams {
            nr_color: 50.0,
            ..Default::default()
        };
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();

        let chroma_var_after: f64 = result
            .data
            .chunks_exact(3)
            .map(|p| {
                let y = 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
                let cb = p[2] as f64 - y;
                let cr = p[0] as f64 - y;
                cb * cb + cr * cr
            })
            .sum();

        assert!(
            chroma_var_after < chroma_var_before * 0.5,
            "chroma variance should decrease: before={chroma_var_before}, after={chroma_var_after}"
        );
    }

    #[test]
    fn uniform_image_unchanged() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            nr_luminance: 50.0,
            nr_color: 50.0,
            ..Default::default()
        };
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();
        for &v in &result.data {
            assert!(
                (v - 0.5).abs() < 1e-4,
                "uniform image should stay uniform, got {v}"
            );
        }
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            nr_luminance: 100.0,
            nr_color: 100.0,
            ..Default::default()
        };
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();
        assert!(result.data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 5, vec![0.4; 150]).unwrap();
        let params = EditParams {
            nr_luminance: 30.0,
            nr_color: 20.0,
            ..Default::default()
        };
        let result = NoiseReduction.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
        assert_eq!(result.data.len(), 150);
    }
}
