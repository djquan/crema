use crema_core::edit::{EditRecipe, ExposureCentistops};
use crema_image::{
    CandidateFormat, PreviewPixels, PreviewSize, RasterFormat, RawFormat,
    edit_render::render_exposure_srgb8,
    jpeg_export::{JpegExportError, encode_srgb_jpeg},
};
use image::GenericImageView;

fn input(rgba: Vec<u8>) -> PreviewPixels {
    PreviewPixels::new(2, 1, rgba, PreviewSize::new(2).unwrap()).unwrap()
}

fn recipe(centistops: i16) -> EditRecipe {
    EditRecipe::new(ExposureCentistops::new(centistops).unwrap())
}

fn analytic(channel: u8, centistops: i16) -> u8 {
    let encoded = f64::from(channel) / 255.0;
    let linear = if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    let adjusted = (linear * 2.0_f64.powf(f64::from(centistops) / 100.0)).clamp(0.0, 1.0);
    let encoded = if adjusted <= 0.003_130_8 {
        adjusted * 12.92
    } else {
        1.055 * adjusted.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

#[test]
fn exposure_renderer_matches_the_transfer_function_and_preserves_alpha() {
    let source = input(vec![0, 64, 128, 7, 192, 254, 255, 201]);
    let rendered = render_exposure_srgb8(&source, &recipe(100));
    let expected = vec![
        analytic(0, 100),
        analytic(64, 100),
        analytic(128, 100),
        7,
        analytic(192, 100),
        analytic(254, 100),
        analytic(255, 100),
        201,
    ];
    assert_eq!(rendered.rgba8(), expected);

    let darker = render_exposure_srgb8(&source, &recipe(-100));
    assert_eq!(darker.rgba8()[0], analytic(0, -100));
    assert_eq!(darker.rgba8()[5], analytic(254, -100));
    assert_eq!(darker.rgba8()[3], 7);
    assert_eq!(darker.rgba8()[7], 201);
}

#[test]
fn zero_exposure_is_byte_identical() {
    let bytes = vec![0, 1, 2, 3, 127, 128, 254, 255];
    let source = input(bytes.clone());
    let rendered = render_exposure_srgb8(&source, &EditRecipe::default());
    assert_eq!(rendered.rgba8(), bytes);
    assert_eq!(rendered.width(), 2);
    assert_eq!(rendered.height(), 1);
}

#[test]
fn candidate_format_owns_the_dng_exception() {
    assert_eq!(
        CandidateFormat::Raw(RawFormat::Orf).sidecar_naming(),
        crema_core::sidecar::SidecarNaming::ReplaceOriginalExtension
    );
    assert_eq!(
        CandidateFormat::Raw(RawFormat::Dng).sidecar_naming(),
        crema_core::sidecar::SidecarNaming::AppendXmpExtension
    );
    assert_eq!(
        CandidateFormat::Raster(RasterFormat::Jpeg).sidecar_naming(),
        crema_core::sidecar::SidecarNaming::AppendXmpExtension
    );
}

#[test]
fn jpeg_contains_the_generated_standard_srgb_profile() {
    let rendered = render_exposure_srgb8(
        &input(vec![12, 34, 56, 255, 210, 180, 90, 255]),
        &recipe(35),
    );
    let jpeg = encode_srgb_jpeg(&rendered, 92).unwrap();
    let expected = moxcms::ColorProfile::new_srgb().encode().unwrap();

    assert_eq!(extract_icc(&jpeg), expected);
    assert_eq!(image::load_from_memory(&jpeg).unwrap().dimensions(), (2, 1));
}

#[test]
fn jpeg_rejects_non_opaque_pixels() {
    let rendered = render_exposure_srgb8(
        &input(vec![12, 34, 56, 255, 210, 180, 90, 254]),
        &EditRecipe::default(),
    );
    assert!(matches!(
        encode_srgb_jpeg(&rendered, 92),
        Err(JpegExportError::NonOpaqueInput)
    ));
}

fn extract_icc(jpeg: &[u8]) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let mut offset = 2;
    let mut chunks = Vec::new();
    let mut expected_count = None;

    while offset + 4 <= jpeg.len() {
        assert_eq!(jpeg[offset], 0xff);
        let marker = jpeg[offset + 1];
        offset += 2;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = usize::from(u16::from_be_bytes([jpeg[offset], jpeg[offset + 1]]));
        let payload = &jpeg[offset + 2..offset + length];
        offset += length;
        if marker == 0xe2 && payload.starts_with(b"ICC_PROFILE\0") {
            let sequence = payload[12];
            let count = payload[13];
            expected_count = Some(count);
            chunks.push((sequence, payload[14..].to_vec()));
        }
    }

    chunks.sort_by_key(|(sequence, _)| *sequence);
    assert_eq!(chunks.len(), usize::from(expected_count.unwrap()));
    chunks.into_iter().flat_map(|(_, chunk)| chunk).collect()
}
