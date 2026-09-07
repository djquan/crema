use crate::{
    jobs::{PreviewRequest, Purpose},
    metrics::Span,
    platform::SourceStamp,
    thumbnail_cache::{CacheConfig, ThumbnailCache},
};
use crema_image::{
    CancelToken, DecodeError, DecodeEvent, DecodeLimits, DecodeOutcome, DecodeResult, Decoder,
    FailureClass, PreviewPixels,
};
use std::{fs::File, path::PathBuf, time::Instant};

pub(crate) struct PreviewEngine {
    decoder: Decoder,
    cache: CacheConfig,
}
impl PreviewEngine {
    pub fn new(executable: PathBuf, cache: CacheConfig) -> Self {
        Self {
            decoder: Decoder::new(executable, DecodeLimits::default()),
            cache,
        }
    }
    pub fn run(
        &self,
        request: &PreviewRequest,
        cancel: &CancelToken,
        span: &Span,
        emit: impl Fn(Purpose, DecodeOutcome, Option<SourceStamp>, bool) -> bool,
    ) {
        let config = request
            .path
            .parent()
            .map(|source| self.cache.clone().outside_source(source));
        let cache = match config {
            Some(config) => ThumbnailCache::new(config.root, config.budget),
            None => ThumbnailCache::new(None, 0),
        };
        for retry in 0..2 {
            if cancel.is_cancelled() {
                return;
            }
            let mut source = match File::open(&request.path) {
                Ok(file) => file,
                Err(error) => {
                    emit(
                        request.key.purpose,
                        DecodeOutcome::Failed(error.into()),
                        None,
                        true,
                    );
                    return;
                }
            };
            let stamp_start = Instant::now();
            let stamp = SourceStamp::read(&source).ok();
            span.record("source_stamp_us", stamp_start.elapsed().as_micros() as u64);
            let key = stamp.as_ref().map(|stamp| cache_key(stamp, request));
            if let Some(cached) = key.as_ref().and_then(|key| cache.load(key)) {
                if !stable(&source, &request.path, stamp.as_ref()) {
                    continue;
                }
                span.record("cache_hit", 1);
                let terminal = request.key.purpose == Purpose::Thumbnail;
                if !emit(
                    Purpose::Thumbnail,
                    DecodeOutcome::Decoded(cached),
                    stamp.clone(),
                    terminal,
                ) || terminal
                {
                    return;
                }
            } else {
                span.record("cache_miss", 1);
            }
            let outcome = self.decoder.decode_opened(
                &mut source,
                request.format,
                request.key.purpose.size(),
                cancel,
                &|event| match event {
                    DecodeEvent::SourceRead(bytes) => span.record("source_read_bytes", bytes),
                    DecodeEvent::WorkerSpawned(pid) => {
                        span.record("worker_spawned", u64::from(pid))
                    }
                    DecodeEvent::WorkerReaped(pid) => span.record("worker_reaped", u64::from(pid)),
                },
            );
            if cancel.is_cancelled() {
                if matches!(&outcome, DecodeOutcome::Decoded(_)) {
                    span.record("obsolete_drop", 1);
                }
                return;
            }
            if !stable(&source, &request.path, stamp.as_ref()) {
                span.record("source_changed", 1);
                if retry == 0 {
                    continue;
                }
                emit(
                    request.key.purpose,
                    DecodeOutcome::Failed(DecodeError {
                        class: FailureClass::Io,
                        message: "source changed during both decode attempts".into(),
                    }),
                    None,
                    true,
                );
                return;
            }
            let thumbnail = match &outcome {
                DecodeOutcome::Decoded(result) => derive_thumbnail(result),
                _ => None,
            };
            if !emit(request.key.purpose, outcome, stamp, true) {
                return;
            }
            if let (Some(key), Some(thumbnail)) = (key, thumbnail) {
                span.record("cache_store", u64::from(cache.store(&key, &thumbnail)));
            }
            if !cancel.is_cancelled() {
                let start = Instant::now();
                cache.maintain(|| !cancel.is_cancelled());
                span.record("cache_maintenance_us", start.elapsed().as_micros() as u64);
            }
            return;
        }
        emit(
            request.key.purpose,
            DecodeOutcome::Failed(DecodeError {
                class: FailureClass::Io,
                message: "source changed during cache lookup".into(),
            }),
            None,
            true,
        );
    }
}

