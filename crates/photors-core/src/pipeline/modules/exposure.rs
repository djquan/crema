use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Exposure;

impl ProcessingModule for Exposure {
    fn name(&self) -> &str {
        "exposure"
    }

    fn process_cpu(&self, mut input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.exposure == 0.0 {
            return Ok(input);
        }

        let multiplier = 2.0_f32.powf(params.exposure);
        for v in &mut input.data {
            *v *= multiplier;
        }
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_exposure_is_identity() {
        let buf = ImageBuf::from_data(2, 1, vec![0.5; 6]).unwrap();
        let expected = buf.data.clone();
        let params = EditParams::default();
        let result = Exposure.process_cpu(buf, &params).unwrap();
        assert_eq!(result.data, expected);
    }

    #[test]
    fn positive_exposure_brightens() {
        let buf = ImageBuf::from_data(1, 1, vec![0.25, 0.25, 0.25]).unwrap();
        let params = EditParams {
            exposure: 1.0,
            ..Default::default()
        };
        let result = Exposure.process_cpu(buf, &params).unwrap();
        assert!((result.data[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn negative_exposure_darkens() {
        let buf = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let params = EditParams {
            exposure: -1.0,
            ..Default::default()
        };
        let result = Exposure.process_cpu(buf, &params).unwrap();
        assert!((result.data[0] - 0.25).abs() < 1e-6);
    }
}
