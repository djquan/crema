use std::{fmt, num::NonZeroU32, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownReason {
    DecoderDoesNotExpose,
    MetadataMissing,
    MetadataInvalid,
    NotMeasured,
}

impl fmt::Display for UnknownReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DecoderDoesNotExpose => "decoder-does-not-expose",
            Self::MetadataMissing => "metadata-missing",
            Self::MetadataInvalid => "metadata-invalid",
            Self::NotMeasured => "not-measured",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Fact<T> {
    Known(T),
    Unknown(UnknownReason),
}

impl<T: fmt::Display> fmt::Display for Fact<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(value) => value.fmt(f),
            Self::Unknown(reason) => write!(f, "unknown:{reason}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSize(NonZeroU32);

impl PreviewSize {
    pub fn new(edge: u32) -> Result<Self, DecodeError> {
        match NonZeroU32::new(edge) {
            Some(edge) if edge.get() <= 4096 => Ok(Self(edge)),
            _ => Err(DecodeError::limit("preview edge must be 1..=4096")),
        }
    }

    pub fn edge(self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug)]
pub struct PreviewPixels {
    width: NonZeroU32,
    height: NonZeroU32,
    rgba8: Vec<u8>,
}

impl PreviewPixels {
    pub fn new(
        width: u32,
        height: u32,
        rgba8: Vec<u8>,
        size: PreviewSize,
    ) -> Result<Self, DecodeError> {
        let width = NonZeroU32::new(width).ok_or_else(|| DecodeError::protocol("zero width"))?;
        let height = NonZeroU32::new(height).ok_or_else(|| DecodeError::protocol("zero height"))?;
        let expected = u64::from(width.get())
            .checked_mul(u64::from(height.get()))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| DecodeError::protocol("preview size overflow"))?;
        if width.get().max(height.get()) > size.edge() || expected != rgba8.len() as u64 {
            return Err(DecodeError::protocol(
                "invalid preview dimensions or byte length",
            ));
        }
        Ok(Self {
            width,
            height,
            rgba8,
        })
    }

    pub fn width(&self) -> u32 {
        self.width.get()
    }
    pub fn height(&self) -> u32 {
        self.height.get()
    }
    pub fn dimensions_usize(&self) -> [usize; 2] {
        [self.width.get() as usize, self.height.get() as usize]
    }
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    JpegDecode,
    RawlerDevelopment,
    HeifDecode,
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::JpegDecode => "jpeg-decode",
            Self::RawlerDevelopment => "rawler-development",
            Self::HeifDecode => "heif-decode",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icc {
    Absent,
    PresentNotApplied,
}

impl fmt::Display for Icc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Absent => "absent",
            Self::PresentNotApplied => "present-not-applied",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Orientation {
    Exif(u8),
    ContainerApplied,
    Unknown(UnknownReason),
}

impl fmt::Display for Orientation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exif(value) => write!(f, "exif:{value}:applied"),
            Self::ContainerApplied => {
                f.write_str("container-applied;source-transform-unknown:decoder-does-not-expose")
            }
            Self::Unknown(reason) => write!(f, "unknown:{reason}:unchanged"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceMetadata {
    pub dimensions: [u32; 2],
    pub decoded_dimensions: [u32; 2],
    pub source_bits: Fact<u8>,
    pub decoded_bits: u8,
    pub orientation: Orientation,
    pub icc: Fact<Icc>,
    pub nclx: Option<[u16; 4]>,
    pub camera_make: String,
    pub camera_model: String,
    pub limitations: String,
}

#[derive(Debug)]
pub struct DecodeResult {
    pub preview: PreviewPixels,
    pub metadata: SourceMetadata,
    pub provenance: Provenance,
}

#[derive(Debug)]
pub enum DecodeOutcome {
    Decoded(DecodeResult),
    Unsupported(String),
    Failed(DecodeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureClass {
    Io,
    InvalidInput,
    LimitExceeded,
    Timeout,
    WorkerExited,
    Protocol,
    Codec,
    Cancelled,
}

impl fmt::Display for FailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "io",
            Self::InvalidInput => "invalid-input",
            Self::LimitExceeded => "limit-exceeded",
            Self::Timeout => "timeout",
            Self::WorkerExited => "worker-exited",
            Self::Protocol => "protocol",
            Self::Codec => "codec",
            Self::Cancelled => "cancelled",
        })
    }
}

#[derive(Debug)]
pub struct DecodeError {
    pub class: FailureClass,
    pub message: String,
}

impl DecodeError {
    pub(crate) fn new(class: FailureClass, message: impl ToString) -> Self {
        Self {
            class,
            message: message.to_string(),
        }
    }
    pub(crate) fn limit(message: impl ToString) -> Self {
        Self::new(FailureClass::LimitExceeded, message)
    }
    pub(crate) fn protocol(message: impl ToString) -> Self {
        Self::new(FailureClass::Protocol, message)
    }
    pub(crate) fn codec(message: impl ToString) -> Self {
        Self::new(FailureClass::Codec, message)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}
impl std::error::Error for DecodeError {}
impl From<std::io::Error> for DecodeError {
    fn from(error: std::io::Error) -> Self {
        Self::new(FailureClass::Io, error)
    }
}

#[derive(Clone, Debug)]
pub struct DecodeLimits {
    pub input_bytes: u64,
    pub source_pixels: u64,
    pub timeout: Duration,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            input_bytes: 256 * 1024 * 1024,
            source_pixels: 100_000_000,
            timeout: Duration::from_secs(60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_reject_invalid_shape_and_preview_limits() {
        let size = PreviewSize::new(3).unwrap();
        assert!(PreviewSize::new(0).is_err());
        assert!(PreviewSize::new(4097).is_err());
        for (w, h, len) in [
            (0, 1, 0),
            (1, 0, 0),
            (2, 3, 23),
            (4, 1, 16),
            (u32::MAX, u32::MAX, 0),
        ] {
            assert!(PreviewPixels::new(w, h, vec![0; len], size).is_err());
        }
        assert_eq!(
            PreviewPixels::new(2, 3, vec![0; 24], size)
                .unwrap()
                .dimensions_usize(),
            [2, 3]
        );
    }
}
