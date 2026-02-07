use anyhow::Result;

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
            let max_ch = pixel[0].max(pixel[1]).max(pixel[2]);
            let min_ch = pixel[0].min(pixel[1]).min(pixel[2]);
            let sat = (max_ch - min_ch) / (max_ch + 1e-6);

            // Selective saturation (SweetFX/ReShade convention):
            //   positive -> targets low-sat pixels (1 - sat)
            //   negative -> targets high-sat pixels (1 + sat)
            let mut effect = strength * (1.0 - sign * sat);

            // Skin tone protection: reduce effect for warm hues to prevent
            // portraits from looking sunburned (boost) or sickly (cut).
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

/// Returns 0.0-1.0 indicating how much this pixel looks like a skin tone.
///
/// Computes HSV hue from gamma-encoded (perceptual) RGB so that hue angles
/// match standard HSV definitions. Skin tones cluster around hue 0-55
/// degrees (red through warm yellow); the range wraps around 360/0 to
/// catch very red skin tones at hue ~355-360.
fn skin_tone_weight(r: f32, g: f32, b: f32) -> f32 {
    let rg = r.max(0.0).powf(1.0 / 2.2);
    let gg = g.max(0.0).powf(1.0 / 2.2);
    let bg = b.max(0.0).powf(1.0 / 2.2);

    let max_ch = rg.max(gg).max(bg);
    let min_ch = rg.min(gg).min(bg);
    let chroma = max_ch - min_ch;
    if chroma < 1e-6 {
        return 0.0;
    }

    let hue = if (max_ch - rg).abs() < 1e-6 {
        60.0 * (((gg - bg) / chroma) % 6.0)
    } else if (max_ch - gg).abs() < 1e-6 {
        60.0 * ((bg - rg) / chroma + 2.0)
    } else {
        60.0 * ((rg - gg) / chroma + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };

    // Skin tone range: 350-70 degrees (wraps around 0/360).
    // Ramp in: 350-5, plateau: 5-55, ramp out: 55-70.
    if hue >= 350.0 || hue <= 70.0 {
        let h = if hue >= 350.0 { hue - 360.0 } else { hue };
        if h < 5.0 {
            ((h + 10.0) / 15.0).clamp(0.0, 1.0)
        } else if h > 55.0 {
            ((70.0 - h) / 15.0).clamp(0.0, 1.0)
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
        let saturated = ImageBuf::from_data(1, 1, vec![0.8, 0.0, 0.0]).unwrap();
        let desaturated = ImageBuf::from_data(1, 1, vec![0.5, 0.45, 0.4]).unwrap();

        let params = EditParams {
            vibrance: 50.0,
            ..Default::default()
        };

        let sat_result = Vibrance.process_cpu(saturated, &params).unwrap();
        let desat_result = Vibrance.process_cpu(desaturated, &params).unwrap();

        let sat_y = 0.2126 * 0.8;
        let sat_delta = (sat_result.data[0] - sat_y).abs() - (0.8 - sat_y).abs();

        let desat_y = 0.2126 * 0.5 + 0.7152 * 0.45 + 0.0722 * 0.4;
        let desat_delta = (desat_result.data[0] - desat_y).abs() - (0.5 - desat_y).abs();

        assert!(
            desat_delta > sat_delta,
            "desaturated pixel should be boosted more: desat_delta={desat_delta} sat_delta={sat_delta}"
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
}
