use crate::image_buf::{EditParams, ImageBuf};

/// Analyze a linear f32 preview and produce edit suggestions.
///
/// Histogram analysis is performed in **perceptual space** (gamma 2.2)
/// where human vision is roughly uniform, making thresholds intuitive.
///
/// Strategy (inspired by pre-ML Lightroom Auto):
///   - Highlights and shadows do the heavy lifting, not exposure.
///   - Pull down bright areas (sqrt ramp for perceptual uniformity).
///   - Lift dark areas to reveal shadow detail (sqrt ramp).
///   - Crush blacks to maintain depth after shadow lift.
///   - Small exposure nudge (45% strength, dead zone ±0.07 EV).
///   - Leave WB at camera's As Shot (gray-world is unreliable).
///   - Modest contrast and vibrance boost.
pub fn auto_enhance(buf: &ImageBuf) -> EditParams {
    let pixel_count = buf.pixel_count();
    if pixel_count == 0 {
        return EditParams::default();
    }

    let mut luminances = Vec::with_capacity(pixel_count);
    let mut sum_sat = 0.0_f64;
    let mut sat_count = 0_u64;

    for pixel in buf.data.chunks_exact(3) {
        let r = pixel[0] as f64;
        let g = pixel[1] as f64;
        let b = pixel[2] as f64;

        let y_linear = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        luminances.push(y_linear.max(0.0).powf(1.0 / 2.2));

        let max_ch = r.max(g).max(b);
        if max_ch >= 0.02 {
            let min_ch = r.min(g).min(b);
            sum_sat += (max_ch - min_ch) / (max_ch + 1e-6);
            sat_count += 1;
        }
    }

    luminances.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let percentile = |p: f64| -> f64 {
        let idx = ((p / 100.0) * (luminances.len() - 1) as f64) as usize;
        luminances[idx.min(luminances.len() - 1)]
    };
    let p5 = percentile(5.0);
    let p10 = percentile(10.0);
    let p50 = percentile(50.0);
    let p95 = percentile(95.0);

    // All thresholds below are in perceptual space [0, 1].

    // ── Exposure ──
    // Target: perceptual mid-gray (~0.46). Third-strength correction.
    let target_mid = 0.46; // 0.18 linear in gamma 2.2
    let raw_ev = if p50 > 0.01 {
        (target_mid / p50).log2() * 0.45
    } else {
        0.0
    };
    let ev = if raw_ev.abs() < 0.07 {
        0.0
    } else {
        raw_ev.clamp(-2.0, 2.0)
    } as f32;

    // ── Highlights ──
    // In perceptual space, 0.58 corresponds to ~0.30 linear.
    // sqrt ramp for perceptual uniformity: more aggressive at moderate brightness.
    let highlights = if p95 > 0.58 {
        let t = ((p95 - 0.58) / 0.42).sqrt();
        -(t * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };

    // ── Shadows ──
    // In perceptual space, 0.38 corresponds to ~0.12 linear.
    // sqrt ramp, cap at 70 (matches Lightroom Auto typical range).
    let shadows = if p10 < 0.38 {
        let t = ((0.38 - p10) / 0.38).sqrt();
        (t * 70.0).clamp(0.0, 70.0) as f32
    } else {
        0.0
    };

    // ── Blacks ──
    // Counterbalance shadow lift to maintain depth.
    // ~30% of shadow amount, cap at -25.
    let blacks = if shadows > 5.0 {
        -(shadows * 0.3).min(25.0)
    } else {
        0.0
    };

    // ── Contrast ──
    // +8 baseline; boost for flat histograms (narrow perceptual spread).
    let spread = p95 - p5;
    let contrast = if spread < 0.35 {
        ((0.35 - spread) / 0.35 * 12.0 + 8.0).clamp(8.0, 20.0) as f32
    } else {
        8.0_f32
    };

    // ── White balance ──
    // Leave at neutral. Gray-world WB is unreliable for real scenes.
    let wb_temp = 5500.0_f32;

    // ── Vibrance ──
    // Modest boost proportional to how desaturated the image is.
    let avg_sat = if sat_count > 0 {
        sum_sat / sat_count as f64
    } else {
        0.0
    };
    let vibrance = ((1.0 - avg_sat) * 25.0).clamp(5.0, 25.0) as f32;

    EditParams {
        exposure: ev,
        wb_temp,
        wb_tint: 0.0,
        contrast,
        highlights,
        shadows,
        blacks,
        vibrance,
        saturation: 0.0,
        crop_x: 0.0,
        crop_y: 0.0,
        crop_w: 1.0,
        crop_h: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_image(r: f32, g: f32, b: f32, size: u32) -> ImageBuf {
        let pixel_count = (size * size) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);
        for _ in 0..pixel_count {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        ImageBuf::from_data(size, size, data).unwrap()
    }

    fn scene_image(dark: f32, mid: f32, bright: f32, size: u32) -> ImageBuf {
        let pixel_count = (size * size) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);
        for i in 0..pixel_count {
            let v = match i % 3 {
                0 => dark,
                1 => mid,
                _ => bright,
            };
            data.push(v);
            data.push(v);
            data.push(v);
        }
        ImageBuf::from_data(size, size, data).unwrap()
    }

    #[test]
    fn dark_image_positive_ev() {
        let buf = uniform_image(0.02, 0.02, 0.02, 10);
        let params = auto_enhance(&buf);
        assert!(
            params.exposure > 0.5,
            "dark image should get positive EV, got {}",
            params.exposure
        );
    }

    #[test]
    fn bright_image_negative_ev() {
        let buf = uniform_image(0.8, 0.8, 0.8, 10);
        let params = auto_enhance(&buf);
        assert!(
            params.exposure < -0.3,
            "bright image should get negative EV, got {}",
            params.exposure
        );
    }

    #[test]
    fn neutral_image_dead_zone() {
        let buf = uniform_image(0.18, 0.18, 0.18, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.exposure, 0.0,
            "mid-gray image should land in dead zone, got {}",
            params.exposure
        );
    }

    #[test]
    fn highlights_recover_bright_areas() {
        let buf = scene_image(0.1, 0.3, 0.85, 12);
        let params = auto_enhance(&buf);
        assert!(
            params.highlights < -20.0,
            "bright p95 should trigger highlight recovery, got {}",
            params.highlights
        );
        assert!(
            params.highlights >= -100.0,
            "highlights should be capped at -100, got {}",
            params.highlights
        );
    }

    #[test]
    fn no_highlights_for_dark_scene() {
        let buf = uniform_image(0.1, 0.1, 0.1, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.highlights, 0.0,
            "dark scene should not pull highlights, got {}",
            params.highlights
        );
    }

    #[test]
    fn shadows_lift_dark_tones() {
        let buf = scene_image(0.005, 0.15, 0.6, 12);
        let params = auto_enhance(&buf);
        assert!(
            params.shadows > 10.0,
            "dark p10 should trigger shadow lift, got {}",
            params.shadows
        );
        assert!(
            params.shadows <= 70.0,
            "shadows should be capped at 70, got {}",
            params.shadows
        );
    }

    #[test]
    fn no_shadows_for_bright_scene() {
        let buf = uniform_image(0.3, 0.3, 0.3, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.shadows, 0.0,
            "bright scene should not lift shadows, got {}",
            params.shadows
        );
    }

    #[test]
    fn blacks_counterbalance_shadow_lift() {
        let buf = scene_image(0.002, 0.15, 0.6, 12);
        let params = auto_enhance(&buf);
        assert!(
            params.shadows > 10.0,
            "precondition: shadows should be lifted"
        );
        assert!(
            params.blacks < -3.0,
            "blacks should crush to counterbalance shadow lift, got {}",
            params.blacks
        );
        let expected_ratio = -params.blacks / params.shadows;
        assert!(
            (0.2..=0.4).contains(&expected_ratio),
            "blacks/shadows ratio should be ~0.3, got {}",
            expected_ratio
        );
    }

    #[test]
    fn contrast_baseline_for_normal_image() {
        let buf = scene_image(0.05, 0.3, 0.8, 12);
        let params = auto_enhance(&buf);
        assert!(
            (7.5..=8.5).contains(&params.contrast),
            "normal image should get ~8 contrast, got {}",
            params.contrast
        );
    }

    #[test]
    fn contrast_boost_for_flat_image() {
        let buf = uniform_image(0.18, 0.18, 0.18, 10);
        let params = auto_enhance(&buf);
        assert!(
            params.contrast > 8.0,
            "flat image should get contrast boost above baseline, got {}",
            params.contrast
        );
    }

    #[test]
    fn wb_always_neutral() {
        let buf = uniform_image(0.5, 0.2, 0.2, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.wb_temp, 5500.0,
            "WB should always stay at 5500, got {}",
            params.wb_temp
        );
    }

    #[test]
    fn all_params_valid_ranges() {
        let buf = scene_image(0.01, 0.2, 0.7, 20);
        let p = auto_enhance(&buf);
        assert!((-2.0..=2.0).contains(&p.exposure), "exposure {}", p.exposure);
        assert_eq!(p.wb_temp, 5500.0);
        assert_eq!(p.wb_tint, 0.0);
        assert!((0.0..=20.0).contains(&p.contrast), "contrast {}", p.contrast);
        assert!(
            (-100.0..=0.0).contains(&p.highlights),
            "highlights {}",
            p.highlights
        );
        assert!((0.0..=70.0).contains(&p.shadows), "shadows {}", p.shadows);
        assert!(
            (-25.0..=0.0).contains(&p.blacks),
            "blacks {}",
            p.blacks
        );
        assert!(
            (5.0..=25.0).contains(&p.vibrance),
            "vibrance {}",
            p.vibrance
        );
        assert_eq!(p.saturation, 0.0);
        assert_eq!(p.crop_x, 0.0);
        assert_eq!(p.crop_y, 0.0);
        assert_eq!(p.crop_w, 1.0);
        assert_eq!(p.crop_h, 1.0);
    }

    #[test]
    fn zero_image_no_panic() {
        let buf = uniform_image(0.0, 0.0, 0.0, 10);
        let params = auto_enhance(&buf);
        assert!(params.exposure.is_finite());
        assert!(params.wb_temp.is_finite());
    }

    #[test]
    fn single_pixel() {
        let buf = ImageBuf::from_data(1, 1, vec![0.3, 0.3, 0.3]).unwrap();
        let params = auto_enhance(&buf);
        assert!(params.exposure.is_finite());
    }

    #[test]
    fn empty_image_returns_defaults() {
        let buf = ImageBuf::from_data(0, 0, vec![]).unwrap();
        let params = auto_enhance(&buf);
        let defaults = EditParams::default();
        assert_eq!(params.exposure, defaults.exposure);
        assert_eq!(params.wb_temp, defaults.wb_temp);
    }
}
