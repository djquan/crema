use crate::*;
use image::{
    DynamicImage, ImageDecoder, codecs::jpeg::JpegDecoder,
    metadata::Orientation as ImageOrientation,
};
use moxcms::{ColorProfile, Layout};
use std::{io::Cursor, sync::Arc};

pub(crate) fn check_dimensions(width: u32, height: u32, limit: u64) -> Result<(), DecodeError> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > limit {
        return Err(DecodeError::limit("source dimensions exceed pixel limit"));
    }
    Ok(())
}

fn finish(
    image: DynamicImage,
    size: PreviewSize,
    metadata: SourceMetadata,
    provenance: Provenance,
) -> Result<DecodeResult, DecodeError> {
    let image = if image.width().max(image.height()) > size.edge() {
        image.thumbnail(size.edge(), size.edge())
    } else {
        image
    }
    .into_rgba8();
    Ok(DecodeResult {
        preview: PreviewPixels::new(image.width(), image.height(), image.into_raw(), size)?,
        metadata,
        provenance,
    })
}

pub(crate) fn jpeg(
    bytes: &[u8],
    size: PreviewSize,
    limits: &DecodeLimits,
) -> Result<DecodeResult, DecodeError> {
    let mut decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(DecodeError::codec)?;
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height, limits.source_pixels)?;
    let mut codec_limits = image::Limits::default();
    codec_limits.max_alloc = Some(limits.source_pixels.saturating_mul(8));
    decoder
        .set_limits(codec_limits)
        .map_err(DecodeError::codec)?;
    let exif = decoder.exif_metadata();
    let orientation = match exif {
        Ok(Some(ref exif)) => ImageOrientation::from_exif_chunk(exif)
            .map(|orientation| Orientation::Exif(orientation.to_exif()))
            .unwrap_or(Orientation::Unknown(UnknownReason::DecoderDoesNotExpose)),
        Ok(None) => Orientation::Unknown(UnknownReason::MetadataMissing),
        Err(_) => Orientation::Unknown(UnknownReason::MetadataInvalid),
    };
    let embedded_icc = decoder
        .icc_profile()
        .map_err(|error| DecodeError::unsupported_color(format!("invalid JPEG ICC: {error}")))?;
    let decoded_bits = (decoder.color_type().bits_per_pixel()
        / u16::from(decoder.color_type().channel_count())) as u8;
    let mut image = DynamicImage::from_decoder(decoder).map_err(DecodeError::codec)?;
    let icc = match embedded_icc {
        Some(profile) => {
            image = apply_icc_to_srgb(image, &profile)?;
            Fact::Known(Icc::AppliedToSrgb)
        }
        None => Fact::Known(Icc::Absent),
    };
    apply_orientation(&mut image, &orientation);
    let limitations = if icc == Fact::Known(Icc::AppliedToSrgb) {
        "embedded-icc-applied-to-srgb;display-profile-not-managed"
    } else {
        "sdr-srgb-assumption;display-profile-not-managed"
    };
    let metadata = SourceMetadata {
        dimensions: [width, height],
        decoded_dimensions: [image.width(), image.height()],
        source_bits: Fact::Unknown(UnknownReason::DecoderDoesNotExpose),
        decoded_bits,
        orientation,
        icc,
        nclx: None,
        camera_make: String::new(),
        camera_model: String::new(),
        limitations: limitations.into(),
    };
    finish(image, size, metadata, Provenance::JpegDecode)
}

fn apply_icc_to_srgb(image: DynamicImage, profile: &[u8]) -> Result<DynamicImage, DecodeError> {
    let source = ColorProfile::new_from_slice(profile)
        .map_err(|error| DecodeError::unsupported_color(format!("invalid JPEG ICC: {error}")))?;
    let destination = ColorProfile::new_srgb();
    let transform = source
        .create_transform_8bit(Layout::Rgba, &destination, Layout::Rgba, Default::default())
        .map_err(|error| {
            DecodeError::unsupported_color(format!("unsupported JPEG ICC transform: {error}"))
        })?;
    let source = image.into_rgba8();
    let mut output = vec![0; source.len()];
    transform
        .transform(source.as_raw(), &mut output)
        .map_err(|error| {
            DecodeError::unsupported_color(format!("JPEG ICC transform failed: {error}"))
        })?;
    let converted = image::RgbaImage::from_raw(source.width(), source.height(), output)
        .ok_or_else(|| DecodeError::protocol("invalid transformed JPEG pixel buffer"))?;
    Ok(DynamicImage::ImageRgba8(converted))
}

fn apply_orientation(image: &mut DynamicImage, orientation: &Orientation) {
    if let Orientation::Exif(value) = orientation {
        image.apply_orientation(
            ImageOrientation::from_exif(*value).expect("validated EXIF orientation"),
        );
    }
}

