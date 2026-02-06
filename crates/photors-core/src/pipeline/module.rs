use anyhow::Result;

use crate::image_buf::{EditParams, ImageBuf};

/// A single step in the processing pipeline.
pub trait ProcessingModule: Send + Sync {
    fn name(&self) -> &str;
    fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf>;
}
