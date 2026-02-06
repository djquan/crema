use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};
use crate::pipeline::module::ProcessingModule;

pub struct Crop;

impl ProcessingModule for Crop {
    fn name(&self) -> &str {
        "crop"
    }

    fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf> {
        if params.crop_x == 0.0
            && params.crop_y == 0.0
            && params.crop_w == 1.0
            && params.crop_h == 1.0
        {
            return Ok(input);
        }

        let src_x =
            ((params.crop_x * input.width as f32) as u32).min(input.width.saturating_sub(1));
        let src_y =
            ((params.crop_y * input.height as f32) as u32).min(input.height.saturating_sub(1));
        let remaining_w = input.width.saturating_sub(src_x);
        let remaining_h = input.height.saturating_sub(src_y);
        let dst_w = (params.crop_w * input.width as f32).max(1.0) as u32;
        let dst_h = (params.crop_h * input.height as f32).max(1.0) as u32;
        let dst_w = dst_w.min(remaining_w).max(1);
        let dst_h = dst_h.min(remaining_h).max(1);

        let mut data = Vec::with_capacity((dst_w * dst_h * 3) as usize);

        for row in src_y..(src_y + dst_h) {
            let row_start = ((row * input.width + src_x) * 3) as usize;
            let row_end = row_start + (dst_w * 3) as usize;
            data.extend_from_slice(&input.data[row_start..row_end]);
        }

        ImageBuf::from_data(dst_w, dst_h, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_crop_is_identity() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams::default();
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
    }

    #[test]
    fn crop_reduces_dimensions() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
    }

    #[test]
    fn crop_at_boundary_does_not_panic() {
        let buf = ImageBuf::from_data(4, 4, vec![0.5; 48]).unwrap();
        let params = EditParams {
            crop_x: 1.0,
            crop_y: 1.0,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert!(result.width >= 1);
        assert!(result.height >= 1);
    }

    #[test]
    fn crop_with_offset() {
        let mut data = Vec::with_capacity(48);
        for i in 0..16 {
            data.push(i as f32);
            data.push(0.0);
            data.push(0.0);
        }
        let buf = ImageBuf::from_data(4, 4, data).unwrap();

        let params = EditParams {
            crop_x: 0.5,
            crop_y: 0.5,
            crop_w: 0.5,
            crop_h: 0.5,
            ..Default::default()
        };
        let result = Crop.process_cpu(buf, &params).unwrap();
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        assert_eq!(result.data[0], 10.0);
    }
}