pub(crate) fn raw(
    bytes: Vec<u8>,
    size: PreviewSize,
    limits: &DecodeLimits,
) -> Result<DecodeResult, DecodeError> {
    let source = rawler::rawsource::RawSource::new_from_shared_vec(Arc::new(bytes));
    let decoder = rawler::get_decoder(&source).map_err(DecodeError::codec)?;
    let params = rawler::decoders::RawDecodeParams::default();
    let metadata = decoder
        .raw_metadata(&source, &params)
        .map_err(DecodeError::codec)?;
    let raw = decoder
        .raw_image(&source, &params, false)
        .map_err(DecodeError::codec)?;
    let width = u32::try_from(raw.width).map_err(DecodeError::limit)?;
    let height = u32::try_from(raw.height).map_err(DecodeError::limit)?;
    check_dimensions(width, height, limits.source_pixels)?;
    let orientation = match metadata.exif.orientation {
        Some(value @ 1..=8) => Orientation::Exif(value as u8),
        Some(_) => Orientation::Unknown(UnknownReason::MetadataInvalid),
        None => Orientation::Unknown(UnknownReason::MetadataMissing),
    };
    let mut limitations = "experimental-rawler-baseline;display-profile-not-managed".to_owned();
    if raw.color_matrix.is_empty() {
        limitations.push_str(";missing-camera-calibration");
    }
    let mut image = rawler::imgop::develop::RawDevelop::default()
        .develop_intermediate(&raw)
        .map_err(DecodeError::codec)?
        .to_dynamic_image()
        .ok_or_else(|| DecodeError::codec("invalid developed dimensions"))?;
    apply_orientation(&mut image, &orientation);
    let facts = SourceMetadata {
        dimensions: [width, height],
        decoded_dimensions: [image.width(), image.height()],
        source_bits: Fact::Unknown(UnknownReason::DecoderDoesNotExpose),
        decoded_bits: u8::try_from(raw.bps).map_err(DecodeError::codec)?,
        orientation,
        icc: Fact::Unknown(UnknownReason::DecoderDoesNotExpose),
        nclx: None,
        camera_make: metadata.make,
        camera_model: metadata.model,
        limitations,
    };
    finish(image, size, facts, Provenance::RawlerDevelopment)
}

pub(crate) fn heif(
    bytes: &[u8],
    size: PreviewSize,
    limits: &DecodeLimits,
) -> Result<DecodeResult, DecodeError> {
    let decoded = heif_oxide::decode_bytes(bytes).map_err(DecodeError::codec)?;
    check_dimensions(decoded.width, decoded.height, limits.source_pixels)?;
    let nclx = decoded.color.nclx.as_ref().map(|color| {
        [
            color.primaries,
            color.transfer,
            color.matrix,
            u16::from(color.full_range),
        ]
    });
    validate_heif_color(decoded.bit_depth() as u8, decoded.color.icc_present, nclx)?;
    let limitations = "experimental-heif-srgb-conversion;display-profile-not-managed".to_owned();
    let metadata = SourceMetadata {
        dimensions: [decoded.width, decoded.height],
        decoded_dimensions: [decoded.width, decoded.height],
        source_bits: Fact::Unknown(UnknownReason::DecoderDoesNotExpose),
        decoded_bits: decoded.bit_depth() as u8,
        orientation: Orientation::ContainerApplied,
        icc: Fact::Known(if decoded.color.icc_present {
            Icc::PresentNotApplied
        } else {
            Icc::Absent
        }),
        nclx,
        camera_make: String::new(),
        camera_model: String::new(),
        limitations,
    };
    let image = image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.to_rgba8())
        .ok_or_else(|| DecodeError::codec("invalid HEIF pixel buffer"))?;
    finish(
        DynamicImage::ImageRgba8(image),
        size,
        metadata,
        Provenance::HeifDecode,
    )
}

