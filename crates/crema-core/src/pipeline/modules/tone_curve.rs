use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

const LUT_SIZE: usize = 4096;

pub struct ToneCurve;

impl ProcessingModule for ToneCurve {
    fn name(&self) -> &str {
        "tone_curve"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.contrast == 0.0
            && params.highlights == 0.0
            && params.shadows == 0.0
            && params.blacks == 0.0
        {
            return Ok(input);
        }

        let lut = build_tone_lut(params);

        for pixel in input.data.chunks_exact_mut(3) {
            let y = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
            if y < 1e-6 {
                continue;
            }

            let y_in = y.clamp(0.0, 1.0);
            let new_y = lut_lerp(&lut, y_in);
            let scale = new_y / y_in;

            pixel[0] = (pixel[0] * scale).max(0.0);
            pixel[1] = (pixel[1] * scale).max(0.0);
            pixel[2] = (pixel[2] * scale).max(0.0);
        }

        Ok(input)
    }
}

// ── Zone layout ──────────────────────────────────────────────────────────
//
// The LUT is built in perceptual space (gamma 2.2), matching how humans
// perceive brightness. Four zones with Lightroom-style boundaries:
//
// ```text
//   perceptual 0.00─0.15  Blacks region  (additive, smoothstep weight)
//              0.10─0.35  Shadow zone    (power curve, gamma varies)
//              0.35─0.65  Midtone gap    (identity, only contrast S-curve)
//              0.65─0.90  Highlight zone (power curve, gamma varies)
//              full range: Contrast S-curve overlay (x^a / (x^a + (1-x)^a))
// ```
//
// Power curves use `n^gamma` where n is normalized [0,1] within the zone.
// gamma < 1 lifts (positive slider), gamma > 1 crushes (negative slider).
// Boundaries are feathered over 5% to ensure C1 slope continuity.

const SHADOW_LO: f32 = 0.10;
const SHADOW_HI: f32 = 0.35;
const HIGHLIGHT_LO: f32 = 0.65;
const HIGHLIGHT_HI: f32 = 0.90;
const FEATHER: f32 = 0.05;

fn build_tone_lut(params: &EditParams) -> [f32; LUT_SIZE] {
    let contrast = params.contrast / 100.0;
    let highlights = params.highlights / 100.0;
    let shadows = params.shadows / 100.0;
    let blacks = params.blacks / 100.0;

    let shadow_width = SHADOW_HI - SHADOW_LO;
    let highlight_width = HIGHLIGHT_HI - HIGHLIGHT_LO;

    // gamma = 3^(-slider): positive slider -> gamma < 1 -> lift/boost
    //                       negative slider -> gamma > 1 -> crush/recover
    let shadow_gamma = 3.0_f32.powf(-shadows);
    let highlight_gamma = 3.0_f32.powf(-highlights);

    let mut lut = [0.0_f32; LUT_SIZE];

    for (i, entry) in lut.iter_mut().enumerate() {
        let linear_in = i as f32 / (LUT_SIZE - 1) as f32;
        let t = linear_in.powf(1.0 / 2.2);

        let mut out = t;

        // Shadow zone [SHADOW_LO, SHADOW_HI] with feathered boundaries
        if t > SHADOW_LO - FEATHER && t < SHADOW_HI + FEATHER && shadows != 0.0 {
            let n = ((t - SHADOW_LO) / shadow_width).clamp(0.0, 1.0);
            let shadow_val = SHADOW_LO + n.powf(shadow_gamma) * shadow_width;

            if t <= SHADOW_LO {
                // Below zone: feather in from identity
                let blend = smoothstep((t - (SHADOW_LO - FEATHER)) / FEATHER);
                out = t * (1.0 - blend) + shadow_val * blend;
            } else if t >= SHADOW_HI {
                // Above zone: feather out to identity
                let blend = smoothstep((t - SHADOW_HI) / FEATHER);
                out = shadow_val * (1.0 - blend) + t * blend;
            } else {
                out = shadow_val;
            }
        }

        // Highlight zone [HIGHLIGHT_LO, HIGHLIGHT_HI] with feathered boundaries
        if t > HIGHLIGHT_LO - FEATHER && t < HIGHLIGHT_HI + FEATHER && highlights != 0.0 {
            let n = ((t - HIGHLIGHT_LO) / highlight_width).clamp(0.0, 1.0);
            let highlight_val = HIGHLIGHT_LO + n.powf(highlight_gamma) * highlight_width;

            if t <= HIGHLIGHT_LO {
                let blend = smoothstep((t - (HIGHLIGHT_LO - FEATHER)) / FEATHER);
                out = out * (1.0 - blend) + highlight_val * blend;
            } else if t >= HIGHLIGHT_HI {
                let blend = smoothstep((t - HIGHLIGHT_HI) / FEATHER);
                out = highlight_val * (1.0 - blend) + t * blend;
            } else {
                out = highlight_val;
            }
        }

        // Contrast: slope-based S-curve using x^a / (x^a + (1-x)^a)
        // a=1 is identity, a>1 increases contrast, a<1 decreases contrast
        if contrast != 0.0 {
            let a = 3.0_f32.powf(contrast);
            out = s_curve(out, a);
        }

        // Blacks: additive near-black adjustment with smooth rolloff
        if blacks != 0.0 {
            out += blacks * smoothstep_down(out, 0.0, 0.15) * 0.30;
        }

        *entry = out.clamp(0.0, 1.0).powf(2.2);
    }

    // Enforce monotonicity (safety net for extreme combined settings)
    for i in 1..LUT_SIZE {
        if lut[i] < lut[i - 1] {
            lut[i] = lut[i - 1];
        }
    }

    lut
}

