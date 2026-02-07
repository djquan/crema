use crate::color::{linear_srgb_to_oklab, linear_to_srgb, srgb_to_linear};
use crate::image_buf::{EditParams, ImageBuf};

/// Analyze a linear f32 preview and produce edit suggestions.
///
/// Histogram analysis is performed in **perceptual space** (sRGB EOTF)
/// where human vision is roughly uniform, making thresholds intuitive.
///
/// Strategy (inspired by pre-ML Lightroom Auto):
///   - Highlights and shadows do the heavy lifting, not exposure.
///   - Feedforward: exposure correction is applied to percentiles
///     before computing highlight/shadow targets.
///   - Pull down bright areas (sqrt ramp for perceptual uniformity).
///   - Lift dark areas to reveal shadow detail (sqrt ramp).
///   - Crush blacks to maintain depth after shadow lift.
///   - Small exposure nudge (45% strength, dead zone +/-0.07 EV).
///   - Gray-point WB: detect neutral candidates via OKLab chroma,
///     estimate and correct color casts conservatively.
///   - Modest contrast and vibrance boost.
pub fn auto_enhance(buf: &ImageBuf) -> EditParams {
    let pixel_count = buf.pixel_count();
    if pixel_count == 0 {
        return EditParams::default();
    }

    let mut luminances = Vec::with_capacity(pixel_count);
    let mut sum_sat = 0.0_f64;
    let mut sat_count = 0_u64;

    // Gray-point WB: accumulate RGB of neutral candidates (low OKLab chroma).
    let mut wb_sum_r = 0.0_f64;
    let mut wb_sum_b = 0.0_f64;
    let mut wb_sum_ok_a = 0.0_f64;
    let mut wb_count = 0_u64;

    for pixel in buf.data.chunks_exact(3) {
        let r = pixel[0] as f64;
        let g = pixel[1] as f64;
        let b = pixel[2] as f64;

        let y_linear = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        luminances.push(linear_to_srgb(y_linear.max(0.0) as f32) as f64);

        let max_ch = r.max(g).max(b);
        if max_ch >= 0.02 {
            let min_ch = r.min(g).min(b);
            sum_sat += (max_ch - min_ch) / (max_ch + 1e-6);
            sat_count += 1;
        }

        // Neutral candidate: mid-brightness, low OKLab chroma.
        let (ok_l, ok_a, ok_b) = linear_srgb_to_oklab(pixel[0], pixel[1], pixel[2]);
        let ok_chroma = (ok_a * ok_a + ok_b * ok_b).sqrt();
        if ok_l > 0.3 && ok_l < 0.85 && ok_chroma < 0.04 {
            wb_sum_r += r;
            wb_sum_b += b;
            wb_sum_ok_a += ok_a as f64;
            wb_count += 1;
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
    // Target: perceptual mid-gray (~0.46). 45% strength correction.
    let target_mid = 0.461; // linear_to_srgb(0.18) ≈ 0.4613
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

    // ── Feedforward: simulate exposure effect on perceptual percentiles ──
    // Without this, the exposure and tonal controls are estimated independently
    // from the original histogram. Feedforward accounts for how the exposure
    // change shifts the histogram before setting highlight/shadow targets.
    let correct_p = |p: f64| -> f64 {
        if ev == 0.0 {
            return p;
        }
        let linear = srgb_to_linear(p as f32) as f64;
        let corrected = linear * 2.0_f64.powf(ev as f64);
        linear_to_srgb(corrected.clamp(0.0, 1.0) as f32) as f64
    };
    let p5_c = correct_p(p5);
    let p10_c = correct_p(p10);
    let p95_c = correct_p(p95);

    // ── Highlights ── (uses exposure-corrected p95)
    // In perceptual space, 0.58 corresponds to ~0.30 linear.
    // sqrt ramp for perceptual uniformity: more aggressive at moderate brightness.
    let highlights = if p95_c > 0.58 {
        let t = ((p95_c - 0.58) / 0.42).sqrt();
        -(t * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };

    // ── Shadows ── (uses exposure-corrected p10)
    // In perceptual space, 0.38 corresponds to ~0.12 linear.
    // sqrt ramp, cap at 70 (matches Lightroom Auto typical range).
    let shadows = if p10_c < 0.38 {
        let t = ((0.38 - p10_c) / 0.38).sqrt();
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

    // ── Contrast ── (uses exposure-corrected spread)
    // Zero for well-spread images; boost proportional to how flat the histogram is.
    let spread = p95_c - p5_c;
    let contrast = if spread < 0.5 {
        ((0.5 - spread) / 0.5 * 20.0).clamp(0.0, 20.0) as f32
    } else {
        0.0_f32
    };

    // ── White balance: gray-point estimation ──
    let (wb_temp, wb_tint) = estimate_wb(wb_sum_r, wb_sum_b, wb_sum_ok_a, wb_count, pixel_count);

    // ── Vibrance ──
    // Modest boost proportional to how desaturated the image is.
    let avg_sat = if sat_count > 0 {
        sum_sat / sat_count as f64
    } else {
        0.0
    };
    let vibrance = ((1.0 - avg_sat) * 25.0).clamp(0.0, 25.0) as f32;

    EditParams {
        exposure: ev,
        wb_temp,
        wb_tint,
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

/// Estimate white balance from neutral candidate pixels.
///
/// Requires at least 2% of pixels to be neutral candidates (low OKLab
/// chroma, mid brightness). If not enough candidates exist, returns
/// neutral (5500K, tint=0) to avoid hallucinating corrections in scenes
/// with no reliable reference (e.g., sunsets, neon lighting).
fn estimate_wb(
    sum_r: f64,
    sum_b: f64,
    sum_ok_a: f64,
    count: u64,
    pixel_count: usize,
) -> (f32, f32) {
    if count < (pixel_count as u64 / 50).max(3) {
        return (5500.0, 0.0);
    }

    let avg_r = sum_r / count as f64;
    let avg_b = sum_b / count as f64;

    if avg_r < 0.01 || avg_b < 0.01 {
        return (5500.0, 0.0);
    }

    // Temperature from R/B ratio in linear light.
    // log2(R/B) > 0 means warm cast; correction lowers wb_temp to cool.
    let rb_log = (avg_r / avg_b).log2();
    let temp_shift = if rb_log.abs() > 0.05 {
        (-rb_log * 1500.0).clamp(-2000.0, 2000.0)
    } else {
        0.0
    };

    // Tint from OKLab a-channel of neutral candidates.
    // Positive ok_a = red/magenta cast, negative = green cast.
    // Negate: green cast (ok_a < 0) needs positive tint (add magenta).
    let avg_ok_a = sum_ok_a / count as f64;
    let tint_shift = if avg_ok_a.abs() > 0.003 {
        (-avg_ok_a * 1000.0).clamp(-30.0, 30.0)
    } else {
        0.0
    };

    (
        (5500.0 + temp_shift).clamp(3000.0, 9000.0) as f32,
        tint_shift as f32,
    )
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
    fn contrast_zero_for_well_spread_image() {
        let buf = scene_image(0.05, 0.3, 0.8, 12);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.contrast, 0.0,
            "well-spread image should get zero contrast, got {}",
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
    fn wb_neutral_for_saturated_scene() {
        // All pixels are high-chroma (warm red) -> no neutral candidates
        let buf = uniform_image(0.5, 0.2, 0.2, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.wb_temp, 5500.0,
            "no neutral candidates -> WB should stay at 5500, got {}",
            params.wb_temp
        );
    }

    #[test]
    fn wb_neutral_for_achromatic_scene() {
        // Gray pixels have no cast -> WB stays neutral
        let buf = uniform_image(0.3, 0.3, 0.3, 10);
        let params = auto_enhance(&buf);
        assert_eq!(
            params.wb_temp, 5500.0,
            "neutral gray -> no correction needed, got {}",
            params.wb_temp
        );
        assert_eq!(params.wb_tint, 0.0);
    }

    #[test]
    fn wb_corrects_warm_cast() {
        // Neutral-ish pixels with warm tint (R > G > B)
        let buf = uniform_image(0.28, 0.25, 0.22, 20);
        let params = auto_enhance(&buf);
        assert!(
            params.wb_temp < 5500.0,
            "warm cast should lower wb_temp below 5500, got {}",
            params.wb_temp
        );
        assert!(
            params.wb_temp > 3000.0,
            "correction should be conservative, got {}",
            params.wb_temp
        );
    }

    #[test]
    fn wb_corrects_cool_cast() {
        // Neutral-ish pixels with cool tint (B > G > R)
        let buf = uniform_image(0.22, 0.25, 0.28, 20);
        let params = auto_enhance(&buf);
        assert!(
            params.wb_temp > 5500.0,
            "cool cast should raise wb_temp above 5500, got {}",
            params.wb_temp
        );
        assert!(
            params.wb_temp < 9000.0,
            "correction should be conservative, got {}",
            params.wb_temp
        );
    }

    #[test]
    fn feedforward_reduces_highlight_recovery() {
        // Scene with bright highlights and slightly dark overall (positive EV).
        // Feedforward should predict that exposure boost pushes highlights higher,
        // resulting in MORE highlight recovery than without feedforward.
        let buf = scene_image(0.03, 0.10, 0.75, 12);
        let params = auto_enhance(&buf);
        // With positive EV feedforward, p95 shifts up -> highlights more negative
        assert!(
            params.highlights < 0.0,
            "feedforward with positive EV should still trigger highlight recovery, got {}",
            params.highlights
        );
    }

    #[test]
    fn all_params_valid_ranges() {
        let buf = scene_image(0.01, 0.2, 0.7, 20);
        let p = auto_enhance(&buf);
        assert!(
            (-2.0..=2.0).contains(&p.exposure),
            "exposure {}",
            p.exposure
        );
        assert!(
            (3000.0..=9000.0).contains(&p.wb_temp),
            "wb_temp {}",
            p.wb_temp
        );
        assert!((-30.0..=30.0).contains(&p.wb_tint), "wb_tint {}", p.wb_tint);
        assert!(
            (0.0..=20.0).contains(&p.contrast),
            "contrast {}",
            p.contrast
        );
        assert!(
            (-100.0..=0.0).contains(&p.highlights),
            "highlights {}",
            p.highlights
        );
        assert!((0.0..=70.0).contains(&p.shadows), "shadows {}", p.shadows);
        assert!((-25.0..=0.0).contains(&p.blacks), "blacks {}", p.blacks);
        assert!(
            (0.0..=25.0).contains(&p.vibrance),
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

    #[test]
    fn bimodal_histogram_balanced() {
        // 50% dark pixels, 50% bright pixels (backlit scene).
        // Median should be near the boundary; exposure should be moderate.
        let size = 20_u32;
        let pixel_count = (size * size) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);
        for i in 0..pixel_count {
            let v = if i < pixel_count / 2 { 0.02 } else { 0.7 };
            data.push(v);
            data.push(v);
            data.push(v);
        }
        let buf = ImageBuf::from_data(size, size, data).unwrap();
        let params = auto_enhance(&buf);

        // Should not wildly over-expose or under-expose
        assert!(
            params.exposure.abs() < 1.5,
            "bimodal scene exposure should be moderate: {}",
            params.exposure
        );
        // Should trigger both shadow lift and highlight recovery
        assert!(
            params.shadows > 0.0 || params.highlights < 0.0,
            "bimodal scene should trigger tonal recovery: shadows={} highlights={}",
            params.shadows,
            params.highlights
        );
    }

    #[test]
    fn neutral_threshold_boundary() {
        // Build an image where exactly 2% of pixels are neutral candidates,
        // and verify WB correction activates.
        let size = 50_u32; // 2500 pixels; 2% = 50 pixels
        let pixel_count = (size * size) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);

        let neutral_count = pixel_count / 50; // exactly 2%
        for i in 0..pixel_count {
            if i < neutral_count {
                // Neutral-ish with warm cast: R slightly > B
                data.push(0.32);
                data.push(0.30);
                data.push(0.25);
            } else {
                // Saturated (non-neutral) pixels
                data.push(0.8);
                data.push(0.2);
                data.push(0.1);
            }
        }
        let buf = ImageBuf::from_data(size, size, data).unwrap();
        let params = auto_enhance(&buf);

        // With exactly 2% neutral candidates and a warm cast, wb_temp should shift
        assert!(
            params.wb_temp != 5500.0,
            "with 2% neutral warm-cast candidates, WB should adjust: {}",
            params.wb_temp
        );
    }

    #[test]
    fn tint_corrects_green_cast() {
        // Gray-ish pixels with a green tint (green channel elevated).
        // OKLab a-channel should be negative for green.
        let buf = uniform_image(0.25, 0.32, 0.25, 20);
        let params = auto_enhance(&buf);
        assert!(
            params.wb_tint > 0.0,
            "green cast should produce positive tint correction: {}",
            params.wb_tint
        );
    }

    #[test]
    fn tint_corrects_magenta_cast() {
        // Gray-ish pixels with a magenta tint (green channel depressed).
        // OKLab a-channel should be positive for magenta.
        let buf = uniform_image(0.30, 0.22, 0.30, 20);
        let params = auto_enhance(&buf);
        assert!(
            params.wb_tint < 0.0,
            "magenta cast should produce negative tint correction: {}",
            params.wb_tint
        );
    }

    #[test]
    fn hdr_input_no_panic() {
        let buf = uniform_image(2.0, 1.5, 3.0, 10);
        let params = auto_enhance(&buf);
        assert!(params.exposure.is_finite());
        assert!(params.wb_temp.is_finite());
        assert!(params.highlights.is_finite());
    }

    #[test]
    fn saturated_scene_zero_vibrance() {
        // Highly saturated scene: vibrance should be near zero, not forced to +5.
        let buf = uniform_image(0.8, 0.1, 0.1, 10);
        let params = auto_enhance(&buf);
        assert!(
            params.vibrance < 5.0,
            "saturated scene should get low vibrance, got {}",
            params.vibrance
        );
    }

    #[test]
    fn nearly_clipped_image() {
        // 99% of pixels near 1.0 (overexposed scene).
        let size = 10_u32;
        let pixel_count = (size * size) as usize;
        let mut data = Vec::with_capacity(pixel_count * 3);
        for i in 0..pixel_count {
            let v = if i == 0 { 0.1 } else { 0.95 };
            data.push(v);
            data.push(v);
            data.push(v);
        }
        let buf = ImageBuf::from_data(size, size, data).unwrap();
        let params = auto_enhance(&buf);
        assert!(
            params.exposure < 0.0,
            "nearly clipped image should get negative exposure: {}",
            params.exposure
        );
        assert!(
            params.highlights < 0.0,
            "nearly clipped image should recover highlights: {}",
            params.highlights
        );
    }

    #[test]
    fn end_to_end_auto_then_pipeline() {
        use crate::pipeline::Pipeline;
        let buf = scene_image(0.01, 0.15, 0.7, 20);
        let params = auto_enhance(&buf);
        let pipeline = Pipeline::new();
        let output = pipeline.process_cpu(buf, &params).unwrap();
        assert!(
            output.data.iter().all(|v| v.is_finite()),
            "pipeline output should be finite"
        );
        // Output should have better tonal distribution
        let y_sum: f64 = output
            .data
            .chunks_exact(3)
            .map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64)
            .sum();
        let avg_y = y_sum / output.pixel_count() as f64;
        assert!(
            avg_y > 0.05 && avg_y < 0.8,
            "auto-enhanced output should have reasonable average luminance: {avg_y}"
        );
    }
}
