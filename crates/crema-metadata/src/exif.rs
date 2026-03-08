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
            width: get_u32(&exif, Tag::PixelXDimension).or_else(|| get_u32(&exif, Tag::ImageWidth)),
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

        let camera = match (&self.camera_make, &self.camera_model) {
            (Some(make), Some(model)) => Some(format!("{make} {model}")),
            (Some(make), None) => Some(make.clone()),
            (None, Some(model)) => Some(model.clone()),
            (None, None) => None,
        };
        if let Some(camera) = camera {
            lines.push(("Camera".into(), camera));
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
    exif.get_field(tag, In::PRIMARY).and_then(|f| {
        let from_raw = match f.value {
            exif::Value::Ascii(ref values) => values
                .iter()
                .filter_map(|v| std::str::from_utf8(v).ok().map(ToOwned::to_owned))
                .map(normalize_exif_string)
                .find(|s| s.is_some())
                .flatten(),
            exif::Value::Undefined(ref values, _) => {
                normalize_exif_string(String::from_utf8_lossy(values).to_string())
            }
            _ => None,
        };
        from_raw.or_else(|| normalize_exif_string(f.display_value().to_string()))
    })
}

fn normalize_exif_string(s: String) -> Option<String> {
    let mut value = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value = &value[1..value.len() - 1];
    }
    let value = value.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn get_u32(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)
        .and_then(|f| match f.value {
            exif::Value::Short(ref v) => v.first().map(|&x| x as u32),
            exif::Value::Long(ref v) => v.first().copied(),
            _ => f.display_value().to_string().trim().parse().ok(),
        })
}

