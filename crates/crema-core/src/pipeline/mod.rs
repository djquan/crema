pub mod auto_enhance;
pub mod module;
pub mod modules;

use anyhow::Result;
use tracing::debug;

use crate::image_buf::{EditParams, ImageBuf};
use module::ProcessingModule;

/// Processing pipeline that chains modules together.
///
/// ```text
/// RAW -> Demosaic -> White Balance -> Exposure -> Crop -> Tone Map -> Display
/// ```
///
/// Each module operates on a linear f32 ImageBuf. When a parameter changes,
/// only the affected module and everything downstream re-executes.
pub struct Pipeline {
    modules: Vec<Box<dyn ProcessingModule>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            modules: vec![
                Box::new(modules::WhiteBalance),
                Box::new(modules::Exposure),
                Box::new(modules::ToneCurve),
                Box::new(modules::Vibrance),
                Box::new(modules::Saturation),
                Box::new(modules::Crop),
            ],
        }
    }

    /// Run the full CPU pipeline on an input image with the given edit params.
    pub fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        let mut current = input;
        for module in &self.modules {
            debug!(module = module.name(), "processing");
            current = module.process_cpu(current, params)?;
        }
        Ok(current)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> ImageBuf {
        // 4x4 image with known values (linear 0.5 across all channels)
        ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap()
    }

    #[test]
    fn default_params_are_identity() {
        let pipeline = Pipeline::new();
        let input = test_image();
        let params = EditParams::default();
        let expected = input.data.clone();
        let output = pipeline.process_cpu(input, &params).unwrap();
        assert_eq!(output.width, 4);
        assert_eq!(output.height, 4);
        assert_eq!(output.data, expected);
    }

    #[test]
    fn exposure_then_crop() {
        let pipeline = Pipeline::new();
        let input = test_image();
        let params = EditParams {
            exposure: 1.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let output = pipeline.process_cpu(input, &params).unwrap();
        assert_eq!(output.width, 2);
        assert_eq!(output.height, 2);
        for &v in &output.data {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn wb_and_exposure_combine() {
        let pipeline = Pipeline::new();
        let input = ImageBuf::from_data(1, 1, vec![0.5, 0.5, 0.5]).unwrap();
        let params = EditParams {
            exposure: 1.0,
            wb_temp: 7000.0,
            ..Default::default()
        };
        let output = pipeline.process_cpu(input, &params).unwrap();
        assert!(output.data[0] > output.data[2]);
    }

    #[test]
    fn pipeline_preserves_dimensions_without_crop() {
        let pipeline = Pipeline::new();
        let input = ImageBuf::from_data(100, 50, vec![0.3; 100 * 50 * 3]).unwrap();
        let params = EditParams {
            exposure: 2.0,
            wb_temp: 4000.0,
            ..Default::default()
        };
        let output = pipeline.process_cpu(input, &params).unwrap();
        assert_eq!(output.width, 100);
        assert_eq!(output.height, 50);
    }

    #[test]
    fn module_ordering() {
        let pipeline = Pipeline::new();
        let names: Vec<&str> = pipeline.modules.iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            vec![
                "white_balance",
                "exposure",
                "tone_curve",
                "vibrance",
                "saturation",
                "crop",
            ]
        );
    }
}
