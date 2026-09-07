use crema_app::editor::{
    Comparison, EditorEvent, EditorRuntime, ExportRequest, RenderDemand, RenderKey, RenderQuality,
    RenderRequest, SaveRequest, SourceIdentity, save_state_label,
};
use crema_core::edit::{EditCommand, EditSession, ExposureCentistops, SaveState};
use crema_core::sidecar::{SidecarLocation, SidecarStore};
use crema_core::{AssetCandidate, ScanEvent, scan_folder};
use crema_image::{CandidateFormat, PreviewPixels, PreviewSize, RasterFormat, classify_candidate};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crema-editor-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_jpeg(path: &Path, width: u32, height: u32) {
    let rgb: Vec<_> = (0..width * height)
        .flat_map(|index| {
            let value = (index % 251) as u8;
            [value, value.saturating_add(1), value.saturating_add(2)]
        })
        .collect();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    fs::write(path, bytes).unwrap();
}

fn candidate(directory: &Directory, name: &str) -> AssetCandidate<CandidateFormat> {
    let path = directory.0.join(name);
    write_jpeg(&path, 16, 12);
    let ScanEvent::Candidate(candidate) = scan_folder(&directory.0, classify_candidate)
        .unwrap()
        .find(|event| matches!(event, ScanEvent::Candidate(found) if found.path() == path))
        .unwrap()
    else {
        panic!("candidate")
    };
    candidate
}

fn edited_session(path: &Path, format: CandidateFormat) -> EditSession {
    let location = SidecarLocation::for_original(path, format.sidecar_naming()).unwrap();
    let mut session = EditSession::open(SidecarStore.open(location).unwrap());
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    session
}

fn wait_for(runtime: &EditorRuntime, predicate: impl Fn(&EditorEvent) -> bool) -> EditorEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = runtime.try_recv()
            && predicate(&event)
        {
            return event;
        }
        assert!(Instant::now() < deadline, "editor runtime deadline");
        std::thread::yield_now();
    }
}

#[test]
fn render_demand_rejects_every_stale_identity_dimension() {
    let directory = Directory::new("render-identity");
    let candidate = candidate(&directory, "photo.jpg");
    let mut session = edited_session(candidate.path(), *candidate.kind());
    let current = RenderKey::new(
        candidate.id(),
        session.snapshot().revision(),
        7,
        RenderQuality::Settled,
    );
    let demand = RenderDemand::new(current);

    assert!(demand.accepts(current));
    assert!(!demand.accepts(RenderKey::new(
        candidate.id(),
        session.snapshot().revision(),
        6,
        RenderQuality::Settled,
    )));
    session.apply(EditCommand::ResetExposure);
    assert!(!demand.accepts(RenderKey::new(
        candidate.id(),
        session.snapshot().revision(),
        7,
        RenderQuality::Settled,
    )));
    assert!(!demand.accepts(RenderKey::new(
        candidate.id(),
        session.snapshot().revision(),
        7,
        RenderQuality::Interactive,
    )));
}

#[test]
fn before_is_view_only_and_blocked_edits_stay_visibly_unsaved() {
    let directory = Directory::new("comparison-state");
    let candidate = candidate(&directory, "photo.jpg");
    let sidecar = candidate.path().with_file_name("photo.jpg.xmp");
    fs::write(&sidecar, b"foreign XMP").unwrap();
    let location =
        SidecarLocation::for_original(candidate.path(), candidate.kind().sidecar_naming()).unwrap();
    let mut session = EditSession::open(SidecarStore.open(location).unwrap());
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(75).unwrap(),
    ));
    let snapshot = session.snapshot();

    assert!(Comparison::Before.uses_original(session.recipe()));
    assert!(!Comparison::After.uses_original(session.recipe()));
    assert_eq!(session.snapshot(), snapshot);
    assert_eq!(
        save_state_label(&session.save_state(), session.is_dirty()),
        "Read-only · Unsaved changes"
    );
    assert_eq!(fs::read(sidecar).unwrap(), b"foreign XMP");
}