fn stable(source: &File, path: &std::path::Path, before: Option<&SourceStamp>) -> bool {
    let Some(before) = before else {
        return true;
    };
    SourceStamp::read(source).as_ref().ok() == Some(before)
        && File::open(path)
            .ok()
            .and_then(|file| SourceStamp::read(&file).ok())
            .as_ref()
            == Some(before)
}
fn cache_key(stamp: &SourceStamp, request: &PreviewRequest) -> Vec<u8> {
    let mut key = b"crema-renderer-1;edge-320;route-".to_vec();
    key.extend(request.format.to_string().as_bytes());
    key.push(0);
    key.extend(&stamp.0);
    key
}
fn derive_thumbnail(result: &DecodeResult) -> Option<DecodeResult> {
    let pixels = &result.preview;
    let source = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
        pixels.width(),
        pixels.height(),
        pixels.rgba8(),
    )?;
    let thumbnail =
        image::imageops::thumbnail(&source, 320.min(pixels.width()), 320.min(pixels.height()));
    Some(DecodeResult {
        preview: PreviewPixels::new(
            thumbnail.width(),
            thumbnail.height(),
            thumbnail.into_raw(),
            Purpose::Thumbnail.size(),
        )
        .ok()?,
        metadata: result.metadata.clone(),
        provenance: result.provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{jobs::JobKey, metrics::Metrics};
    use crema_core::{ScanEvent, scan_folder};
    use std::sync::Mutex;
    #[test]
    fn warm_thumbnail_reads_no_source_content_and_viewer_derives_it() {
        let root = std::env::temp_dir().join(format!("crema-engine-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join("originals")).unwrap();
        let path = root.join("originals/source.jpg");
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode(
                &vec![100; 800 * 600 * 3],
                800,
                600,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let ScanEvent::Candidate(candidate) =
            scan_folder(root.join("originals"), crema_image::classify_candidate)
                .unwrap()
                .next()
                .unwrap()
        else {
            panic!("candidate");
        };
        let request = PreviewRequest {
            key: JobKey {
                generation: 1,
                asset: candidate.id(),
                purpose: Purpose::Viewer,
            },
            path: path.clone(),
            format: *candidate.kind(),
            needed: true,
        };
        let metrics = Metrics::new(true);
        let span = Span {
            metrics: metrics.clone(),
            key: request.key,
            interest: 1,
            attempt: 1,
        };
        let config = CacheConfig {
            root: Some(root.join("cache")),
            budget: 1024 * 1024,
        };
        let engine = PreviewEngine::new(std::env::current_exe().unwrap(), config.clone());
        let delivered_stamp = Mutex::new(None);
        engine.run(
            &request,
            &CancelToken::new(),
            &span,
            |purpose, outcome, source_stamp, terminal| {
                assert_eq!(purpose, Purpose::Viewer);
                assert!(terminal);
                assert!(matches!(outcome, DecodeOutcome::Decoded(_)));
                *delivered_stamp.lock().unwrap() = source_stamp;
                true
            },
        );
        assert_eq!(
            delivered_stamp.lock().unwrap().as_ref(),
            Some(&SourceStamp::read(&File::open(&path).unwrap()).unwrap())
        );
        assert!(
            metrics
                .records()
                .iter()
                .any(|event| event.event == "source_read_bytes")
        );
        drop(engine);
        let request = PreviewRequest {
            key: JobKey {
                purpose: Purpose::Thumbnail,
                ..request.key
            },
            ..request
        };
        let metrics = Metrics::new(true);
        let span = Span {
            metrics: metrics.clone(),
            key: request.key,
            interest: 2,
            attempt: 2,
        };
        let engine = PreviewEngine::new(std::env::current_exe().unwrap(), config);
        let pixels = Mutex::new(Vec::new());
        engine.run(
            &request,
            &CancelToken::new(),
            &span,
            |_, outcome, _source_stamp, terminal| {
                assert!(terminal);
                let DecodeOutcome::Decoded(result) = outcome else {
                    panic!("cache hit");
                };
                *pixels.lock().unwrap() = result.preview.rgba8().to_vec();
                true
            },
        );
        assert!(!pixels.lock().unwrap().is_empty());
        assert!(
            metrics
                .records()
                .iter()
                .any(|event| event.event == "cache_hit")
        );
        assert!(
            !metrics
                .records()
                .iter()
                .any(|event| matches!(event.event, "source_read_bytes" | "worker_spawned"))
        );
        let stamp = SourceStamp::read(&File::open(&path).unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_ne!(
            stamp,
            SourceStamp::read(&File::open(&path).unwrap()).unwrap()
        );
        let moved = root.join("moved.jpg");
        std::fs::rename(&path, &moved).unwrap();
        assert!(!stable(&File::open(&moved).unwrap(), &path, Some(&stamp)));
        std::fs::remove_dir_all(root).unwrap();
    }
}
