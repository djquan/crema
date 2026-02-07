use anyhow::Result;

use crate::color::{OKLAB_MAX_CHROMA, linear_srgb_to_oklab, linear_to_srgb};
use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Vibrance;

impl ProcessingModule for Vibrance {
    fn name(&self) -> &str {
        "vibrance"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.vibrance == 0.0 {
            return Ok(input);
        }

        let strength = params.vibrance / 100.0;
        let sign = strength.signum();
        for pixel in input.data.chunks_exact_mut(3) {
            let y = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];

            // OKLab chroma: perceptually uniform saturation metric.
            let (_, ok_a, ok_b) = linear_srgb_to_oklab(pixel[0], pixel[1], pixel[2]);
            let chroma = (ok_a * ok_a + ok_b * ok_b).sqrt();
            let sat = (chroma / OKLAB_MAX_CHROMA).clamp(0.0, 1.0);

            // Selective saturation (SweetFX/ReShade convention):
            //   positive -> targets low-sat pixels (1 - sat)
            //   negative -> targets high-sat pixels (1 + sat)
            let mut effect = (strength * (1.0 - sign * sat)).max(-1.0);

            // Skin tone protection: reduce effect for warm hues to prevent
            // portraits from looking sunburned (boost) or sickly (cut).
            let max_ch = pixel[0].max(pixel[1]).max(pixel[2]);
            if max_ch > 1e-6 {
                let skin_factor = skin_tone_weight(pixel[0], pixel[1], pixel[2]);
                effect *= 1.0 - skin_factor * 0.7;
            }

            pixel[0] = (y + (1.0 + effect) * (pixel[0] - y)).max(0.0);
            pixel[1] = (y + (1.0 + effect) * (pixel[1] - y)).max(0.0);
            pixel[2] = (y + (1.0 + effect) * (pixel[2] - y)).max(0.0);
        }
        Ok(input)
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Returns 0.0-1.0 indicating how much this pixel looks like a skin tone.
///
/// Computes HSV hue from gamma-encoded (perceptual) RGB so that hue angles
/// match standard HSV definitions. Skin tones cluster around hue 0-55
/// degrees (red through warm yellow); the range wraps around 360/0 to
/// catch very red skin tones at hue ~355-360.
fn skin_tone_weight(r: f32, g: f32, b: f32) -> f32 {
    let rg = linear_to_srgb(r.max(0.0));
    let gg = linear_to_srgb(g.max(0.0));
    let bg = linear_to_srgb(b.max(0.0));

    let max_ch = rg.max(gg).max(bg);
    let min_ch = rg.min(gg).min(bg);
    let chroma = max_ch - min_ch;
    if chroma < 1e-6 {
        return 0.0;
    }

    let hue = if (max_ch - rg).abs() < 1e-6 {
        60.0 * ((gg - bg) / chroma).rem_euclid(6.0)
    } else if (max_ch - gg).abs() < 1e-6 {
        60.0 * ((bg - rg) / chroma + 2.0)
    } else {
        60.0 * ((rg - gg) / chroma + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };

    // Skin tone range: 350-85 degrees (wraps around 0/360).
    // Ramp in: 350-5, plateau: 5-55, ramp out: 55-85.
    // Extended to 85 to cover olive/warm-yellow skin tones.
    if hue >= 350.0 || hue <= 85.0 {
        let h = if hue >= 350.0 { hue - 360.0 } else { hue };
        if h < 5.0 {
            smoothstep((h + 10.0) / 15.0)
        } else if h > 55.0 {
            smoothstep((85.0 - h) / 30.0)
        } else {
            1.0
        }
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_noop() {
        let buf = ImageBuf::from_data(2, 2, vec![0.5; 12]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams::default();
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn positive_boosts_desaturated_more() {
        // Use blue-ish pixels to avoid skin tone protection interference.
        let saturated = ImageBuf::from_data(1, 1, vec![0.2, 0.2, 0.8]).unwrap();
        let desaturated = ImageBuf::from_data(1, 1, vec![0.45, 0.45, 0.55]).unwrap();

        let params = EditParams {
            vibrance: 50.0,
            ..Default::default()
        };

        let sat_result = Vibrance.process_cpu(saturated, &params).unwrap();
        let desat_result = Vibrance.process_cpu(desaturated, &params).unwrap();

        // Compare relative boost: (new_deviation / old_deviation).
        // Vibrance selectivity means desaturated gets a higher percentage boost.
        let sat_y = 0.2126 * 0.2 + 0.7152 * 0.2 + 0.0722 * 0.8;
        let sat_old_dev = (0.8_f32 - sat_y).abs();
        let sat_new_dev = (sat_result.data[2] - sat_y).abs();
        let sat_relative = sat_new_dev / sat_old_dev;

        let desat_y = 0.2126 * 0.45 + 0.7152 * 0.45 + 0.0722 * 0.55;
        let desat_old_dev = (0.55_f32 - desat_y).abs();
        let desat_new_dev = (desat_result.data[2] - desat_y).abs();
        let desat_relative = desat_new_dev / desat_old_dev;

        assert!(
            desat_relative > sat_relative,
            "desaturated pixel should get higher relative boost: desat={desat_relative} sat={sat_relative}"
        );
    }

    #[test]
    fn negative_desaturates() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.2, 0.1]).unwrap();
        let params = EditParams {
            vibrance: -50.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        let spread_before = 0.8 - 0.1;
        let spread_after = result.data[0] - result.data[2];
        assert!(
            spread_after < spread_before,
            "negative vibrance should reduce spread: {spread_after} vs {spread_before}"
        );
    }

    #[test]
    fn negative_targets_saturated_more() {
        // Blue pixel (high sat, not a skin tone)
        let saturated = ImageBuf::from_data(1, 1, vec![0.2, 0.2, 0.8]).unwrap();
        // Low-sat cool pixel (not a skin tone)
        let desaturated = ImageBuf::from_data(1, 1, vec![0.5, 0.45, 0.55]).unwrap();

        let params = EditParams {
            vibrance: -50.0,
            ..Default::default()
        };

        let sat_result = Vibrance.process_cpu(saturated, &params).unwrap();
        let desat_result = Vibrance.process_cpu(desaturated, &params).unwrap();

        let sat_spread_before = 0.8 - 0.2;
        let sat_spread_after = sat_result.data[2] - sat_result.data[0];
        let sat_reduction = sat_spread_before - sat_spread_after;

        let desat_spread_before = 0.55 - 0.45;
        let desat_spread_after = desat_result.data[2] - desat_result.data[0];
        let desat_reduction = desat_spread_before - desat_spread_after;

        assert!(
            sat_reduction > desat_reduction,
            "negative vibrance should desaturate high-sat more: sat={sat_reduction} desat={desat_reduction}"
        );
    }

    #[test]
    fn clamps_at_zero() {
        let buf = ImageBuf::from_data(1, 1, vec![0.1, 0.01, 0.01]).unwrap();
        let params = EditParams {
            vibrance: -100.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        for &v in &result.data {
            assert!(v >= 0.0, "values should be >= 0, got {v}");
        }
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = ImageBuf::from_data(2, 2, vec![0.4; 12]).unwrap();
        for vib in [-100.0, 100.0] {
            let params = EditParams {
                vibrance: vib,
                ..Default::default()
            };
            let result = Vibrance.process_cpu(buf.clone(), &params).unwrap();
            assert!(result.data.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 5, vec![0.3; 150]).unwrap();
        let params = EditParams {
            vibrance: 30.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn gray_pixel_unaffected() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams {
            vibrance: 80.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        for (got, want) in result.data.iter().zip(expected.iter()) {
            assert!(
                (got - want).abs() < 1e-5,
                "gray pixel should be mostly unaffected: got {got}, want {want}"
            );
        }
    }

    #[test]
    fn skin_tone_protected() {
        // Orange-ish skin tone pixel (hue ~25 degrees)
        let skin = ImageBuf::from_data(1, 1, vec![0.6, 0.35, 0.2]).unwrap();
        // Blue pixel (hue ~240 degrees, no protection)
        let blue = ImageBuf::from_data(1, 1, vec![0.2, 0.2, 0.6]).unwrap();

        let params = EditParams {
            vibrance: 80.0,
            ..Default::default()
        };

        let skin_result = Vibrance.process_cpu(skin.clone(), &params).unwrap();
        let blue_result = Vibrance.process_cpu(blue.clone(), &params).unwrap();

        let skin_spread_before = 0.6 - 0.2;
        let skin_spread_after = skin_result.data[0] - skin_result.data[2];
        let skin_change = (skin_spread_after - skin_spread_before).abs();

        let blue_spread_before = 0.6 - 0.2;
        let blue_spread_after = blue_result.data[2] - blue_result.data[0];
        let blue_change = (blue_spread_after - blue_spread_before).abs();

        assert!(
            skin_change < blue_change,
            "skin tones should be boosted less than blue: skin={skin_change} blue={blue_change}"
        );
    }

    #[test]
    fn skin_tone_weight_values() {
        // Pure warm orange: hue ~44 degrees (gamma-encoded), fully protected
        assert!(skin_tone_weight(1.0, 0.5, 0.0) > 0.9);
        // Pure blue: hue ~240 degrees, no protection
        assert!(skin_tone_weight(0.0, 0.0, 1.0) < 0.01);
        // Gray: no protection (chroma too low)
        assert!(skin_tone_weight(0.5, 0.5, 0.5) < 0.01);
        // Pure green: hue ~120 degrees, no protection
        assert!(skin_tone_weight(0.0, 1.0, 0.0) < 0.01);
        // Very red skin (hue ~0 degrees): protected via wrap-around
        assert!(skin_tone_weight(1.0, 0.3, 0.3) > 0.5);
    }

    #[test]
    fn skin_tone_weight_boundary_hues() {
        // Ramp-in start: hue ~350 degrees (very red), should be partial
        let w350 = skin_tone_weight(0.8, 0.2, 0.25);
        assert!(
            w350 > 0.0 && w350 < 1.0,
            "hue ~350 should be in ramp-in: {w350}"
        );

        // Plateau: hue ~30 degrees (classic skin tone), fully protected
        let w30 = skin_tone_weight(0.8, 0.5, 0.2);
        assert!(w30 > 0.8, "hue ~30 should be in plateau: {w30}");

        // Ramp-out: hue ~70 degrees (warm yellow), partial protection
        let w70 = skin_tone_weight(0.7, 0.7, 0.1);
        assert!(
            w70 > 0.0,
            "hue ~70 should still have some protection: {w70}"
        );

        // Extended range: hue ~80 degrees (olive skin), partial protection
        let w80 = skin_tone_weight(0.5, 0.6, 0.1);
        assert!(
            w80 > 0.0,
            "hue ~80 (olive skin) should have partial protection: {w80}"
        );

        // Just outside: hue ~90 degrees (yellow-green), no protection
        let w90 = skin_tone_weight(0.3, 0.6, 0.1);
        assert!(w90 < 0.01, "hue ~90 should have no protection: {w90}");
    }

    #[test]
    fn no_color_inversion_at_extreme_negative() {
        // Saturated blue pixel at vibrance=-100: should desaturate toward gray, not invert.
        let buf = ImageBuf::from_data(1, 1, vec![0.1, 0.1, 0.9]).unwrap();
        let params = EditParams {
            vibrance: -100.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();

        // Blue channel should still be >= other channels (no inversion)
        assert!(
            result.data[2] >= result.data[0] - 1e-6,
            "blue should still be >= red after extreme negative vibrance: B={} R={}",
            result.data[2],
            result.data[0]
        );

        // The blend factor (1 + effect) should never be negative
        let y = 0.2126 * 0.1 + 0.7152 * 0.1 + 0.0722 * 0.9;
        for &v in &result.data {
            assert!(
                (v - y).abs() <= (0.9 - y) + 1e-4,
                "output should be between input and gray, got {v} (y={y})"
            );
        }
    }

    #[test]
    fn hdr_input_handled() {
        // Scene-referred HDR values above 1.0
        let buf = ImageBuf::from_data(1, 1, vec![2.0, 1.5, 0.5]).unwrap();
        let params = EditParams {
            vibrance: 50.0,
            ..Default::default()
        };
        let result = Vibrance.process_cpu(buf, &params).unwrap();
        assert!(
            result.data.iter().all(|v| v.is_finite() && *v >= 0.0),
            "HDR input should produce finite non-negative output: {:?}",
            result.data
        );
    }

    #[test]
    fn negative_100_fully_desaturates_all() {
        // At -100%, the selectivity formula collapses: all pixels fully
        // desaturate to luminance regardless of their initial saturation.
        // This matches SweetFX/ReShade behavior.
        let saturated = ImageBuf::from_data(1, 1, vec![0.2, 0.2, 0.8]).unwrap();
        let desaturated = ImageBuf::from_data(1, 1, vec![0.45, 0.45, 0.55]).unwrap();

        let params = EditParams {
            vibrance: -100.0,
            ..Default::default()
        };

        let sat_result = Vibrance.process_cpu(saturated, &params).unwrap();
        let desat_result = Vibrance.process_cpu(desaturated, &params).unwrap();

        // Both should be at or near grayscale
        let sat_y = 0.2126 * 0.2 + 0.7152 * 0.2 + 0.0722 * 0.8;
        for &v in &sat_result.data {
            assert!(
                (v - sat_y).abs() < 0.01,
                "saturated pixel should be near-gray at -100%: {v} vs {sat_y}"
            );
        }

        let desat_y = 0.2126 * 0.45 + 0.7152 * 0.45 + 0.0722 * 0.55;
        for &v in &desat_result.data {
            assert!(
                (v - desat_y).abs() < 0.01,
                "desaturated pixel should be near-gray at -100%: {v} vs {desat_y}"
            );
        }
    }

    #[test]
    fn skin_tone_weight_dark_skin() {
        // Very dark skin tone: low absolute values, warm hue.
        let w = skin_tone_weight(0.03, 0.015, 0.008);
        assert!(w.is_finite(), "dark skin tone weight should be finite: {w}");
        // Hue should still be in skin range despite low values
        assert!(
            w > 0.0,
            "dark warm-hued pixel should have some skin protection: {w}"
        );
    }

    #[test]
    fn skin_tone_weight_hdr() {
        // HDR skin-tone pixel (values above 1.0)
        let w = skin_tone_weight(2.0, 1.0, 0.5);
        assert!(w.is_finite(), "HDR skin tone weight should be finite: {w}");
        assert!(
            w > 0.0,
            "HDR warm-hued pixel should have skin protection: {w}"
        );
    }

    #[test]
    fn skin_tone_ramps_are_smooth() {
        // Sweep through the ramp-out region (55-85 degrees) and verify
        // no discontinuities. Smoothstep should give C1 continuity.
        let ramp_pixels: &[(f32, f32, f32)] = &[
            (0.8, 0.5, 0.2),   // ~30 degrees (plateau, weight=1.0)
            (0.8, 0.6, 0.2),   // ~40 degrees (plateau)
            (0.8, 0.7, 0.2),   // ~50 degrees (plateau)
            (0.75, 0.7, 0.15), // ~55 degrees (start ramp-out)
            (0.7, 0.7, 0.1),   // ~65 degrees (mid ramp)
            (0.6, 0.7, 0.1),   // ~75 degrees (late ramp)
            (0.5, 0.6, 0.1),   // ~80 degrees (near end)
        ];
        let mut prev = skin_tone_weight(ramp_pixels[0].0, ramp_pixels[0].1, ramp_pixels[0].2);
        for &(r, g, b) in &ramp_pixels[1..] {
            let w = skin_tone_weight(r, g, b);
            let jump = (w - prev).abs();
            assert!(
                jump < 0.5,
                "skin tone ramp should be smooth: prev={prev} curr={w} jump={jump} at ({r},{g},{b})"
            );
            prev = w;
        }
        // Verify the ramp is monotonically decreasing in this region
        let w_55 = skin_tone_weight(0.75, 0.7, 0.15);
        let w_75 = skin_tone_weight(0.6, 0.7, 0.1);
        assert!(
            w_55 > w_75,
            "weight should decrease through ramp-out: w55={w_55} w75={w_75}"
        );
    }
}
