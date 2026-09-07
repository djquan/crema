use std::fmt;
use std::path::Path;

mod decode;
pub mod edit_render;
mod frame;
pub mod jpeg_export;
mod worker;

pub use frame::*;
pub use worker::{CancelToken, DecodeEvent, Decoder, run_worker};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateFormat {
    Raw(RawFormat),
    Raster(RasterFormat),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawFormat {
    Raf,
    Orf,
    Dng,
    Cr2,
    Cr3,
    Nef,
    Arw,
    Rw2,
    Pef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterFormat {
    Jpeg,
    Heic,
    Png,
    Tiff,
}

pub fn classify_candidate(path: &Path) -> Option<CandidateFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "raf" => Some(CandidateFormat::Raw(RawFormat::Raf)),
        "orf" => Some(CandidateFormat::Raw(RawFormat::Orf)),
        "dng" => Some(CandidateFormat::Raw(RawFormat::Dng)),
        "cr2" => Some(CandidateFormat::Raw(RawFormat::Cr2)),
        "cr3" => Some(CandidateFormat::Raw(RawFormat::Cr3)),
        "nef" => Some(CandidateFormat::Raw(RawFormat::Nef)),
        "arw" => Some(CandidateFormat::Raw(RawFormat::Arw)),
        "rw2" => Some(CandidateFormat::Raw(RawFormat::Rw2)),
        "pef" => Some(CandidateFormat::Raw(RawFormat::Pef)),
        "jpg" | "jpeg" => Some(CandidateFormat::Raster(RasterFormat::Jpeg)),
        "heic" | "heif" => Some(CandidateFormat::Raster(RasterFormat::Heic)),
        "png" => Some(CandidateFormat::Raster(RasterFormat::Png)),
        "tif" | "tiff" => Some(CandidateFormat::Raster(RasterFormat::Tiff)),
        _ => None,
    }
}

impl CandidateFormat {
    pub fn sidecar_naming(self) -> crema_core::sidecar::SidecarNaming {
        use crema_core::sidecar::SidecarNaming;

        match self {
            Self::Raw(RawFormat::Dng) | Self::Raster(_) => SidecarNaming::AppendXmpExtension,
            Self::Raw(_) => SidecarNaming::ReplaceOriginalExtension,
        }
    }
}

impl fmt::Display for CandidateFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Raw(RawFormat::Raf) => "RAF",
            Self::Raw(RawFormat::Orf) => "ORF",
            Self::Raw(RawFormat::Dng) => "DNG",
            Self::Raw(RawFormat::Cr2) => "CR2",
            Self::Raw(RawFormat::Cr3) => "CR3",
            Self::Raw(RawFormat::Nef) => "NEF",
            Self::Raw(RawFormat::Arw) => "ARW",
            Self::Raw(RawFormat::Rw2) => "RW2",
            Self::Raw(RawFormat::Pef) => "PEF",
            Self::Raster(RasterFormat::Jpeg) => "JPEG",
            Self::Raster(RasterFormat::Heic) => "HEIC",
            Self::Raster(RasterFormat::Png) => "PNG",
            Self::Raster(RasterFormat::Tiff) => "TIFF",
        };
        formatter.write_str(label)
    }
}

#[cfg(test)]
mod tests {
    use super::{CandidateFormat, RasterFormat, RawFormat, classify_candidate};
    use std::path::Path;

    #[test]
    fn classifies_supported_extensions_and_aliases_without_case_sensitivity() {
        let cases = [
            ("image.RAF", CandidateFormat::Raw(RawFormat::Raf), "RAF"),
            ("image.orf", CandidateFormat::Raw(RawFormat::Orf), "ORF"),
            ("image.DnG", CandidateFormat::Raw(RawFormat::Dng), "DNG"),
            ("image.CR2", CandidateFormat::Raw(RawFormat::Cr2), "CR2"),
            ("image.cr3", CandidateFormat::Raw(RawFormat::Cr3), "CR3"),
            ("image.NeF", CandidateFormat::Raw(RawFormat::Nef), "NEF"),
            ("image.ARW", CandidateFormat::Raw(RawFormat::Arw), "ARW"),
            ("image.rw2", CandidateFormat::Raw(RawFormat::Rw2), "RW2"),
            ("image.Pef", CandidateFormat::Raw(RawFormat::Pef), "PEF"),
            (
                "image.JPG",
                CandidateFormat::Raster(RasterFormat::Jpeg),
                "JPEG",
            ),
            (
                "image.jpeg",
                CandidateFormat::Raster(RasterFormat::Jpeg),
                "JPEG",
            ),
            (
                "image.HeIc",
                CandidateFormat::Raster(RasterFormat::Heic),
                "HEIC",
            ),
            (
                "image.HEIF",
                CandidateFormat::Raster(RasterFormat::Heic),
                "HEIC",
            ),
            (
                "image.png",
                CandidateFormat::Raster(RasterFormat::Png),
                "PNG",
            ),
            (
                "image.TIF",
                CandidateFormat::Raster(RasterFormat::Tiff),
                "TIFF",
            ),
            (
                "image.tiff",
                CandidateFormat::Raster(RasterFormat::Tiff),
                "TIFF",
            ),
        ];

        for (path, expected, label) in cases {
            let actual = classify_candidate(Path::new(path));
            assert_eq!(actual, Some(expected), "path: {path}");
            assert_eq!(expected.to_string(), label);
        }
    }

    #[test]
    fn ignores_missing_and_unsupported_extensions() {
        for path in ["image", ".jpg", "image.raw", "image.jpg.tmp"] {
            assert_eq!(classify_candidate(Path::new(path)), None, "path: {path}");
        }
    }
}
