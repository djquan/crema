use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Saturation;

impl ProcessingModule for Saturation {
    fn name(&self) -> &str {
        "saturation"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.saturation == 0.0 {
            return Ok(input);
        }

        let strength = params.saturation / 100.0;
        let blend = 1.0 + strength;
        for pixel in input.data.chunks_exact_mut(3) {
            let y = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
            pixel[0] = (y + blend * (pixel[0] - y)).max(0.0);
            pixel[1] = (y + blend * (pixel[1] - y)).max(0.0);
            pixel[2] = (y + blend * (pixel[2] - y)).max(0.0);
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
        let result = Saturation.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn positive_increases_saturation() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let params = EditParams {
            saturation: 50.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        let spread_before = 0.8 - 0.1;
        let spread_after = result.data[0] - result.data[2];
        assert!(spread_after > spread_before);
    }

    #[test]
    fn negative_decreases_saturation() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let params = EditParams {
            saturation: -50.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        let spread_before = 0.8 - 0.1;
        let spread_after = result.data[0] - result.data[2];
        assert!(spread_after < spread_before);
    }

    #[test]
    fn minus_100_produces_grayscale() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let params = EditParams {
            saturation: -100.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        let y = 0.2126 * 0.8 + 0.7152 * 0.3 + 0.0722 * 0.1;
        for &v in &result.data {
            assert!(
                (v - y).abs() < 1e-6,
                "at -100 saturation all channels should equal Y={y}, got {v}"
            );
        }
    }

    #[test]
    fn gray_pixel_stays_gray() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        for sat in [-100.0, -50.0, 50.0, 100.0] {
            let params = EditParams {
                saturation: sat,
                ..Default::default()
            };
            let result = Saturation.process_cpu(buf.clone(), &params).unwrap();
            for &v in &result.data {
                assert!(
                    (v - 0.5).abs() < 1e-6,
                    "gray pixel should stay gray at saturation={sat}, got {v}"
                );
            }
        }
    }

    #[test]
    fn clamps_at_zero() {
        let buf = ImageBuf::from_data(1, 1, vec![0.9, 0.0, 0.0]).unwrap();
        let params = EditParams {
            saturation: -200.0, // extreme oversaturation negative
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        for &v in &result.data {
            assert!(v >= 0.0, "values should be >= 0, got {v}");
        }
    }

    #[test]
    fn extreme_values_no_panic() {
        let buf = ImageBuf::from_data(2, 2, vec![0.4; 12]).unwrap();
        for sat in [-100.0, 100.0] {
            let params = EditParams {
                saturation: sat,
                ..Default::default()
            };
            let result = Saturation.process_cpu(buf.clone(), &params).unwrap();
            assert!(result.data.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn preserves_dimensions() {
        let buf = ImageBuf::from_data(10, 5, vec![0.4; 150]).unwrap();
        let params = EditParams {
            saturation: 30.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn hdr_input_handled() {
        let buf = ImageBuf::from_data(1, 1, vec![2.0, 1.5, 0.5]).unwrap();
        let params = EditParams {
            saturation: 50.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        assert!(
            result.data.iter().all(|v| v.is_finite() && *v >= 0.0),
            "HDR input should produce finite non-negative output: {:?}",
            result.data
        );
    }

    #[test]
    fn positive_100_doubles_deviation() {
        let buf = ImageBuf::from_data(1, 1, vec![0.8, 0.3, 0.1]).unwrap();
        let y = 0.2126 * 0.8 + 0.7152 * 0.3 + 0.0722 * 0.1;
        let params = EditParams {
            saturation: 100.0,
            ..Default::default()
        };
        let result = Saturation.process_cpu(buf, &params).unwrap();
        // At +100%, blend=2.0, so deviation from Y doubles
        let expected_r = y + 2.0 * (0.8 - y);
        assert!(
            (result.data[0] - expected_r).abs() < 1e-5,
            "saturation +100 should double deviation: got {} expected {}",
            result.data[0],
            expected_r
        );
    }
}