fn validate_heif_color(
    bit_depth: u8,
    icc_present: bool,
    nclx: Option<[u16; 4]>,
) -> Result<(), DecodeError> {
    if bit_depth != 8 {
        return Err(DecodeError::unsupported_color(format!(
            "HEIC {bit_depth}-bit color requires a precision-preserving conversion"
        )));
    }
    if icc_present {
        return Err(DecodeError::unsupported_color(
            "HEIC ICC is present but the decoder does not expose it for conversion",
        ));
    }
    let [_, transfer, _, _] = nclx.ok_or_else(|| {
        DecodeError::unsupported_color("HEIC transfer is not declared by the decoder")
    })?;
    if !matches!(transfer, 1 | 13) {
        return Err(DecodeError::unsupported_color(format!(
            "HEIC transfer {transfer} has no verified SDR conversion"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn encoded_jpeg() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode(&[90; 2 * 3 * 3], 2, 3, image::ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn encoded_jpeg_with_icc(profile: Vec<u8>) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        encoder.set_icc_profile(profile).unwrap();
        encoder
            .encode(&[90; 2 * 3 * 3], 2, 3, image::ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_real_jpeg_and_keeps_metadata_uncertainty() {
        let bytes = encoded_jpeg();
        let decoded = jpeg(
            &bytes,
            PreviewSize::new(320).unwrap(),
            &DecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(decoded.metadata.dimensions, [2, 3]);
        assert_eq!(decoded.preview.rgba8().len(), 24);
        assert_eq!(
            decoded.metadata.orientation,
            Orientation::Unknown(UnknownReason::MetadataMissing)
        );
        assert_eq!(decoded.metadata.icc, Fact::Known(Icc::Absent));
        assert_eq!(
            decoded.metadata.source_bits,
            Fact::Unknown(UnknownReason::DecoderDoesNotExpose)
        );
        assert!(
            jpeg(
                b"not a JPEG",
                PreviewSize::new(320).unwrap(),
                &DecodeLimits::default()
            )
            .is_err()
        );
        let limits = DecodeLimits {
            source_pixels: 5,
            ..DecodeLimits::default()
        };
        assert_eq!(
            jpeg(&bytes, PreviewSize::new(320).unwrap(), &limits)
                .unwrap_err()
                .class,
            FailureClass::LimitExceeded
        );
    }

    #[test]
    fn applies_all_eight_exif_orientations_to_real_pixel_pattern() {
        let expected = [
            vec![1, 2, 3, 4, 5, 6],
            vec![2, 1, 4, 3, 6, 5],
            vec![6, 5, 4, 3, 2, 1],
            vec![5, 6, 3, 4, 1, 2],
            vec![1, 3, 5, 2, 4, 6],
            vec![5, 3, 1, 6, 4, 2],
            vec![6, 4, 2, 5, 3, 1],
            vec![2, 4, 6, 1, 3, 5],
        ];
        for value in 1..=8 {
            let pixels = image::GrayImage::from_raw(2, 3, vec![1, 2, 3, 4, 5, 6]).unwrap();
            let mut image = DynamicImage::ImageLuma8(pixels);
            apply_orientation(&mut image, &Orientation::Exif(value));
            assert_eq!(image.into_luma8().into_raw(), expected[value as usize - 1]);
        }
    }

    #[test]
    fn jpeg_applies_embedded_icc_and_exif_orientation_before_preview() {
        let profile = moxcms::ColorProfile::new_adobe_rgb().encode().unwrap();
        let encoded = encoded_jpeg_with_icc(profile);
        let mut bytes = encoded[..2].to_vec();
        let mut exif =
            b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        bytes.extend_from_slice(&[0xff, 0xe1]);
        bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        bytes.append(&mut exif);
        bytes.extend_from_slice(&encoded[2..]);
        let result = jpeg(
            &bytes,
            PreviewSize::new(320).unwrap(),
            &DecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(result.metadata.orientation, Orientation::Exif(6));
        assert_eq!(result.metadata.dimensions, [2, 3]);
        assert_eq!(result.metadata.decoded_dimensions, [3, 2]);
        assert_eq!(result.preview.dimensions_usize(), [3, 2]);
        assert_eq!(result.metadata.icc, Fact::Known(Icc::AppliedToSrgb));
        assert!(
            !result
                .metadata
                .limitations
                .contains("embedded-icc-not-applied")
        );
    }

    #[test]
    fn jpeg_refuses_an_invalid_embedded_icc_instead_of_guessing_srgb() {
        let error = jpeg(
            &encoded_jpeg_with_icc(b"not an ICC profile".to_vec()),
            PreviewSize::new(320).unwrap(),
            &DecodeLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error.class, FailureClass::UnsupportedColor);
    }

    #[test]
    fn heif_color_gate_refuses_precision_and_transfers_we_do_not_render() {
        assert!(validate_heif_color(8, false, Some([1, 1, 1, 1])).is_ok());
        for unsupported in [
            validate_heif_color(10, false, Some([1, 1, 1, 1])),
            validate_heif_color(8, true, Some([1, 1, 1, 1])),
            validate_heif_color(8, false, None),
            validate_heif_color(8, false, Some([9, 16, 9, 0])),
            validate_heif_color(8, false, Some([9, 18, 9, 0])),
            validate_heif_color(8, false, Some([1, 2, 1, 0])),
        ] {
            assert_eq!(
                unsupported.unwrap_err().class,
                FailureClass::UnsupportedColor
            );
        }
    }
}