#[test]
#[cfg(unix)]
fn save_and_latest_render_complete_on_independent_lanes() {
    let directory = Directory::new("independent-lanes");
    let candidate = candidate(&directory, "photo.jpg");
    let mut session = edited_session(candidate.path(), *candidate.kind());
    let identity = SourceIdentity::capture(candidate.path().to_owned()).unwrap();
    let save = SaveRequest::new(
        candidate.id(),
        identity,
        session.begin_save().unwrap().unwrap(),
    );
    let snapshot = session.snapshot();
    let pixels = Arc::new(
        PreviewPixels::new(1, 1, vec![64, 96, 128, 255], PreviewSize::new(1).unwrap()).unwrap(),
    );
    let stale = RenderRequest::new(
        RenderKey::new(
            candidate.id(),
            snapshot.revision(),
            1,
            RenderQuality::Interactive,
        ),
        pixels.clone(),
        snapshot.clone(),
    );
    let latest = RenderRequest::new(
        RenderKey::new(
            candidate.id(),
            snapshot.revision(),
            1,
            RenderQuality::Settled,
        ),
        pixels,
        snapshot,
    );
    let runtime = EditorRuntime::new(std::env::current_exe().unwrap(), || {});

    runtime.replace_render(stale);
    runtime.replace_render(latest);
    runtime.submit_save(save).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saved_asset = None;
    let mut settled_render = false;
    while saved_asset.is_none() || !settled_render {
        if let Some(event) = runtime.try_recv() {
            match event {
                EditorEvent::Saved(result) => saved_asset = Some(result.asset()),
                EditorEvent::Rendered(result) => {
                    settled_render = result.key().quality() == RenderQuality::Settled
                }
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "independent runtime deadline");
        std::thread::yield_now();
    }
    assert_eq!(saved_asset, Some(candidate.id()));
}

#[test]
#[cfg(unix)]
fn export_is_profiled_bounded_atomic_and_never_overwrites() {
    let directory = Directory::new("export");
    let candidate = candidate(&directory, "photo.original.JPG");
    let source = candidate.path().to_owned();
    let original = fs::read(&source).unwrap();
    let sidecar = source.with_file_name("photo.original.JPG.xmp");
    let sidecar_bytes = b"not touched by export";
    fs::write(&sidecar, sidecar_bytes).unwrap();
    let session = edited_session(&source, *candidate.kind());
    let identity = SourceIdentity::capture(source.clone()).unwrap();
    let request = ExportRequest::new(
        candidate.id(),
        identity.clone(),
        *candidate.kind(),
        session.snapshot(),
    )
    .unwrap();
    assert_eq!(
        request.destination(),
        source.with_file_name("photo.original.JPG-crema.jpg")
    );
    let destination = request.destination().to_owned();
    let runtime = EditorRuntime::new(std::env::current_exe().unwrap(), || {});

    runtime.submit_export(request).unwrap();
    let EditorEvent::Exported(exported) =
        wait_for(&runtime, |event| matches!(event, EditorEvent::Exported(_)))
    else {
        panic!("export result")
    };
    assert_eq!(exported.destination(), destination);
    assert_eq!(exported.snapshot().recipe().exposure().value(), 100);
    let decoded = image::load_from_memory(&fs::read(&destination).unwrap()).unwrap();
    assert!(decoded.width().max(decoded.height()) <= 4096);
    let exported_bytes = fs::read(&destination).unwrap();
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&exported_bytes)).unwrap();
    assert!(
        image::ImageDecoder::icc_profile(&mut decoder)
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);

    let replacement = b"existing destination";
    fs::write(&destination, replacement).unwrap();
    let retry = ExportRequest::new(
        candidate.id(),
        identity,
        CandidateFormat::Raster(RasterFormat::Jpeg),
        session.snapshot(),
    )
    .unwrap();
    runtime.submit_export(retry).unwrap();
    let failure = wait_for(&runtime, |event| {
        matches!(event, EditorEvent::ExportFailed(_))
    });
    assert!(matches!(failure, EditorEvent::ExportFailed(_)));
    assert_eq!(fs::read(&destination).unwrap(), replacement);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
}

#[test]
#[cfg(unix)]
fn source_replacement_blocks_save_and_export_and_aliases_never_overwrite() {
    let directory = Directory::new("source-identity");
    let candidate = candidate(&directory, "photo.jpg");
    let source = candidate.path().to_owned();
    let identity = SourceIdentity::capture(source.clone()).unwrap();
    let mut session = edited_session(&source, *candidate.kind());
    let save = SaveRequest::new(
        candidate.id(),
        identity.clone(),
        session.begin_save().unwrap().unwrap(),
    );
    let export = ExportRequest::new(
        candidate.id(),
        identity,
        *candidate.kind(),
        session.snapshot(),
    )
    .unwrap();
    let destination = export.destination().to_owned();
    let parked = directory.0.join("parked.jpg");
    fs::rename(&source, &parked).unwrap();
    write_jpeg(&source, 8, 8);
    let runtime = EditorRuntime::new(std::env::current_exe().unwrap(), || {});

    runtime.submit_save(save).unwrap();
    runtime.submit_export(export).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saved = None;
    let mut export_failed = false;
    while saved.is_none() || !export_failed {
        if let Some(event) = runtime.try_recv() {
            match event {
                EditorEvent::Saved(result) => saved = Some(result),
                EditorEvent::ExportFailed(_) => export_failed = true,
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "identity rejection deadline");
        std::thread::yield_now();
    }
    let saved = saved.unwrap();
    assert_eq!(saved.asset(), candidate.id());
    session.accept_save(saved.into_completion());
    assert!(matches!(session.save_state(), SaveState::Failed(_)));
    assert!(!session.sidecar_path().exists());
    assert!(!destination.exists());

    fs::remove_file(&source).unwrap();
    fs::rename(&parked, &source).unwrap();
    fs::hard_link(&source, &destination).unwrap();
    let source_before = fs::read(&source).unwrap();
    let alias_export = ExportRequest::new(
        candidate.id(),
        SourceIdentity::capture(source.clone()).unwrap(),
        *candidate.kind(),
        session.snapshot(),
    )
    .unwrap();
    runtime.submit_export(alias_export).unwrap();
    let failed = wait_for(&runtime, |event| {
        matches!(event, EditorEvent::ExportFailed(_))
    });
    assert!(matches!(failed, EditorEvent::ExportFailed(_)));
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(fs::read(&destination).unwrap(), source_before);
}

#[test]
#[cfg(unix)]
fn submit_after_shutdown_finishes_the_save_as_failed() {
    let directory = Directory::new("closed-save-runtime");
    let candidate = candidate(&directory, "photo.jpg");
    let mut session = edited_session(candidate.path(), *candidate.kind());
    let request = SaveRequest::new(
        candidate.id(),
        SourceIdentity::capture(candidate.path().to_owned()).unwrap(),
        session.begin_save().unwrap().unwrap(),
    );
    let mut runtime = EditorRuntime::new(std::env::current_exe().unwrap(), || {});
    runtime.shutdown();

    assert!(runtime.submit_save(request).is_err());
    let event = runtime.try_recv().expect("failed save completion");
    let EditorEvent::Saved(result) = event else {
        panic!("save completion")
    };
    session.accept_save(result.into_completion());
    assert!(matches!(session.save_state(), SaveState::Failed(_)));
}