/// S-curve: x^a / (x^a + (1-x)^a)
///
/// Properties: f(0)=0, f(1)=1, f(0.5)=0.5, monotonic for a>0.
/// a=1 is identity; a>1 increases slope at midpoint (contrast boost);
/// a<1 decreases slope (contrast reduction).
fn s_curve(x: f32, a: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let xa = x.powf(a);
    let one_minus_xa = (1.0 - x).powf(a);
    xa / (xa + one_minus_xa)
}

/// Hermite smoothstep: 0 at t<=0, 1 at t>=1, smooth in between.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth ramp from 1.0 at `lo` to 0.0 at `hi`.
fn smoothstep_down(x: f32, lo: f32, hi: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

fn lut_lerp(lut: &[f32; LUT_SIZE], y: f32) -> f32 {
    let idx_f = y * (LUT_SIZE - 1) as f32;
    let i0 = (idx_f as usize).min(LUT_SIZE - 2);
    let frac = idx_f - i0 as f32;
    lut[i0] * (1.0 - frac) + lut[i0 + 1] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(r: f32, g: f32, b: f32, w: u32, h: u32) -> ImageBuf {
        let n = (w * h) as usize;
        let mut data = Vec::with_capacity(n * 3);
        for _ in 0..n {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        ImageBuf::from_data(w, h, data).unwrap()
    }

    fn params_with(f: impl FnOnce(&mut EditParams)) -> EditParams {
        let mut p = EditParams::default();
        f(&mut p);
        p
    }

    // ── Identity ──

    #[test]
    fn identity_noop() {
        let buf = uniform(0.5, 0.5, 0.5, 4, 4);
        let expected = buf.data.clone();
        let result = ToneCurve
            .process_cpu(buf, &EditParams::default())
            .unwrap();
        assert_eq!(result.data, expected);
    }

    // ── Contrast ──

    #[test]
    fn contrast_positive_spreads() {
        let params = params_with(|p| p.contrast = 50.0);
        let bright = ToneCurve
            .process_cpu(uniform(0.5, 0.5, 0.5, 1, 1), &params)
            .unwrap();
        let dark = ToneCurve
            .process_cpu(uniform(0.05, 0.05, 0.05, 1, 1), &params)
            .unwrap();
        assert!(bright.data[0] > 0.5, "bright should get brighter");
        assert!(dark.data[0] < 0.05, "dark should get darker");
    }

    #[test]
    fn contrast_negative_compresses() {
        let params = params_with(|p| p.contrast = -50.0);
        let bright = ToneCurve
            .process_cpu(uniform(0.5, 0.5, 0.5, 1, 1), &params)
            .unwrap();
        let dark = ToneCurve
            .process_cpu(uniform(0.05, 0.05, 0.05, 1, 1), &params)
            .unwrap();
        assert!(bright.data[0] < 0.5, "bright should get darker");
        assert!(dark.data[0] > 0.05, "dark should get brighter");
    }

    #[test]
    fn contrast_preserves_midpoint() {
        let params = params_with(|p| p.contrast = 80.0);
        let result = ToneCurve
            .process_cpu(uniform(0.18, 0.18, 0.18, 1, 1), &params)
            .unwrap();
        // 0.18 linear -> ~0.46 perceptual, S-curve pivot at 0.5, small shift
        let delta = (result.data[0] - 0.18).abs();
        assert!(
            delta < 0.05,
            "mid-gray (0.18) should be near-stable under contrast, delta={delta}"
        );
    }

    #[test]
    fn s_curve_identity_at_one() {
        assert!((s_curve(0.0, 1.0)).abs() < 1e-6);
        assert!((s_curve(0.5, 1.0) - 0.5).abs() < 1e-6);
        assert!((s_curve(1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((s_curve(0.3, 1.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn s_curve_increases_contrast() {
        let a = 2.0;
        // Below midpoint: should decrease (darken)
        assert!(s_curve(0.3, a) < 0.3);
        // Above midpoint: should increase (brighten)
        assert!(s_curve(0.7, a) > 0.7);
        // Midpoint preserved
        assert!((s_curve(0.5, a) - 0.5).abs() < 1e-6);
    }

    // ── Shadows ──

    #[test]
    fn shadows_positive_lifts_dark() {
        let params = params_with(|p| p.shadows = 50.0);
        let result = ToneCurve
            .process_cpu(uniform(0.02, 0.02, 0.02, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] > 0.02,
            "shadows+50 should lift dark pixels, got {}",
            result.data[0]
        );
    }

    #[test]
    fn shadows_negative_darkens() {
        let params = params_with(|p| p.shadows = -50.0);
        let result = ToneCurve
            .process_cpu(uniform(0.05, 0.05, 0.05, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] < 0.05,
            "shadows-50 should darken dark pixels, got {}",
            result.data[0]
        );
    }

    #[test]
    fn shadows_bright_unaffected() {
        let params = params_with(|p| p.shadows = 100.0);
        let result = ToneCurve
            .process_cpu(uniform(0.8, 0.8, 0.8, 1, 1), &params)
            .unwrap();
        // 0.8 linear -> perceptual ~0.91, well above shadow zone [0.10, 0.35]
        let delta = (result.data[0] - 0.8).abs();
        assert!(
            delta < 0.02,
            "bright pixels should be barely affected by shadows, delta={delta}"
        );
    }

    // ── Highlights ──

    #[test]
    fn highlights_negative_recovers() {
        let params = params_with(|p| p.highlights = -60.0);
        let result = ToneCurve
            .process_cpu(uniform(0.85, 0.85, 0.85, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] < 0.85,
            "highlights-60 should pull bright down, got {}",
            result.data[0]
        );
    }

    #[test]
    fn highlights_positive_boosts() {
        let params = params_with(|p| p.highlights = 50.0);
        let result = ToneCurve
            .process_cpu(uniform(0.7, 0.7, 0.7, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] > 0.7,
            "highlights+50 should boost bright, got {}",
            result.data[0]
        );
    }

    #[test]
    fn highlights_dark_unaffected() {
        let params = params_with(|p| p.highlights = -100.0);
        let result = ToneCurve
            .process_cpu(uniform(0.02, 0.02, 0.02, 1, 1), &params)
            .unwrap();
        // 0.02 linear -> perceptual ~0.18, well below highlight zone [0.65, 0.90]
        let delta = (result.data[0] - 0.02).abs();
        assert!(
            delta < 0.005,
            "dark pixels should be barely affected by highlights, delta={delta}"
        );
    }

    // ── Blacks ──

    #[test]
    fn blacks_negative_crushes() {
        let params = params_with(|p| p.blacks = -50.0);
        let result = ToneCurve
            .process_cpu(uniform(0.01, 0.01, 0.01, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] < 0.01,
            "blacks-50 should crush near-black, got {}",
            result.data[0]
        );
    }

    #[test]
    fn blacks_positive_lifts() {
        let params = params_with(|p| p.blacks = 50.0);
        let result = ToneCurve
            .process_cpu(uniform(0.01, 0.01, 0.01, 1, 1), &params)
            .unwrap();
        assert!(
            result.data[0] > 0.01,
            "blacks+50 should lift near-black, got {}",
            result.data[0]
        );
    }

    #[test]
    fn blacks_bright_unaffected() {
        let params = params_with(|p| p.blacks = -100.0);
        let result = ToneCurve
            .process_cpu(uniform(0.5, 0.5, 0.5, 1, 1), &params)
            .unwrap();
        // 0.5 linear -> perceptual ~0.73, well above blacks region [0, 0.15]
        let delta = (result.data[0] - 0.5).abs();
        assert!(
            delta < 0.01,
            "bright pixels should be unaffected by blacks, delta={delta}"
        );
    }

    // ── Cross-cutting ──

    #[test]
    fn monotonic_lut() {
        let params = EditParams {
            contrast: 80.0,
            highlights: -60.0,
            shadows: 80.0,
            blacks: -30.0,
            ..Default::default()
        };
        let lut = build_tone_lut(&params);
        for i in 1..LUT_SIZE {
            assert!(
                lut[i] >= lut[i - 1],
                "LUT must be monotonic: lut[{i}]={} < lut[{}]={}",
                lut[i],
                i - 1,
                lut[i - 1]
            );
        }
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = uniform(0.3, 0.3, 0.3, 4, 4);
        for v in [-100.0_f32, 100.0] {
            let params = EditParams {
                contrast: v,
                highlights: v,
                shadows: v,
                blacks: v,
                ..Default::default()
            };
            let result = ToneCurve.process_cpu(buf.clone(), &params).unwrap();
            assert!(result.data.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn preserves_dimensions() {
        let buf = uniform(0.4, 0.4, 0.4, 10, 5);
        let params = params_with(|p| p.contrast = 30.0);
        let result = ToneCurve.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn preserves_neutral_gray() {
        let buf = uniform(0.3, 0.3, 0.3, 1, 1);
        let params = params_with(|p| p.shadows = 40.0);
        let result = ToneCurve.process_cpu(buf, &params).unwrap();
        let r = result.data[0];
        let g = result.data[1];
        let b = result.data[2];
        assert!(
            (r - g).abs() < 1e-6 && (g - b).abs() < 1e-6,
            "neutral gray must stay neutral: [{r}, {g}, {b}]"
        );
    }

    #[test]
    fn zero_pixels_stay_zero() {
        let buf = uniform(0.0, 0.0, 0.0, 1, 1);
        let params = EditParams {
            contrast: 50.0,
            shadows: 80.0,
            ..Default::default()
        };
        let result = ToneCurve.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn lut_endpoints() {
        let params = params_with(|p| p.contrast = 30.0);
        let lut = build_tone_lut(&params);
        assert!(
            lut[0] < 0.001,
            "LUT at 0 should be near-zero, got {}",
            lut[0]
        );
        assert!(
            lut[LUT_SIZE - 1] > 0.99,
            "LUT at 1 should be near-one, got {}",
            lut[LUT_SIZE - 1]
        );
    }

    #[test]
    fn shadow_zone_does_not_affect_midtones() {
        // 0.18 linear -> ~0.46 perceptual, above shadow zone [0.10, 0.35+feather]
        let params = params_with(|p| p.shadows = 100.0);
        let result = ToneCurve
            .process_cpu(uniform(0.18, 0.18, 0.18, 1, 1), &params)
            .unwrap();
        let delta = (result.data[0] - 0.18).abs();
        assert!(
            delta < 0.01,
            "shadows should not affect mid-gray (0.18 linear), delta={delta}"
        );
    }

    #[test]
    fn highlight_zone_does_not_affect_midtones() {
        // 0.18 linear -> ~0.46 perceptual, below highlight zone [0.65-feather, ...]
        let params = params_with(|p| p.highlights = -100.0);
        let result = ToneCurve
            .process_cpu(uniform(0.18, 0.18, 0.18, 1, 1), &params)
            .unwrap();
        let delta = (result.data[0] - 0.18).abs();
        assert!(
            delta < 0.01,
            "highlights should not affect mid-gray (0.18 linear), delta={delta}"
        );
    }
}
