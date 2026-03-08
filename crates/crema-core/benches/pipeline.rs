use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use crema_core::image_buf::{EditParams, ImageBuf};
use crema_core::pipeline::Pipeline;
use crema_core::pipeline::module::ProcessingModule;
use crema_core::pipeline::modules::{
    Crop, Exposure, Saturation, ToneCurve, Vibrance, WhiteBalance,
};

fn synthetic_image(width: u32, height: u32) -> ImageBuf {
    let n = (width * height * 3) as usize;
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        // Vary values across the image so branch predictors and identity
        // early-returns don't mask real work. Values stay in [0.05, 0.95]
        // to be realistic linear-light scene data.
        data.push(0.05 + 0.9 * ((i % 997) as f32 / 996.0));
    }
    ImageBuf::from_data(width, height, data).unwrap()
}

fn active_params() -> EditParams {
    EditParams {
        exposure: 0.7,
        wb_temp: 6200.0,
        wb_tint: 8.0,
        contrast: 25.0,
        highlights: -30.0,
        shadows: 20.0,
        blacks: -5.0,
        vibrance: 15.0,
        saturation: 10.0,
        crop_x: 0.05,
        crop_y: 0.05,
        crop_w: 0.9,
        crop_h: 0.9,
    }
}

// ── Full pipeline ────────────────────────────────────────────────────────

fn bench_full_pipeline(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();
    let pipeline = Pipeline::new();

    c.bench_function("pipeline_full_2048x2048", |b| {
        b.iter(|| pipeline.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

// ── Individual modules ───────────────────────────────────────────────────

fn bench_white_balance(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_white_balance_2048", |b| {
        b.iter(|| WhiteBalance.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

fn bench_exposure(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_exposure_2048", |b| {
        b.iter(|| Exposure.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

fn bench_tone_curve(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_tone_curve_2048", |b| {
        b.iter(|| ToneCurve.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

fn bench_vibrance(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_vibrance_2048", |b| {
        b.iter(|| Vibrance.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

fn bench_saturation(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_saturation_2048", |b| {
        b.iter(|| Saturation.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

fn bench_crop(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);
    let params = active_params();

    c.bench_function("module_crop_2048", |b| {
        b.iter(|| Crop.process_cpu(black_box(img.clone()), black_box(&params)))
    });
}

// ── ImageBuf conversions ─────────────────────────────────────────────────

fn bench_to_rgba_u8_srgb(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);

    c.bench_function("to_rgba_u8_srgb_2048", |b| {
        b.iter(|| black_box(&img).to_rgba_u8_srgb())
    });
}

fn bench_to_rgba_f32(c: &mut Criterion) {
    let img = synthetic_image(2048, 2048);

    c.bench_function("to_rgba_f32_2048", |b| {
        b.iter(|| black_box(&img).to_rgba_f32())
    });
}

fn bench_downsample(c: &mut Criterion) {
    let img = synthetic_image(4096, 3072);

    c.bench_function("downsample_4096x3072_to_2048", |b| {
        b.iter(|| black_box(&img).downsample(black_box(2048)))
    });
}

criterion_group!(
    benches,
    bench_full_pipeline,
    bench_white_balance,
    bench_exposure,
    bench_tone_curve,
    bench_vibrance,
    bench_saturation,
    bench_crop,
    bench_to_rgba_u8_srgb,
    bench_to_rgba_f32,
    bench_downsample,
);
criterion_main!(benches);
