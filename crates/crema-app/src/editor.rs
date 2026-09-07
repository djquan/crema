use crate::platform::SourceStamp;
use crema_core::AssetId;
use crema_core::edit::{
    EditRecipe, EditRevision, RecipeSnapshot, SaveCommand, SaveCompletion, SaveState,
};
use crema_image::edit_render::{RenderedSdr, render_exposure_srgb8};
use crema_image::jpeg_export::encode_srgb_jpeg;
use crema_image::{
    CancelToken, CandidateFormat, DecodeLimits, DecodeOutcome, Decoder, PreviewPixels, PreviewSize,
};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    Arc, Condvar, Mutex,
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};

const EXPORT_QUALITY: u8 = 92;
static NEXT_EXPORT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RenderQuality {
    Interactive,
    Settled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Comparison {
    Before,
    #[default]
    After,
}

impl Comparison {
    pub fn uses_original(self, recipe: &EditRecipe) -> bool {
        self == Self::Before || recipe.exposure().value() == 0
    }
}

pub fn save_state_label(state: &SaveState, dirty: bool) -> String {
    match state {
        SaveState::Saved => "Saved".to_owned(),
        SaveState::Dirty => "Unsaved changes".to_owned(),
        SaveState::Saving { submitted, current } if submitted == current => "Saving".to_owned(),
        SaveState::Saving { .. } => "Saving · Unsaved changes".to_owned(),
        SaveState::ReadOnly(_) if dirty => "Read-only · Unsaved changes".to_owned(),
        SaveState::ReadOnly(_) => "Read-only".to_owned(),
        SaveState::Conflict(_) if dirty => "Conflict · Unsaved changes".to_owned(),
        SaveState::Conflict(_) => "Conflict".to_owned(),
        SaveState::Failed(_) if dirty => "Save failed · Unsaved changes".to_owned(),
        SaveState::Failed(_) => "Save failed".to_owned(),
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RenderKey {
    asset: AssetId,
    revision: EditRevision,
    input_epoch: u64,
    quality: RenderQuality,
}

impl RenderKey {
    pub const fn new(
        asset: AssetId,
        revision: EditRevision,
        input_epoch: u64,
        quality: RenderQuality,
    ) -> Self {
        Self {
            asset,
            revision,
            input_epoch,
            quality,
        }
    }

    pub const fn asset(self) -> AssetId {
        self.asset
    }

    pub const fn revision(self) -> EditRevision {
        self.revision
    }

    pub const fn input_epoch(self) -> u64 {
        self.input_epoch
    }

    pub const fn quality(self) -> RenderQuality {
        self.quality
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderDemand(RenderKey);

impl RenderDemand {
    pub const fn new(key: RenderKey) -> Self {
        Self(key)
    }

    pub fn accepts(self, key: RenderKey) -> bool {
        self.0 == key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentity {
    path: PathBuf,
    stamp: SourceStamp,
}

impl SourceIdentity {
    pub fn capture(path: PathBuf) -> io::Result<Self> {
        let stamp = SourceStamp::read(&File::open(&path)?)?;
        Ok(Self { path, stamp })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn from_validated(path: PathBuf, stamp: SourceStamp) -> Self {
        Self { path, stamp }
    }
}

pub struct RenderRequest {
    key: RenderKey,
    pixels: Arc<PreviewPixels>,
    snapshot: RecipeSnapshot,
}

impl RenderRequest {
    pub fn new(key: RenderKey, pixels: Arc<PreviewPixels>, snapshot: RecipeSnapshot) -> Self {
        Self {
            key,
            pixels,
            snapshot,
        }
    }
}

pub struct RenderResult {
    key: RenderKey,
    rendered: RenderedSdr,
}

impl RenderResult {
    pub const fn key(&self) -> RenderKey {
        self.key
    }

    pub const fn rendered(&self) -> &RenderedSdr {
        &self.rendered
    }
}

pub struct SaveRequest {
    asset: AssetId,
    source: SourceIdentity,
    command: SaveCommand,
}

impl SaveRequest {
    pub const fn new(asset: AssetId, source: SourceIdentity, command: SaveCommand) -> Self {
        Self {
            asset,
            source,
            command,
        }
    }
}

pub struct SaveResult {
    asset: AssetId,
    completion: SaveCompletion,
}

impl SaveResult {
    pub const fn asset(&self) -> AssetId {
        self.asset
    }

    pub fn into_completion(self) -> SaveCompletion {
        self.completion
    }
}

pub struct ExportRequest {
    asset: AssetId,
    source: SourceIdentity,
    destination: PathBuf,
    format: CandidateFormat,
    snapshot: RecipeSnapshot,
}

impl ExportRequest {
    pub fn new(
        asset: AssetId,
        source: SourceIdentity,
        format: CandidateFormat,
        snapshot: RecipeSnapshot,
    ) -> io::Result<Self> {
        let destination = export_destination(source.path())?;
        Ok(Self {
            asset,
            source,
            destination,
            format,
            snapshot,
        })
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }
}

pub struct ExportResult {
    asset: AssetId,
    destination: PathBuf,
    dimensions: [u32; 2],
    snapshot: RecipeSnapshot,
}

impl ExportResult {
    pub const fn asset(&self) -> AssetId {
        self.asset
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub const fn dimensions(&self) -> [u32; 2] {
        self.dimensions
    }

    pub const fn snapshot(&self) -> &RecipeSnapshot {
        &self.snapshot
    }
}

pub struct ExportFailure {
    asset: AssetId,
    message: String,
}

impl ExportFailure {
    pub const fn asset(&self) -> AssetId {
        self.asset
    }
}

impl fmt::Display for ExportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub enum EditorEvent {
    Rendered(RenderResult),
    Saved(SaveResult),
    Exported(ExportResult),
    ExportFailed(ExportFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    Busy,
    Closed,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "editor operation queue is full",
            Self::Closed => "editor runtime is closed",
        })
    }
}

impl Error for SubmitError {}

#[derive(Default)]
struct RenderState {
    stopping: bool,
    pending: Option<RenderRequest>,
    latest: Option<RenderKey>,
}

type SharedRender = Arc<(Mutex<RenderState>, Condvar)>;
type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct EditorRuntime {
    render: SharedRender,
    save_sender: Option<SyncSender<SaveRequest>>,
    export_sender: Option<SyncSender<ExportRequest>>,
    event_sender: mpsc::Sender<EditorEvent>,
    receiver: Receiver<EditorEvent>,
    wake: Wake,
    workers: Vec<JoinHandle<()>>,
}

impl EditorRuntime {
    pub fn new(executable: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let wake: Wake = Arc::new(wake);
        let (event_sender, receiver) = mpsc::channel();
        let render = Arc::new((Mutex::new(RenderState::default()), Condvar::new()));

        let render_worker = {
            let render = render.clone();
            let sender = event_sender.clone();
            let wake = wake.clone();
            thread::spawn(move || {
                loop {
                    let request = {
                        let mut state = render.0.lock().expect("editor render state");
                        while state.pending.is_none() && !state.stopping {
                            state = render.1.wait(state).expect("editor render demand");
                        }
                        if state.stopping {
                            return;
                        }
                        state.pending.take().expect("pending render")
                    };
                    let rendered =
                        render_exposure_srgb8(&request.pixels, request.snapshot.recipe());
                    let state = render.0.lock().expect("editor render state");
                    let current = !state.stopping && state.latest == Some(request.key);
                    drop(state);
                    if current {
                        let _ = sender.send(EditorEvent::Rendered(RenderResult {
                            key: request.key,
                            rendered,
                        }));
                        wake();
                    }
                }
            })
        };

        let (save_sender, save_receiver) = mpsc::sync_channel::<SaveRequest>(8);
        let save_worker = {
            let sender = event_sender.clone();
            let wake = wake.clone();
            thread::spawn(move || {
                let store = crema_core::sidecar::SidecarStore;
                for request in save_receiver {
                    let SaveRequest {
                        asset,
                        source,
                        command,
                    } = request;
                    let failure_path = source.path.clone();
                    let completion = store.commit_guarded(command, || {
                        verify_source_identity(&source).map_err(|message| {
                            crema_core::sidecar::SaveFailure::Io {
                                path: failure_path,
                                message,
                            }
                        })
                    });
                    let _ = sender.send(EditorEvent::Saved(SaveResult { asset, completion }));
                    wake();
                }
            })
        };

        let (export_sender, export_receiver) = mpsc::sync_channel::<ExportRequest>(4);
        let export_worker = {
            let sender = event_sender.clone();
            let wake = wake.clone();
            thread::spawn(move || {
                let decoder = Decoder::new(executable, DecodeLimits::default());
                for request in export_receiver {
                    let asset = request.asset;
                    let event = match export_one(&decoder, request) {
                        Ok(result) => EditorEvent::Exported(result),
                        Err(message) => EditorEvent::ExportFailed(ExportFailure { asset, message }),
                    };
                    let _ = sender.send(event);
                    wake();
                }
            })
        };

        Self {
            render,
            save_sender: Some(save_sender),
            export_sender: Some(export_sender),
            event_sender,
            receiver,
            wake,
            workers: vec![render_worker, save_worker, export_worker],
        }
    }

    pub fn replace_render(&self, request: RenderRequest) {
        let mut state = self.render.0.lock().expect("editor render state");
        if state.stopping {
            return;
        }
        state.latest = Some(request.key);
        state.pending = Some(request);
        self.render.1.notify_one();
    }

    pub fn clear_render(&self, asset: AssetId) {
        let mut state = self.render.0.lock().expect("editor render state");
        if state.latest.is_some_and(|key| key.asset == asset) {
            state.latest = None;
            state.pending = None;
        }
    }

    pub fn submit_save(&self, request: SaveRequest) -> Result<(), SubmitError> {
        let Some(sender) = &self.save_sender else {
            self.reject_save(request, "save runtime is closed");
            return Err(SubmitError::Closed);
        };
        match sender.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                self.reject_save(request, "save queue is full");
                Err(SubmitError::Busy)
            }
            Err(TrySendError::Disconnected(request)) => {
                self.reject_save(request, "save runtime is closed");
                Err(SubmitError::Closed)
            }
        }
    }

    fn reject_save(&self, request: SaveRequest, message: &str) {
        let completion = SaveCompletion::failed(
            request.command.job(),
            crema_core::sidecar::SaveFailure::Io {
                path: request.source.path.clone(),
                message: message.to_owned(),
            },
        );
        let _ = self.event_sender.send(EditorEvent::Saved(SaveResult {
            asset: request.asset,
            completion,
        }));
        (self.wake)();
    }

    pub fn submit_export(&self, request: ExportRequest) -> Result<(), SubmitError> {
        let Some(sender) = &self.export_sender else {
            return Err(SubmitError::Closed);
        };
        match sender.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                let _ = self
                    .event_sender
                    .send(EditorEvent::ExportFailed(ExportFailure {
                        asset: request.asset,
                        message: "export queue is full".to_owned(),
                    }));
                (self.wake)();
                Err(SubmitError::Busy)
            }
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Closed),
        }
    }

    pub fn try_recv(&self) -> Option<EditorEvent> {
        self.receiver.try_recv().ok()
    }

    pub fn shutdown(&mut self) {
        {
            let mut state = self.render.0.lock().expect("editor render state");
            state.stopping = true;
            state.pending = None;
            self.render.1.notify_all();
        }
        self.save_sender = None;
        self.export_sender = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for EditorRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn export_destination(source: &Path) -> io::Result<PathBuf> {
    let file_name = source
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source has no filename"))?;
    let mut destination = OsString::from(file_name);
    destination.push("-crema.jpg");
    Ok(source.with_file_name(destination))
}

fn export_one(decoder: &Decoder, request: ExportRequest) -> Result<ExportResult, String> {
    match fs::symlink_metadata(&request.destination) {
        Ok(_) => {
            return Err(format!(
                "export already exists at {}",
                request.destination.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect export destination: {error}")),
    }

    let mut source = File::open(request.source.path())
        .map_err(|error| format!("cannot reopen source read-only: {error}"))?;
    ensure_source_identity(&source, &request.source)?;
    let decoded = decoder.decode_opened(
        &mut source,
        request.format,
        PreviewSize::new(4096).expect("fixed export bound"),
        &CancelToken::new(),
        &|_| {},
    );
    let decoded = match decoded {
        DecodeOutcome::Decoded(decoded) => decoded,
        DecodeOutcome::Unsupported(message) => return Err(message),
        DecodeOutcome::Failed(error) => return Err(error.to_string()),
    };
    ensure_source_identity(&source, &request.source)?;
    let rendered = render_exposure_srgb8(&decoded.preview, request.snapshot.recipe());
    let dimensions = [rendered.width(), rendered.height()];
    let jpeg = encode_srgb_jpeg(&rendered, EXPORT_QUALITY).map_err(|error| error.to_string())?;
    publish_export(&request.destination, &jpeg, || {
        ensure_source_identity(&source, &request.source)
    })?;
    Ok(ExportResult {
        asset: request.asset,
        destination: request.destination,
        dimensions,
        snapshot: request.snapshot,
    })
}

fn verify_source_identity(expected: &SourceIdentity) -> Result<(), String> {
    let source = File::open(expected.path()).map_err(|error| error.to_string())?;
    ensure_source_identity(&source, expected)
}

fn ensure_source_identity(source: &File, expected: &SourceIdentity) -> Result<(), String> {
    let opened = SourceStamp::read(source).map_err(|error| error.to_string())?;
    let current = File::open(expected.path())
        .and_then(|file| SourceStamp::read(&file))
        .map_err(|error| error.to_string())?;
    if opened == expected.stamp && current == expected.stamp {
        Ok(())
    } else {
        Err("source changed since it was opened".to_owned())
    }
}

fn publish_export(
    destination: &Path,
    bytes: &[u8],
    validate: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let (temporary_path, mut temporary) = create_export_temp(destination)?;
    let result = (|| {
        temporary
            .write_all(bytes)
            .and_then(|_| temporary.flush())
            .and_then(|_| temporary.sync_all())
            .map_err(|error| error.to_string())?;
        drop(temporary);
        validate()?;
        fs::hard_link(&temporary_path, destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                format!("export already exists at {}", destination.display())
            } else {
                format!("cannot publish export: {error}")
            }
        })?;
        let _ = fs::remove_file(&temporary_path);
        sync_parent(destination).map_err(|error| format!("cannot sync export folder: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_export_temp(destination: &Path) -> Result<(PathBuf, File), String> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let file_name = destination
        .file_name()
        .expect("validated export path")
        .to_string_lossy();
    for _ in 0..100 {
        let sequence = NEXT_EXPORT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.crema-tmp-{}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create export temporary file: {error}")),
        }
    }
    Err("cannot reserve a unique export temporary file".to_owned())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}
