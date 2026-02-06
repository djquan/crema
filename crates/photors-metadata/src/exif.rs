use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use exif::{In, Tag};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExifData {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub iso: Option<u32>,
    pub date_taken: Option<String>,
    pub orientation: Option<u32>,
}

impl ExifData {
    pub fn from_file(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let exif_reader = exif::Reader::new();
        let exif = exif_reader
            .read_from_container(&mut reader)
            .with_context(|| format!("read EXIF from {}", path.display()))?;

        Ok(Self {
            width: get_u32(&exif, Tag::PixelXDimension)
                .or_else(|| get_u32(&exif, Tag::ImageWidth)),
            height: get_u32(&exif, Tag::PixelYDimension)
                .or_else(|| get_u32(&exif, Tag::ImageLength)),
            camera_make: get_string(&exif, Tag::Make),
            camera_model: get_string(&exif, Tag::Model),
            lens: get_string(&exif, Tag::LensModel),
            focal_length: get_rational_f64(&exif, Tag::FocalLength),
            aperture: get_rational_f64(&exif, Tag::FNumber),
            shutter_speed: get_string(&exif, Tag::ExposureTime),
            iso: get_u32(&exif, Tag::PhotographicSensitivity),
            date_taken: get_string(&exif, Tag::DateTimeOriginal),
            orientation: get_u32(&exif, Tag::Orientation),
        })
    }

    pub fn summary_lines(&self) -> Vec<(String, String)> {
        let mut lines = Vec::new();

        if let Some(ref make) = self.camera_make {
            lines.push(("Camera".into(), format!("{} {}", make, self.camera_model.as_deref().unwrap_or(""))));
        }
        if let Some(ref lens) = self.lens {
            lines.push(("Lens".into(), lens.clone()));
        }
        if let Some(fl) = self.focal_length {
            lines.push(("Focal Length".into(), format!("{fl:.0}mm")));
        }
        if let Some(ap) = self.aperture {
            lines.push(("Aperture".into(), format!("f/{ap:.1}")));
        }
        if let Some(ref ss) = self.shutter_speed {
            lines.push(("Shutter".into(), ss.clone()));
        }
        if let Some(iso) = self.iso {
            lines.push(("ISO".into(), iso.to_string()));
        }
        if let (Some(w), Some(h)) = (self.width, self.height) {
            lines.push(("Resolution".into(), format!("{w} x {h}")));
        }
        if let Some(ref date) = self.date_taken {
            lines.push(("Date".into(), date.clone()));
        }

        lines
    }
}

fn get_string(exif: &exif::Exif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY)
        .map(|f| f.display_value().to_string().trim().to_string())
        .filter(|s| !s.is_empty())
}

fn get_u32(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY).and_then(|f| match f.value {
        exif::Value::Short(ref v) => v.first().map(|&x| x as u32),
        exif::Value::Long(ref v) => v.first().copied(),
        _ => f.display_value().to_string().trim().parse().ok(),
    })
}

fn get_rational_f64(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    exif.get_field(tag, In::PRIMARY).and_then(|f| match f.value {
        exif::Value::Rational(ref v) => v.first().map(|r| r.num as f64 / r.denom as f64),
        _ => f.display_value().to_string().trim().parse().ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_exif_summary() {
        let data = ExifData::default();
        let lines = data.summary_lines();
        assert!(lines.is_empty());
    }

    #[test]
    fn full_exif_summary() {
        let data = ExifData {
            camera_make: Some("Canon".into()),
            camera_model: Some("EOS R5".into()),
            lens: Some("RF 50mm F1.2L".into()),
            focal_length: Some(50.0),
            aperture: Some(1.2),
            shutter_speed: Some("1/200".into()),
            iso: Some(100),
            width: Some(8192),
            height: Some(5464),
            date_taken: Some("2024-03-15 10:30:00".into()),
            orientation: None,
        };
        let lines = data.summary_lines();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0].0, "Camera");
        assert!(lines[0].1.contains("Canon"));
        assert!(lines[0].1.contains("EOS R5"));
        assert_eq!(lines[1].0, "Lens");
        assert_eq!(lines[2].0, "Focal Length");
        assert!(lines[2].1.contains("50mm"));
        assert_eq!(lines[3].0, "Aperture");
        assert!(lines[3].1.contains("1.2"));
    }

    #[test]
    fn partial_exif_summary() {
        let data = ExifData {
            iso: Some(800),
            aperture: Some(2.8),
            ..Default::default()
        };
        let lines = data.summary_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "Aperture");
        assert_eq!(lines[1].0, "ISO");
    }

    #[test]
    fn exif_from_nonexistent_file() {
        let result = ExifData::from_file(std::path::Path::new("/nonexistent/photo.jpg"));
        assert!(result.is_err());
    }

    #[test]
    fn exif_serialization_roundtrip() {
        let data = ExifData {
            camera_make: Some("Nikon".into()),
            focal_length: Some(85.0),
            iso: Some(200),
            ..Default::default()
        };
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: ExifData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.camera_make.as_deref(), Some("Nikon"));
        assert_eq!(deserialized.focal_length, Some(85.0));
        assert_eq!(deserialized.iso, Some(200));
    }
}
