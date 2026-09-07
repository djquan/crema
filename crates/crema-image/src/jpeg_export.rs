use crate::edit_render::RenderedSdr;
use image::ExtendedColorType;
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::error::UnsupportedError;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum JpegExportError {
    InvalidQuality(u8),
    NonOpaqueInput,
    Profile(moxcms::CmsError),
    ProfileEmbedding(UnsupportedError),
    Encode(image::ImageError),
}

impl fmt::Display for JpegExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuality(quality) => {
                write!(formatter, "JPEG quality {quality} is outside 1..=100")
            }
            Self::NonOpaqueInput => formatter.write_str("JPEG export requires opaque pixels"),
            Self::Profile(error) => write!(formatter, "cannot generate the sRGB profile: {error}"),
            Self::ProfileEmbedding(error) => {
                write!(formatter, "cannot embed the sRGB profile: {error}")
            }
            Self::Encode(error) => write!(formatter, "cannot encode JPEG: {error}"),
        }
    }
}

impl Error for JpegExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::ProfileEmbedding(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::InvalidQuality(_) | Self::NonOpaqueInput => None,
        }
    }
}

pub fn encode_srgb_jpeg(rendered: &RenderedSdr, quality: u8) -> Result<Vec<u8>, JpegExportError> {
    if !(1..=100).contains(&quality) {
        return Err(JpegExportError::InvalidQuality(quality));
    }
    if rendered
        .rgba8()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] != 255)
    {
        return Err(JpegExportError::NonOpaqueInput);
    }
    let mut rgb = Vec::with_capacity(rendered.rgba8().len() / 4 * 3);
    for pixel in rendered.rgba8().as_chunks::<4>().0 {
        rgb.extend_from_slice(&pixel[..3]);
    }
    let profile = moxcms::ColorProfile::new_srgb()
        .encode()
        .map_err(JpegExportError::Profile)?;
    let mut output = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut output, quality);
    encoder
        .set_icc_profile(profile)
        .map_err(JpegExportError::ProfileEmbedding)?;
    encoder
        .encode(
            &rgb,
            rendered.width(),
            rendered.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(JpegExportError::Encode)?;
    Ok(output)
}
