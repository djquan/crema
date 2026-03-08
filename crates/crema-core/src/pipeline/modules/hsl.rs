use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Hsl;

impl ProcessingModule for Hsl {
    fn name(&self) -> &str {
        "hsl"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.hsl_hue == 0.0 && params.hsl_saturation == 0.0 && params.hsl_lightness == 0.0 {
            return Ok(input);
        }

        let do_hue = params.hsl_hue != 0.0;
        let do_sat = params.hsl_saturation != 0.0;
        let do_light = params.hsl_lightness != 0.0;

        // Precompute hue rotation matrix (Rodrigues' rotation around luminance axis)
        let hue_matrix = if do_hue {
            Some(hue_rotation_matrix(params.hsl_hue))
        } else {
            None
        };

        let sat_blend = 1.0 + params.hsl_saturation / 100.0;
        let light_scale = 1.0 + params.hsl_lightness / 100.0;

        for pixel in input.data.chunks_exact_mut(3) {
            let (mut r, mut g, mut b) = (pixel[0], pixel[1], pixel[2]);

            // Hue rotation
            if let Some(m) = &hue_matrix {
                let nr = m[0] * r + m[1] * g + m[2] * b;
                let ng = m[3] * r + m[4] * g + m[5] * b;
                let nb = m[6] * r + m[7] * g + m[8] * b;
                r = nr;
                g = ng;
                b = nb;
            }

            // Saturation (blend toward luminance)
            if do_sat {
                let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = y + sat_blend * (r - y);
                g = y + sat_blend * (g - y);
                b = y + sat_blend * (b - y);
            }

            // Lightness (scale luminance, redistribute preserving ratios)
            if do_light {
                let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                if y > 0.0 {
                    let target_y = y * light_scale;
                    let scale = target_y / y;
                    r *= scale;
                    g *= scale;
                    b *= scale;
                }
            }

            pixel[0] = r.max(0.0);
            pixel[1] = g.max(0.0);
            pixel[2] = b.max(0.0);
        }

        Ok(input)
    }
}

/// Build a 3x3 rotation matrix that rotates RGB colors around the luminance axis.
///
/// Uses Rodrigues' rotation formula with the luminance vector (0.2126, 0.7152, 0.0722)
/// normalized as the rotation axis.
fn hue_rotation_matrix(degrees: f32) -> [f32; 9] {
    let angle = degrees.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    // Luminance axis (normalized)
    let len = (0.2126_f32.powi(2) + 0.7152_f32.powi(2) + 0.0722_f32.powi(2)).sqrt();
    let (kx, ky, kz) = (0.2126 / len, 0.7152 / len, 0.0722 / len);

    // Rodrigues: R = I*cos(a) + (1-cos(a))*K*Kt + sin(a)*K_cross
    let one_minus_cos = 1.0 - cos_a;
    [
        cos_a + one_minus_cos * kx * kx,
        one_minus_cos * kx * ky - sin_a * kz,
        one_minus_cos * kx * kz + sin_a * ky,
        one_minus_cos * ky * kx + sin_a * kz,
        cos_a + one_minus_cos * ky * ky,
        one_minus_cos * ky * kz - sin_a * kx,
        one_minus_cos * kz * kx - sin_a * ky,
        one_minus_cos * kz * ky + sin_a * kx,
        cos_a + one_minus_cos * kz * kz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_noop() {
        let buf = ImageBuf::from_data(2, 2, vec![0.5; 12]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams::default();
        let result = Hsl.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn hue_rotation_360_identity() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let params = EditParams {
            hsl_hue: 360.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        assert!((result.data[0] - 0.8).abs() < 1e-4);
        assert!((result.data[1] - 0.3).abs() < 1e-4);
        assert!((result.data[2] - 0.1).abs() < 1e-4);
    }

    #[test]
    fn hue_rotation_changes_color() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.2, 0.1]).unwrap();
        let params = EditParams {
            hsl_hue: 90.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        // Should be noticeably different
        let diff = (result.data[0] - 0.8).abs()
            + (result.data[1] - 0.2).abs()
            + (result.data[2] - 0.1).abs();
        assert!(diff > 0.1, "90 degree hue rotation should change color");
    }

    #[test]
    fn hue_rotation_matrix_preserves_luminance() {
        // Verify the rotation matrix preserves luminance (before clamping)
        let m = hue_rotation_matrix(90.0);
        let (r, g, b) = (0.8, 0.3, 0.1);
        let nr = m[0] * r + m[1] * g + m[2] * b;
        let ng = m[3] * r + m[4] * g + m[5] * b;
        let nb = m[6] * r + m[7] * g + m[8] * b;
        let y_before = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let y_after = 0.2126 * nr + 0.7152 * ng + 0.0722 * nb;
        assert!(
            (y_before - y_after).abs() < 1e-4,
            "rotation matrix should preserve luminance: before={y_before}, after={y_after}"
        );
    }

    #[test]
    fn saturation_positive() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let params = EditParams {
            hsl_saturation: 50.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        let spread_before = 0.8 - 0.1;
        let spread_after = result.data[0] - result.data[2];
        assert!(spread_after > spread_before);
    }

    #[test]
    fn lightness_positive() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.3, 0.2]).unwrap();
        let y_before = 0.2126 * 0.5 + 0.7152 * 0.3 + 0.0722 * 0.2;
        let params = EditParams {
            hsl_lightness: 50.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        let y_after = 0.2126 * result.data[0] + 0.7152 * result.data[1] + 0.0722 * result.data[2];
        assert!(y_after > y_before);
    }

    #[test]
    fn lightness_negative() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.3, 0.2]).unwrap();
        let y_before = 0.2126 * 0.5 + 0.7152 * 0.3 + 0.0722 * 0.2;
        let params = EditParams {
            hsl_lightness: -50.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        let y_after = 0.2126 * result.data[0] + 0.7152 * result.data[1] + 0.0722 * result.data[2];
        assert!(y_after < y_before);
    }

    #[test]
    fn all_three_combined() {
        let buf = ImageBuf::from_data(
            2,
            2,
            vec![0.5, 0.3, 0.1, 0.7, 0.2, 0.4, 0.1, 0.8, 0.3, 0.4, 0.4, 0.6],
        )
        .unwrap();
        let params = EditParams {
            hsl_hue: 45.0,
            hsl_saturation: 30.0,
            hsl_lightness: 20.0,
            ..Default::default()
        };
        let result = Hsl.process_cpu(buf, &params).unwrap();
        assert!(result.data.iter().all(|v| v.is_finite() && *v >= 0.0));
    }

    #[test]
    fn extreme_values_no_panic() {
        for hue in [-180.0, 0.0, 180.0] {
            for sat in [-100.0, 0.0, 100.0] {
                for light in [-100.0, 0.0, 100.0] {
                    let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.3, 0.1]).unwrap();
                    let params = EditParams {
                        hsl_hue: hue,
                        hsl_saturation: sat,
                        hsl_lightness: light,
                        ..Default::default()
                    };
                    let result = Hsl.process_cpu(buf, &params).unwrap();
                    assert!(
                        result.data.iter().all(|v| v.is_finite() && *v >= 0.0),
                        "failed at hue={hue}, sat={sat}, light={light}: {:?}",
                        result.data
                    );
                }
            }
        }
    }
}