fn get_rational_f64(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    exif.get_field(tag, In::PRIMARY)
        .and_then(|f| match f.value {
            exif::Value::Rational(ref v) => v.first().and_then(|r| {
                if r.denom == 0 {
                    None
                } else {
                    Some(r.num as f64 / r.denom as f64)
                }
            }),
            _ => f.display_value().to_string().trim().parse().ok(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ---------------------------------------------------------------
    // Helpers for building minimal JPEG files with EXIF segments
    // ---------------------------------------------------------------

    fn write_u16_be(buf: &mut Vec<u8>, val: u16) {
        buf.extend_from_slice(&val.to_be_bytes());
    }

    fn write_u32_be(buf: &mut Vec<u8>, val: u32) {
        buf.extend_from_slice(&val.to_be_bytes());
    }

    type IfdEntry = (u16, u16, u32, Vec<u8>);

    struct TiffBuilder {
        ifd0_entries: Vec<IfdEntry>,
        exif_entries: Vec<IfdEntry>,
    }

    impl TiffBuilder {
        fn new() -> Self {
            Self {
                ifd0_entries: Vec::new(),
                exif_entries: Vec::new(),
            }
        }

        fn add_ascii(&mut self, tag: u16, value: &str) {
            let bytes: Vec<u8> = value.bytes().chain(std::iter::once(0)).collect();
            let count = bytes.len() as u32;
            self.push_entry(tag, 2, count, bytes);
        }

        fn add_short(&mut self, tag: u16, value: u16) {
            let mut data = Vec::new();
            write_u16_be(&mut data, value);
            self.push_entry(tag, 3, 1, data);
        }

        fn add_long(&mut self, tag: u16, value: u32) {
            let mut data = Vec::new();
            write_u32_be(&mut data, value);
            self.push_entry(tag, 4, 1, data);
        }

        fn add_rational(&mut self, tag: u16, num: u32, denom: u32) {
            let mut data = Vec::new();
            write_u32_be(&mut data, num);
            write_u32_be(&mut data, denom);
            self.push_entry(tag, 5, 1, data);
        }

        fn push_entry(&mut self, tag: u16, dtype: u16, count: u32, data: Vec<u8>) {
            if Self::is_exif_ifd_tag(tag) {
                self.exif_entries.push((tag, dtype, count, data));
            } else {
                self.ifd0_entries.push((tag, dtype, count, data));
            }
        }

        fn is_exif_ifd_tag(tag: u16) -> bool {
            matches!(
                tag,
                0x829D  // FNumber
                | 0x8827 // PhotographicSensitivity (ISO)
                | 0x829A // ExposureTime
                | 0x9003 // DateTimeOriginal
                | 0x920A // FocalLength
                | 0xA002 // PixelXDimension
                | 0xA003 // PixelYDimension
                | 0xA434 // LensModel
            )
        }

        fn build_jpeg(&self) -> Vec<u8> {
            let tiff_body = self.build_tiff();
            let exif_prefix = b"Exif\x00\x00";
            let app1_payload_len = 2 + exif_prefix.len() + tiff_body.len();

            let mut jpeg = Vec::new();
            jpeg.extend_from_slice(&[0xFF, 0xD8]);
            jpeg.extend_from_slice(&[0xFF, 0xE1]);
            write_u16_be(&mut jpeg, app1_payload_len as u16);
            jpeg.extend_from_slice(exif_prefix);
            jpeg.extend_from_slice(&tiff_body);
            jpeg.extend_from_slice(&[0xFF, 0xD9]);
            jpeg
        }

        fn build_tiff(&self) -> Vec<u8> {
            let has_exif_ifd = !self.exif_entries.is_empty();

            // IFD0 entries, plus an ExifIFDPointer entry if we have sub-IFD tags.
            // The pointer value is a placeholder; we'll patch it after computing sizes.
            let ifd0_count = self.ifd0_entries.len() + if has_exif_ifd { 1 } else { 0 };

            let mut buf = Vec::new();
            // TIFF header
            buf.extend_from_slice(b"MM");
            write_u16_be(&mut buf, 42);
            write_u32_be(&mut buf, 8);

            write_u16_be(&mut buf, ifd0_count as u16);

            let data_offset_base = 8 + 2 + (ifd0_count * 12) + 4;
            let mut overflow = Vec::new();

            // Write IFD0 entries (sorted by tag for spec compliance)
            let mut all_ifd0: Vec<IfdEntry> = self.ifd0_entries.clone();
            if has_exif_ifd {
                // ExifIFDPointer (0x8769), type LONG, count 1, value = placeholder
                all_ifd0.push((0x8769, 4, 1, vec![0, 0, 0, 0]));
            }
            all_ifd0.sort_by_key(|(tag, _, _, _)| *tag);

            let mut exif_ptr_buf_pos = None;

            for (tag, dtype, count, data) in &all_ifd0 {
                write_u16_be(&mut buf, *tag);
                write_u16_be(&mut buf, *dtype);
                write_u32_be(&mut buf, *count);

                if *tag == 0x8769 {
                    // Record position so we can patch with the real offset
                    exif_ptr_buf_pos = Some(buf.len());
                }

                if data.len() <= 4 {
                    let mut inline = [0u8; 4];
                    inline[..data.len()].copy_from_slice(data);
                    buf.extend_from_slice(&inline);
                } else {
                    let offset = data_offset_base + overflow.len();
                    write_u32_be(&mut buf, offset as u32);
                    overflow.extend_from_slice(data);
                }
            }

            // Next IFD offset = 0
            write_u32_be(&mut buf, 0);
            buf.extend_from_slice(&overflow);

            // Build EXIF sub-IFD if needed
            if has_exif_ifd {
                let exif_ifd_offset = buf.len();

                // Patch the ExifIFDPointer value in IFD0
                if let Some(pos) = exif_ptr_buf_pos {
                    let bytes = (exif_ifd_offset as u32).to_be_bytes();
                    buf[pos..pos + 4].copy_from_slice(&bytes);
                }

                let mut sorted_exif = self.exif_entries.clone();
                sorted_exif.sort_by_key(|(tag, _, _, _)| *tag);

                write_u16_be(&mut buf, sorted_exif.len() as u16);

                let exif_data_base = exif_ifd_offset + 2 + (sorted_exif.len() * 12) + 4;
                let mut exif_overflow = Vec::new();

                for (tag, dtype, count, data) in &sorted_exif {
                    write_u16_be(&mut buf, *tag);
                    write_u16_be(&mut buf, *dtype);
                    write_u32_be(&mut buf, *count);

                    if data.len() <= 4 {
                        let mut inline = [0u8; 4];
                        inline[..data.len()].copy_from_slice(data);
                        buf.extend_from_slice(&inline);
                    } else {
                        let offset = exif_data_base + exif_overflow.len();
                        write_u32_be(&mut buf, offset as u32);
                        exif_overflow.extend_from_slice(data);
                    }
                }

                write_u32_be(&mut buf, 0);
                buf.extend_from_slice(&exif_overflow);
            }

            buf
        }
    }

    fn write_temp_file(data: &[u8], suffix: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(suffix).tempfile().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        f
    }

    // ---------------------------------------------------------------
    // summary_lines() tests
    // ---------------------------------------------------------------

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
    fn summary_line_order_matches_display_priority() {
        let data = ExifData {
            camera_make: Some("Sony".into()),
            camera_model: Some("A7IV".into()),
            lens: Some("FE 24-70mm".into()),
            focal_length: Some(35.0),
            aperture: Some(4.0),
            shutter_speed: Some("1/125".into()),
            iso: Some(400),
            width: Some(7008),
            height: Some(4672),
            date_taken: Some("2025-01-20 14:00:00".into()),
            orientation: Some(1),
        };
        let lines = data.summary_lines();
        let labels: Vec<&str> = lines.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Camera",
                "Lens",
                "Focal Length",
                "Aperture",
                "Shutter",
                "ISO",
                "Resolution",
                "Date"
            ]
        );
    }

    #[test]
    fn summary_camera_make_without_model() {
        let data = ExifData {
            camera_make: Some("Fujifilm".into()),
            camera_model: None,
            ..Default::default()
        };
        let lines = data.summary_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, "Camera");
        assert_eq!(lines[0].1, "Fujifilm");
    }

    #[test]
    fn summary_camera_model_without_make() {
        let data = ExifData {
            camera_make: None,
            camera_model: Some("X-T5".into()),
            ..Default::default()
        };
        let lines = data.summary_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, "Camera");
        assert_eq!(lines[0].1, "X-T5");
    }

    #[test]
    fn summary_resolution_requires_both_dimensions() {
        let only_width = ExifData {
            width: Some(4000),
            height: None,
            ..Default::default()
        };
        assert!(only_width.summary_lines().is_empty());

        let only_height = ExifData {
            width: None,
            height: Some(3000),
            ..Default::default()
        };
        assert!(only_height.summary_lines().is_empty());

        let both = ExifData {
            width: Some(4000),
            height: Some(3000),
            ..Default::default()
        };
        let lines = both.summary_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].1, "4000 x 3000");
    }

    #[test]
    fn summary_focal_length_rounds_to_integer() {
        let data = ExifData {
            focal_length: Some(23.7),
            ..Default::default()
        };
        let lines = data.summary_lines();
        assert_eq!(lines[0].1, "24mm");
    }

    #[test]
    fn summary_aperture_one_decimal() {
        let data = ExifData {
            aperture: Some(5.6),
            ..Default::default()
        };
        let lines = data.summary_lines();
        assert_eq!(lines[0].1, "f/5.6");

        let whole = ExifData {
            aperture: Some(8.0),
            ..Default::default()
        };
        let lines = whole.summary_lines();
        assert_eq!(lines[0].1, "f/8.0");
    }

    // ---------------------------------------------------------------
    // from_file() with invalid/non-image files
    // ---------------------------------------------------------------

    #[test]
    fn exif_from_nonexistent_file() {
        let result = ExifData::from_file(std::path::Path::new("/nonexistent/photo.jpg"));
        assert!(result.is_err());
    }

    #[test]
    fn exif_from_plain_text_file() {
        let f = write_temp_file(b"this is not a jpeg", ".txt");
        let result = ExifData::from_file(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn exif_from_empty_file() {
        let f = write_temp_file(b"", ".jpg");
        let result = ExifData::from_file(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn exif_from_truncated_jpeg() {
        // Just SOI marker, nothing else
        let f = write_temp_file(&[0xFF, 0xD8], ".jpg");
        let result = ExifData::from_file(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn exif_from_jpeg_without_exif_segment() {
        // Valid JPEG structure but no APP1/EXIF segment
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI
        let f = write_temp_file(&jpeg, ".jpg");
        let result = ExifData::from_file(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn exif_from_random_bytes() {
        let f = write_temp_file(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03], ".jpg");
        let result = ExifData::from_file(f.path());
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // from_file() with crafted EXIF data
    // ---------------------------------------------------------------

    #[test]
    fn exif_parses_ascii_make_and_model() {
        let mut tb = TiffBuilder::new();
        // Tag 0x010F = Make, Tag 0x0110 = Model
        tb.add_ascii(0x010F, "TestCorp");
        tb.add_ascii(0x0110, "Phantom X1");

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.camera_make.as_deref(), Some("TestCorp"));
        assert_eq!(data.camera_model.as_deref(), Some("Phantom X1"));
    }

    #[test]
    fn exif_parses_short_orientation() {
        let mut tb = TiffBuilder::new();
        // Tag 0x0112 = Orientation
        tb.add_short(0x0112, 6);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.orientation, Some(6));
    }

    #[test]
    fn exif_parses_long_image_dimensions() {
        let mut tb = TiffBuilder::new();
        // Tag 0x0100 = ImageWidth, Tag 0x0101 = ImageLength
        tb.add_long(0x0100, 6000);
        tb.add_long(0x0101, 4000);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.width, Some(6000));
        assert_eq!(data.height, Some(4000));
    }

    #[test]
    fn exif_parses_rational_focal_length() {
        let mut tb = TiffBuilder::new();
        // Tag 0x920A = FocalLength (rational: 50/1)
        tb.add_rational(0x920A, 50, 1);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.focal_length, Some(50.0));
    }

    #[test]
    fn exif_rational_zero_denominator_returns_none() {
        let mut tb = TiffBuilder::new();
        // FocalLength with denom=0 should be rejected
        tb.add_rational(0x920A, 50, 0);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.focal_length, None);
    }

    #[test]
    fn exif_rational_fractional_aperture() {
        let mut tb = TiffBuilder::new();
        // Tag 0x829D = FNumber (rational: 28/10 = 2.8)
        tb.add_rational(0x829D, 28, 10);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        let aperture = data.aperture.unwrap();
        assert!((aperture - 2.8).abs() < 0.001);
    }

    #[test]
    fn exif_short_image_width_parsed_as_u32() {
        let mut tb = TiffBuilder::new();
        // Use Short type for ImageWidth instead of Long
        tb.add_short(0x0100, 1920);

        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();
        assert_eq!(data.width, Some(1920));
    }

    #[test]
    fn exif_all_orientation_values() {
        for orientation in 1u16..=8 {
            let mut tb = TiffBuilder::new();
            tb.add_short(0x0112, orientation);

            let jpeg = tb.build_jpeg();
            let f = write_temp_file(&jpeg, ".jpg");
            let data = ExifData::from_file(f.path()).unwrap();
            assert_eq!(data.orientation, Some(orientation as u32));
        }
    }

    #[test]
    fn exif_empty_ifd_returns_all_none() {
        let tb = TiffBuilder::new();
        let jpeg = tb.build_jpeg();
        let f = write_temp_file(&jpeg, ".jpg");
        let data = ExifData::from_file(f.path()).unwrap();

        assert!(data.width.is_none());
        assert!(data.height.is_none());
        assert!(data.camera_make.is_none());
        assert!(data.camera_model.is_none());
        assert!(data.lens.is_none());
        assert!(data.focal_length.is_none());
        assert!(data.aperture.is_none());
        assert!(data.shutter_speed.is_none());
        assert!(data.iso.is_none());
        assert!(data.date_taken.is_none());
        assert!(data.orientation.is_none());
    }

    // ---------------------------------------------------------------
    // Serialization tests
    // ---------------------------------------------------------------

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

    #[test]
    fn serialization_all_none_fields() {
        let data = ExifData::default();
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: ExifData = serde_json::from_str(&json).unwrap();
        assert!(deserialized.camera_make.is_none());
        assert!(deserialized.focal_length.is_none());
        assert!(deserialized.iso.is_none());
        assert!(deserialized.width.is_none());
        assert!(deserialized.height.is_none());
        assert!(deserialized.orientation.is_none());
    }

    #[test]
    fn serialization_all_fields_populated() {
        let data = ExifData {
            width: Some(8192),
            height: Some(5464),
            camera_make: Some("Hasselblad".into()),
            camera_model: Some("X2D 100C".into()),
            lens: Some("XCD 55V".into()),
            focal_length: Some(55.0),
            aperture: Some(2.5),
            shutter_speed: Some("1/500".into()),
            iso: Some(64),
            date_taken: Some("2025-06-01 08:15:30".into()),
            orientation: Some(1),
        };
        let json = serde_json::to_string(&data).unwrap();
        let rt: ExifData = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.width, Some(8192));
        assert_eq!(rt.height, Some(5464));
        assert_eq!(rt.camera_make.as_deref(), Some("Hasselblad"));
        assert_eq!(rt.camera_model.as_deref(), Some("X2D 100C"));
        assert_eq!(rt.lens.as_deref(), Some("XCD 55V"));
        assert_eq!(rt.focal_length, Some(55.0));
        assert_eq!(rt.aperture, Some(2.5));
        assert_eq!(rt.shutter_speed.as_deref(), Some("1/500"));
        assert_eq!(rt.iso, Some(64));
        assert_eq!(rt.date_taken.as_deref(), Some("2025-06-01 08:15:30"));
        assert_eq!(rt.orientation, Some(1));
    }

    #[test]
    fn deserialization_from_json_with_unknown_fields() {
        let json = r#"{"camera_make":"Leica","unknown_field":42}"#;
        let result: Result<ExifData, _> = serde_json::from_str(json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().camera_make.as_deref(), Some("Leica"));
    }

    #[test]
    fn default_exifdata_is_all_none() {
        let data = ExifData::default();
        assert!(data.width.is_none());
        assert!(data.height.is_none());
        assert!(data.camera_make.is_none());
        assert!(data.camera_model.is_none());
        assert!(data.lens.is_none());
        assert!(data.focal_length.is_none());
        assert!(data.aperture.is_none());
        assert!(data.shutter_speed.is_none());
        assert!(data.iso.is_none());
        assert!(data.date_taken.is_none());
        assert!(data.orientation.is_none());
    }

    #[test]
    fn exifdata_clone_is_independent() {
        let data = ExifData {
            camera_make: Some("Pentax".into()),
            iso: Some(1600),
            ..Default::default()
        };
        let cloned = data.clone();
        assert_eq!(cloned.camera_make, data.camera_make);
        assert_eq!(cloned.iso, data.iso);
    }

    #[test]
    fn exifdata_debug_format() {
        let data = ExifData::default();
        let debug = format!("{:?}", data);
        assert!(debug.contains("ExifData"));
    }
}
