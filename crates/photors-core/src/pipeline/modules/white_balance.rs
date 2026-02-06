use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct WhiteBalance;

impl ProcessingModule for WhiteBalance {
    fn name(&self) -> &str {
        "white_balance"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        let (r_mult, g_mult, b_mult) = temp_tint_to_multipliers(params.wb_temp, params.wb_tint);

        if (r_mult - 1.0).abs() < 1e-6
            && (g_mult - 1.0).abs() < 1e-6
            && (b_mult - 1.0).abs() < 1e-6
        {
            return Ok(input);
        }

        for pixel in input.data.chunks_exact_mut(3) {
            pixel[0] *= r_mult;
            pixel[1] *= g_mult;
            pixel[2] *= b_mult;
        }

        Ok(input)
    }
}

/// Convert color temperature (Kelvin) and tint to per-channel multipliers.
///
/// This is a simplified approximation. The neutral point is D55 (5500K, tint=0).
/// Temperature shifts the blue-yellow axis, tint shifts green-magenta.
fn temp_tint_to_multipliers(temp: f32, tint: f32) -> (f32, f32, f32) {
    let temp_shift = (temp - 5500.0) / 5500.0;

    let r_mult = 1.0 + temp_shift * 0.3;
    let b_mult = 1.0 - temp_shift * 0.3;

    let g_mult = 1.0 + tint * 0.01;

    (r_mult.max(0.1), g_mult.max(0.1), b_mult.max(0.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_temp_is_identity() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams::default(); // 5500K, tint=0
        let result = WhiteBalance.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn warm_temp_boosts_red() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let params = EditParams {
            wb_temp: 7000.0,
            ..Default::default()
        };
        let result = WhiteBalance.process_cpu(buf, &params).unwrap();
        assert!(result.data[0] > 0.5); // red boosted
        assert!(result.data[2] < 0.5); // blue reduced
    }

    #[test]
    fn cool_temp_boosts_blue() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let params = EditParams {
            wb_temp: 3500.0,
            ..Default::default()
        };
        let result = WhiteBalance.process_cpu(buf, &params).unwrap();
        assert!(result.data[0] < 0.5); // red reduced
        assert!(result.data[2] > 0.5); // blue boosted
    }

    #[test]
    fn multipliers_never_zero() {
        let (r, g, b) = temp_tint_to_multipliers(1000.0, -100.0);
        assert!(r >= 0.1);
        assert!(g >= 0.1);
        assert!(b >= 0.1);
    }
}
