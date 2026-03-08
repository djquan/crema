use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct SplitTone;

/// Convert HSL (h in degrees, s in 0..1, l=0.5) to linear RGB tint offsets.
fn hsl_to_rgb(hue: f32, sat: f32) -> [f32; 3] {
    if sat <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let h = hue % 360.0;
    let c = sat; // chroma = sat * (1 - |2*0.5 - 1|) = sat
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());

    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let m = 0.5 - c / 2.0;
    [r1 + m, g1 + m, b1 + m]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl ProcessingModule for SplitTone {
    fn name(&self) -> &str {
        "split_tone"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.split_shadow_sat == 0.0 && params.split_highlight_sat == 0.0 {
            return Ok(input);
        }

        let shadow_rgb = hsl_to_rgb(params.split_shadow_hue, params.split_shadow_sat / 100.0);
        let highlight_rgb = hsl_to_rgb(
            params.split_highlight_hue,
            params.split_highlight_sat / 100.0,
        );

        // Balance shifts the crossover point. At 0, crossover is 0.5.
        // Positive balance = more highlight area (crossover moves down).
        // Negative balance = more shadow area (crossover moves up).
        let crossover = 0.5 - params.split_balance / 200.0;

        let shadow_strength = params.split_shadow_sat / 100.0;
        let highlight_strength = params.split_highlight_sat / 100.0;

        for pixel in input.data.chunks_exact_mut(3) {
            let y = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
            let y_clamped = y.clamp(0.0, 1.0);

            // Shadow weight: 1.0 for dark pixels, 0.0 for bright pixels
            let shadow_w = smoothstep(crossover, 0.0, y_clamped) * shadow_strength;
            // Highlight weight: 1.0 for bright pixels, 0.0 for dark pixels
            let highlight_w = smoothstep(crossover, 1.0, y_clamped) * highlight_strength;

            // Tint offset: difference between tint color and neutral gray (0.5)
            pixel[0] = (pixel[0]
                + shadow_w * (shadow_rgb[0] - 0.5)
                + highlight_w * (highlight_rgb[0] - 0.5))
                .max(0.0);
            pixel[1] = (pixel[1]
                + shadow_w * (shadow_rgb[1] - 0.5)
                + highlight_w * (highlight_rgb[1] - 0.5))
                .max(0.0);
            pixel[2] = (pixel[2]
                + shadow_w * (shadow_rgb[2] - 0.5)
                + highlight_w * (highlight_rgb[2] - 0.5))
                .max(0.0);
        }

        Ok(input)
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
        let result = SplitTone.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn shadow_tints_dark_pixels() {
        // Dark pixel should be tinted by shadow hue
        let buf = ImageBuf::from_data(1, 1, vec![0.05, 0.05, 0.05]).unwrap();
        let params = EditParams {
            split_shadow_hue: 0.0, // red
            split_shadow_sat: 80.0,
            ..Default::default()
        };
        let result = SplitTone.process_cpu(buf, &params).unwrap();
        // Red channel should increase relative to blue for a red tint
        assert!(
            result.data[0] > result.data[2],
            "shadow red tint should boost red: r={} b={}",
            result.data[0],
            result.data[2]
        );
    }

    #[test]
    fn highlight_tints_bright_pixels() {
        // Bright pixel should be tinted by highlight hue
        let buf = ImageBuf::from_data(1, 1, vec![0.9, 0.9, 0.9]).unwrap();
        let params = EditParams {
            split_highlight_hue: 240.0, // blue
            split_highlight_sat: 80.0,
            ..Default::default()
        };
        let result = SplitTone.process_cpu(buf, &params).unwrap();
        // Blue channel should increase relative to red for a blue tint
        assert!(
            result.data[2] > result.data[0],
            "highlight blue tint should boost blue: b={} r={}",
            result.data[2],
            result.data[0]
        );
    }

    #[test]
    fn balance_shifts_crossover() {
        // With positive balance, the crossover moves down,
        // so a mid-tone pixel should be more highlight-affected.
        let buf_pos = ImageBuf::from_data(1, 1, vec![0.4, 0.4, 0.4]).unwrap();
        let buf_neg = ImageBuf::from_data(1, 1, vec![0.4, 0.4, 0.4]).unwrap();

        let params_pos = EditParams {
            split_highlight_hue: 0.0,
            split_highlight_sat: 100.0,
            split_balance: 50.0,
            ..Default::default()
        };
        let params_neg = EditParams {
            split_highlight_hue: 0.0,
            split_highlight_sat: 100.0,
            split_balance: -50.0,
            ..Default::default()
        };

        let result_pos = SplitTone.process_cpu(buf_pos, &params_pos).unwrap();
        let result_neg = SplitTone.process_cpu(buf_neg, &params_neg).unwrap();

        // Positive balance should produce more highlight tinting on this mid-tone pixel
        assert!(
            result_pos.data[0] > result_neg.data[0],
            "positive balance should increase highlight tinting: pos_r={} neg_r={}",
            result_pos.data[0],
            result_neg.data[0]
        );
    }

    #[test]
    fn extreme_values_no_panic() {
        for shadow_sat in [0.0, 100.0] {
            for highlight_sat in [0.0, 100.0] {
                for balance in [-100.0, 0.0, 100.0] {
                    let buf = ImageBuf::from_data(2, 2, vec![0.4; 12]).unwrap();
                    let params = EditParams {
                        split_shadow_hue: 359.0,
                        split_shadow_sat: shadow_sat,
                        split_highlight_hue: 180.0,
                        split_highlight_sat: highlight_sat,
                        split_balance: balance,
                        ..Default::default()
                    };
                    let result = SplitTone.process_cpu(buf, &params).unwrap();
                    assert!(result.data.iter().all(|v| v.is_finite()));
                }
            }
        }
    }

    #[test]
    fn preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 5, vec![0.4; 150]).unwrap();
        let params = EditParams {
            split_shadow_hue: 200.0,
            split_shadow_sat: 50.0,
            split_highlight_hue: 40.0,
            split_highlight_sat: 30.0,
            ..Default::default()
        };
        let result = SplitTone.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }
}
