use crate::editor::{
    Comparison, EditorEvent, EditorRuntime, ExportRequest, RenderDemand, RenderKey, RenderQuality,
    RenderRequest, SaveRequest, SourceIdentity, save_state_label,
};
use crate::jobs::{Event, JobKey, PreviewDemand, PreviewRequest, PreviewRuntime, Purpose};
use crate::{metrics::Metrics, thumbnail_cache::CacheConfig};
use crema_core::edit::{EditCommand, EditSession, ExposureCentistops, SaveState};
use crema_core::sidecar::{SidecarLocation, SidecarStore};
use crema_core::{AssetCandidate, AssetId};
use crema_image::{
    CandidateFormat, DecodeOutcome, DecodeResult, FailureClass, PreviewPixels, PreviewSize,
    Provenance, SourceMetadata,
};
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum View {
    Grid,
    Viewer,
}

struct Workspace {
    generation: u64,
    assets: Vec<AssetCandidate<CandidateFormat>>,
    index: HashMap<AssetId, usize>,
    selected: Option<AssetId>,
    scanning: bool,
    failures: Vec<String>,
    view: View,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            generation: 1,
            assets: Vec::new(),
            index: HashMap::new(),
            selected: None,
            scanning: true,
            failures: Vec::new(),
            view: View::Grid,
        }
    }
}

impl Workspace {
    fn demand_keys(&self, visible: Vec<JobKey>) -> (Option<JobKey>, Vec<JobKey>) {
        let selected = self.selected.map(|asset| JobKey {
            generation: self.generation,
            asset,
            purpose: if self.view == View::Viewer {
                Purpose::Viewer
            } else {
                Purpose::Thumbnail
            },
        });
        let thumbnails = visible
            .into_iter()
            .filter(|key| Some(*key) != selected)
            .collect();
        (selected, thumbnails)
    }
    fn insert(&mut self, generation: u64, candidate: AssetCandidate<CandidateFormat>) {
        if generation != self.generation {
            return;
        }
        let id = candidate.id();
        self.index.insert(id, self.assets.len());
        self.assets.push(candidate);
        if self.selected.is_none() {
            self.selected = Some(id);
        }
    }

    fn accepts(&self, key: JobKey) -> bool {
        key.generation == self.generation && self.index.contains_key(&key.asset)
    }

    fn navigate(&mut self, offset: isize) {
        if self.assets.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.index.get(&id).copied())
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(offset)
            .min(self.assets.len() - 1);
        self.selected = Some(self.assets[next].id());
    }

    fn shortcut(&mut self, key: egui::Key, wants_keyboard_input: bool) -> bool {
        let selected = self.selected;
        if key == egui::Key::Escape && self.view == View::Viewer {
            self.view = View::Grid;
            return false;
        }
        if wants_keyboard_input && self.view == View::Viewer {
            return false;
        }
        match key {
            egui::Key::ArrowLeft => self.navigate(-1),
            egui::Key::ArrowRight => self.navigate(1),
            egui::Key::Enter => self.view = View::Viewer,
            _ => {}
        }
        self.selected != selected
    }
}

fn thumbnail_access_id(asset: AssetId) -> egui::Id {
    egui::Id::new(("crema-thumbnail", asset))
}

fn viewer_access_id(asset: AssetId) -> egui::Id {
    egui::Id::new(("crema-viewer-image", asset))
}

fn thumbnail_author_id(asset: AssetId) -> String {
    format!("thumbnail-{asset}")
}

fn viewer_author_id(asset: AssetId) -> String {
    format!("viewer-image-{asset}")
}

fn reveal_scroll_offset(index: usize, columns: usize, row_height: f32) -> f32 {
    (index / columns) as f32 * row_height
}

enum Cached {
    Image {
        texture: TextureHandle,
        viewer_pixels: Option<ViewerPixels>,
        metadata: SourceMetadata,
        provenance: Provenance,
        used: u64,
    },
    Unavailable {
        reason: String,
        failure: Option<FailureClass>,
        used: u64,
    },
}

#[derive(Clone)]
struct ViewerPixels {
    interactive: Arc<PreviewPixels>,
    detail: Arc<PreviewPixels>,
}

enum EditorDocument {
    Ready {
        session: EditSession,
        source: Option<SourceIdentity>,
        source_error: Option<String>,
    },
    ReadOnly(String),
}

struct EditedPreview {
    demand: RenderDemand,
    texture: TextureHandle,
}

#[derive(Default)]
enum ExportState {
    #[default]
    Idle,
    Exporting,
    Exported(String),
    Failed(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CloseConfirmation {
    #[default]
    Inactive,
    Prompting,
    Discarding,
}

impl Cached {
    fn bytes(&self) -> usize {
        match self {
            Self::Image {
                texture,
                viewer_pixels,
                ..
            } => {
                let cpu = viewer_pixels
                    .as_ref()
                    .map(|pixels| pixels.interactive.rgba8().len() + pixels.detail.rgba8().len())
                    .unwrap_or(0);
                texture.size()[0] * texture.size()[1] * 4 + cpu
            }
            Self::Unavailable { .. } => 0,
        }
    }

    fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable {
                failure: Some(
                    FailureClass::Io
                        | FailureClass::Timeout
                        | FailureClass::Cancelled
                        | FailureClass::WorkerExited
                ),
                ..
            }
        )
    }
}

fn unavailable_to_evict(
    cache: &HashMap<JobKey, Cached>,
    purpose: Purpose,
    cap: usize,
) -> Option<JobKey> {
    let entries: Vec<_> = cache
        .iter()
        .filter_map(|(key, value)| match value {
            Cached::Unavailable { used, .. } if key.purpose == purpose => Some((*key, *used)),
            _ => None,
        })
        .collect();
    (entries.len() > cap).then(|| {
        entries
            .into_iter()
            .min_by_key(|(_, used)| *used)
            .expect("over capacity")
            .0
    })
}

fn viewer_display_key(
    cache: &HashMap<JobKey, Cached>,
    viewer: JobKey,
    thumbnail: JobKey,
) -> JobKey {
    if matches!(cache.get(&viewer), Some(Cached::Image { .. })) {
        viewer
    } else if matches!(cache.get(&thumbnail), Some(Cached::Image { .. })) {
        thumbnail
    } else if cache.contains_key(&viewer) {
        viewer
    } else {
        thumbnail
    }
}

fn concise_reason(reason: &str) -> String {
    reason
        .lines()
        .next()
        .unwrap_or(reason)
        .chars()
        .take(160)
        .collect()
}

fn open_document(path: &std::path::Path, format: CandidateFormat) -> EditorDocument {
    let location = match SidecarLocation::for_original(path, format.sidecar_naming()) {
        Ok(location) => location,
        Err(error) => return EditorDocument::ReadOnly(error.to_string()),
    };
    match SidecarStore.open(location) {
        Ok(opened) => EditorDocument::Ready {
            session: EditSession::open(opened),
            source: None,
            source_error: None,
        },
        Err(error) => EditorDocument::ReadOnly(error.to_string()),
    }
}

fn prepare_viewer_pixels(pixels: PreviewPixels) -> ViewerPixels {
    let detail = Arc::new(pixels);
    if detail.width().max(detail.height()) <= 1024 {
        return ViewerPixels {
            interactive: detail.clone(),
            detail,
        };
    }
    let image = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
        detail.width(),
        detail.height(),
        detail.rgba8(),
    )
    .expect("validated preview pixels");
    let interactive = image::imageops::thumbnail(&image, 1024, 1024);
    let interactive = PreviewPixels::new(
        interactive.width(),
        interactive.height(),
        interactive.into_raw(),
        PreviewSize::new(1024).expect("fixed interactive bound"),
    )
    .expect("derived interactive preview");
    ViewerPixels {
        interactive: Arc::new(interactive),
        detail,
    }
}

fn retain_viewer_pixels(purpose: Purpose, pixels: PreviewPixels) -> Option<ViewerPixels> {
    (purpose == Purpose::Viewer).then(|| prepare_viewer_pixels(pixels))
}

fn accepts_render(demands: &HashMap<AssetId, RenderDemand>, key: RenderKey) -> bool {
    demands
        .get(&key.asset())
        .is_some_and(|demand| demand.accepts(key))
}

fn needs_activation_render(
    previous: Option<AssetId>,
    next: AssetId,
    nonzero_recipe: bool,
    has_accepted_texture: bool,
) -> bool {
    previous != Some(next) && nonzero_recipe && !has_accepted_texture
}

fn cancel_close(requested: bool, has_unsaved: bool, state: CloseConfirmation) -> bool {
    requested && has_unsaved && state != CloseConfirmation::Discarding
}

pub struct Browser {
    root: PathBuf,
    workspace: Workspace,
    jobs: PreviewRuntime,
    cache: HashMap<JobKey, Cached>,
    tick: u64,
    actual_pixels: bool,
    thumbnail_width: f32,
    metrics: Metrics,
    metric_path: Option<PathBuf>,
    measured_selection: Option<JobKey>,
    drawn: HashSet<Purpose>,
    editor: EditorRuntime,
    documents: HashMap<AssetId, EditorDocument>,
    input_epochs: HashMap<AssetId, u64>,
    render_demands: HashMap<AssetId, RenderDemand>,
    edited_previews: HashMap<AssetId, EditedPreview>,
    comparison: Comparison,
    exports: HashMap<AssetId, ExportState>,
    active_viewer: Option<AssetId>,
    close_confirmation: CloseConfirmation,
    reveal_selected: bool,
}

impl Browser {
    pub fn new(root: PathBuf, executable: PathBuf, context: egui::Context) -> Self {
        Self::with_options(
            root,
            executable,
            context,
            CacheConfig::default(),
            Metrics::default(),
            None,
        )
    }
    pub fn with_options(
        root: PathBuf,
        executable: PathBuf,
        context: egui::Context,
        cache: CacheConfig,
        metrics: Metrics,
        metric_path: Option<PathBuf>,
    ) -> Self {
        context.set_visuals(egui::Visuals::dark());
        let mut style = (*context.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.visuals.selection.bg_fill = Color32::from_rgb(99, 116, 93);
        style.visuals.panel_fill = Color32::from_rgb(27, 28, 27);
        context.set_style_of(egui::Theme::Dark, style);
        let preview_context = context.clone();
        let editor_executable = executable.clone();
        let jobs = PreviewRuntime::with_options(
            executable,
            cache.outside_source(&root),
            metrics.clone(),
            move || preview_context.request_repaint(),
        );
        let editor_context = context.clone();
        let editor =
            EditorRuntime::new(editor_executable, move || editor_context.request_repaint());
        jobs.scan(root.clone(), 1);
        Self {
            root,
            workspace: Workspace::default(),
            jobs,
            cache: HashMap::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
            metrics,
            metric_path,
            measured_selection: None,
            drawn: HashSet::new(),
            editor,
            documents: HashMap::new(),
            input_epochs: HashMap::new(),
            render_demands: HashMap::new(),
            edited_previews: HashMap::new(),
            comparison: Comparison::After,
            exports: HashMap::new(),
            active_viewer: None,
            close_confirmation: CloseConfirmation::Inactive,
            reveal_selected: false,
        }
    }

    fn key(&self, asset: AssetId, purpose: Purpose) -> JobKey {
        JobKey {
            generation: self.workspace.generation,
            asset,
            purpose,
        }
    }

    fn set_source_identity(&mut self, asset: AssetId, stamp: Option<crate::SourceStamp>) {
        let Some(path) = self
            .workspace
            .index
            .get(&asset)
            .map(|index| self.workspace.assets[*index].path().to_owned())
        else {
            return;
        };
        if let Some(EditorDocument::Ready {
            source,
            source_error,
            ..
        }) = self.documents.get_mut(&asset)
        {
            match stamp {
                Some(stamp) => {
                    *source = Some(SourceIdentity::from_validated(path, stamp));
                    *source_error = None;
                }
                None => {
                    *source = None;
                    *source_error = Some("source identity is unavailable".to_owned());
                }
            }
        }
    }

    fn ensure_document(&mut self, asset: AssetId) {
        if self.documents.contains_key(&asset) {
            return;
        }
        let Some(index) = self.workspace.index.get(&asset).copied() else {
            return;
        };
        let candidate = &self.workspace.assets[index];
        self.documents
            .insert(asset, open_document(candidate.path(), *candidate.kind()));
        self.exports.entry(asset).or_default();
    }

    fn activate_viewer(&mut self, next: Option<AssetId>) {
        let previous = self.active_viewer;
        if previous == next {
            return;
        }
        self.active_viewer = next;
        let Some(asset) = next else {
            return;
        };
        self.ensure_document(asset);
        let nonzero_recipe = matches!(
            self.documents.get(&asset),
            Some(EditorDocument::Ready { session, .. })
                if session.recipe().exposure().value() != 0
        );
        let has_accepted_texture = self.edited_previews.get(&asset).is_some_and(|preview| {
            self.render_demands
                .get(&asset)
                .is_some_and(|demand| *demand == preview.demand)
        });
        if needs_activation_render(previous, asset, nonzero_recipe, has_accepted_texture) {
            self.schedule_render(asset, RenderQuality::Settled);
        }
    }

    fn remove_cache_entry(&mut self, key: JobKey) {
        self.cache.remove(&key);
        if key.purpose == Purpose::Viewer {
            self.render_demands.remove(&key.asset);
            self.edited_previews.remove(&key.asset);
            self.editor.clear_render(key.asset);
        }
    }

    fn has_unsaved_changes(&self) -> bool {
        self.documents.values().any(|document| {
            matches!(
                document,
                EditorDocument::Ready { session, .. } if session.is_dirty()
            )
        })
    }

    fn handle_close(&mut self, context: &egui::Context) {
        let requested = context.input(|input| input.viewport().close_requested());
        if cancel_close(
            requested,
            self.has_unsaved_changes(),
            self.close_confirmation,
        ) {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirmation = CloseConfirmation::Prompting;
        }
        if self.close_confirmation != CloseConfirmation::Prompting {
            return;
        }
        egui::Modal::new(egui::Id::new("dirty-close-confirmation")).show(context, |ui| {
            ui.heading("Unsaved changes");
            ui.label("One or more photos have changes that are not saved to XMP.");
            ui.horizontal(|ui| {
                if ui.button("Keep editing").clicked() {
                    self.close_confirmation = CloseConfirmation::Inactive;
                }
                if ui.button("Discard changes and close").clicked() {
                    self.close_confirmation = CloseConfirmation::Discarding;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }

    fn schedule_render(&mut self, asset: AssetId, quality: RenderQuality) {
        let Some(EditorDocument::Ready { session, .. }) = self.documents.get(&asset) else {
            return;
        };
        if session.recipe().exposure().value() == 0 {
            self.render_demands.remove(&asset);
            self.edited_previews.remove(&asset);
            self.editor.clear_render(asset);
            return;
        }
        let snapshot = session.snapshot();
        let Some(epoch) = self.input_epochs.get(&asset).copied() else {
            return;
        };
        let viewer = self.key(asset, Purpose::Viewer);
        let Some(Cached::Image {
            viewer_pixels: Some(pixels),
            ..
        }) = self.cache.get(&viewer)
        else {
            return;
        };
        let pixels = match quality {
            RenderQuality::Interactive => pixels.interactive.clone(),
            RenderQuality::Settled => pixels.detail.clone(),
        };
        let key = RenderKey::new(asset, snapshot.revision(), epoch, quality);
        self.metrics.record(
            "edit_render_requested",
            Some(viewer),
            0,
            0,
            u64::from(pixels.width()) * u64::from(pixels.height()),
        );
        self.render_demands.insert(asset, RenderDemand::new(key));
        self.editor
            .replace_render(RenderRequest::new(key, pixels, snapshot));
    }

    fn receive_editor(&mut self, context: &egui::Context) {
        while let Some(event) = self.editor.try_recv() {
            match event {
                EditorEvent::Rendered(result) => {
                    let key = result.key();
                    if !accepts_render(&self.render_demands, key) {
                        continue;
                    }
                    let rendered = result.rendered();
                    self.metrics.record(
                        "edit_render_received",
                        Some(self.key(key.asset(), Purpose::Viewer)),
                        0,
                        0,
                        u64::from(rendered.width()) * u64::from(rendered.height()),
                    );
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [rendered.width() as usize, rendered.height() as usize],
                        rendered.rgba8(),
                    );
                    let texture = context.load_texture(
                        format!(
                            "{}-edit-{}-{}-{:?}",
                            key.asset(),
                            key.revision().value(),
                            key.input_epoch(),
                            key.quality()
                        ),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.edited_previews.insert(
                        key.asset(),
                        EditedPreview {
                            demand: RenderDemand::new(key),
                            texture,
                        },
                    );
                }
                EditorEvent::Saved(result) => {
                    let asset = result.asset();
                    let mut succeeded = false;
                    if let Some(EditorDocument::Ready { session, .. }) =
                        self.documents.get_mut(&asset)
                    {
                        session.accept_save(result.into_completion());
                        succeeded = !matches!(session.save_state(), SaveState::Failed(_));
                    }
                    self.metrics.record(
                        "save_finished",
                        Some(self.key(asset, Purpose::Viewer)),
                        0,
                        0,
                        u64::from(succeeded),
                    );
                }
                EditorEvent::Exported(result) => {
                    self.metrics.record(
                        "export_finished",
                        Some(self.key(result.asset(), Purpose::Viewer)),
                        0,
                        0,
                        1,
                    );
                    let exposure = result.snapshot().recipe().exposure().as_stops();
                    let current_is_newer = matches!(
                        self.documents.get(&result.asset()),
                        Some(EditorDocument::Ready { session, .. })
                            if session.revision() != result.snapshot().revision()
                    );
                    let newer = if current_is_newer {
                        ". Current controls are newer"
                    } else {
                        ""
                    };
                    self.exports.insert(
                        result.asset(),
                        ExportState::Exported(format!(
                            "Exported {} × {} at {exposure:+.2} EV to {}{newer}",
                            result.dimensions()[0],
                            result.dimensions()[1],
                            result.destination().display()
                        )),
                    );
                }
                EditorEvent::ExportFailed(failure) => {
                    self.metrics.record(
                        "export_finished",
                        Some(self.key(failure.asset(), Purpose::Viewer)),
                        0,
                        0,
                        0,
                    );
                    self.exports
                        .insert(failure.asset(), ExportState::Failed(failure.to_string()));
                }
            }
        }
    }

    fn editor_controls(&mut self, ui: &mut egui::Ui) {
        let Some(asset) = self.workspace.selected else {
            return;
        };
        let mut render = None;
        let mut save = None;
        let mut export = None;
        let viewer_key = self.key(asset, Purpose::Viewer);
        let editing_available = matches!(
            self.cache.get(&viewer_key),
            Some(Cached::Image {
                viewer_pixels: Some(_),
                ..
            })
        );
        ui.horizontal_wrapped(|ui| {
            let Some(document) = self.documents.get_mut(&asset) else {
                ui.label("Read-only");
                return;
            };
            match document {
                EditorDocument::ReadOnly(message) => {
                    ui.colored_label(Color32::from_rgb(235, 156, 130), "Read-only")
                        .on_hover_text(message.as_str());
                }
                EditorDocument::Ready {
                    session,
                    source,
                    source_error,
                } => {
                    let mut exposure = session.recipe().exposure().value();
                    let slider = ui.add_enabled(
                        editing_available,
                        egui::Slider::new(&mut exposure, -500..=500)
                            .step_by(5.0)
                            .text("Exposure (EV)")
                            .custom_formatter(|value, _| format!("{:+.2}", value / 100.0)),
                    );
                    if slider.changed()
                        && let Ok(exposure) = ExposureCentistops::new(exposure)
                        && session.apply(EditCommand::SetExposure(exposure)).is_some()
                    {
                        render = Some(if slider.dragged() {
                            RenderQuality::Interactive
                        } else {
                            RenderQuality::Settled
                        });
                    }
                    if slider.drag_stopped() {
                        render = Some(RenderQuality::Settled);
                    }
                    if ui
                        .add_enabled(editing_available, egui::Button::new("Reset"))
                        .clicked()
                        && session.apply(EditCommand::ResetExposure).is_some()
                    {
                        render = Some(RenderQuality::Settled);
                    }

                    let mut before = self.comparison == Comparison::Before;
                    if ui
                        .add_enabled(
                            editing_available,
                            egui::Button::selectable(before, "Before"),
                        )
                        .clicked()
                    {
                        before = !before;
                        self.comparison = if before {
                            Comparison::Before
                        } else {
                            Comparison::After
                        };
                    }
                    if !editing_available {
                        ui.colored_label(
                            Color32::from_rgb(235, 156, 130),
                            "Editing unavailable because viewer pixels are unavailable",
                        );
                    }

                    let state = session.save_state();
                    let label = if source.is_none() {
                        if session.is_dirty() {
                            "Read-only · Unsaved changes".to_owned()
                        } else {
                            "Read-only".to_owned()
                        }
                    } else {
                        save_state_label(&state, session.is_dirty())
                    };
                    let state_label = ui.label(label);
                    if let Some(error) = source_error {
                        state_label.on_hover_text(error.as_str());
                    }
                    let save_enabled = source.is_some()
                        && session.is_dirty()
                        && !matches!(
                            state,
                            SaveState::Saving { .. }
                                | SaveState::ReadOnly(_)
                                | SaveState::Conflict(_)
                        );
                    if ui
                        .add_enabled(save_enabled, egui::Button::new("Save XMP"))
                        .clicked()
                        && let (Some(source), Ok(Some(command))) =
                            (source.clone(), session.begin_save())
                    {
                        save = Some(SaveRequest::new(asset, source, command));
                    }

                    ui.separator();
                    ui.label("SDR JPEG, max 4096 px");
                    let exporting =
                        matches!(self.exports.get(&asset), Some(ExportState::Exporting));
                    if ui
                        .add_enabled(
                            source.is_some() && !exporting,
                            egui::Button::new("Export JPEG"),
                        )
                        .clicked()
                        && let Some(source) = source.clone()
                    {
                        export = Some((source, session.snapshot()));
                    }
                }
            }
        });

        if let Some(quality) = render {
            self.schedule_render(asset, quality);
        }
        if let Some(request) = save
            && self.editor.submit_save(request).is_ok()
        {
            self.metrics
                .record("save_requested", Some(viewer_key), 0, 0, 1);
        }
        if let Some((source, snapshot)) = export {
            let Some(index) = self.workspace.index.get(&asset).copied() else {
                return;
            };
            let format = *self.workspace.assets[index].kind();
            match ExportRequest::new(asset, source, format, snapshot) {
                Ok(request) => {
                    self.exports.insert(asset, ExportState::Exporting);
                    if let Err(error) = self.editor.submit_export(request) {
                        self.exports
                            .insert(asset, ExportState::Failed(error.to_string()));
                    } else {
                        self.metrics
                            .record("export_requested", Some(viewer_key), 0, 0, 1);
                    }
                }
                Err(error) => {
                    self.exports
                        .insert(asset, ExportState::Failed(error.to_string()));
                }
            }
        }
        match self.exports.get(&asset) {
            Some(ExportState::Exporting) => {
                ui.label("Exporting");
            }
            Some(ExportState::Exported(message)) => {
                ui.label(message);
            }
            Some(ExportState::Failed(message)) => {
                ui.colored_label(Color32::from_rgb(235, 156, 130), message);
            }
            Some(ExportState::Idle) | None => {}
        }
    }

    fn receive(&mut self, context: &egui::Context) {
        for _ in 0..32 {
            let Some(event) = self.jobs.try_recv() else {
                return;
            };
            match event {
                Event::Candidate {
                    generation,
                    candidate,
                } => {
                    self.workspace.insert(generation, candidate);
                }
                Event::ScanFinished {
                    generation,
                    failures,
                } if generation == self.workspace.generation => {
                    self.workspace.scanning = false;
                    self.workspace.failures = failures;
                    self.metrics.record(
                        "scan_finished",
                        None,
                        generation,
                        0,
                        self.workspace.assets.len() as u64,
                    );
                }
                Event::ScanFinished { .. } => {}
                Event::Decoded {
                    key,
                    outcome,
                    source_stamp,
                } if self.workspace.accepts(key) => {
                    let cached = match outcome {
                        DecodeOutcome::Decoded(result) => {
                            self.metrics.record("gui_received", Some(key), 0, 0, 1);
                            let DecodeResult {
                                preview,
                                metadata,
                                provenance,
                            } = result;
                            let incoming = preview.rgba8().len()
                                * if key.purpose == Purpose::Viewer { 3 } else { 1 };
                            self.evict(key.purpose, incoming);
                            let upload_started = Instant::now();
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                preview.dimensions_usize(),
                                preview.rgba8(),
                            );
                            let texture = context.load_texture(
                                format!("{}-{:?}", key.asset, key.purpose),
                                image,
                                TextureOptions::LINEAR,
                            );
                            self.metrics.record(
                                "texture_upload_call_us",
                                Some(key),
                                0,
                                0,
                                upload_started.elapsed().as_micros() as u64,
                            );
                            let viewer_pixels = retain_viewer_pixels(key.purpose, preview);
                            Cached::Image {
                                texture,
                                viewer_pixels,
                                metadata,
                                provenance,
                                used: self.tick,
                            }
                        }
                        DecodeOutcome::Unsupported(reason) => Cached::Unavailable {
                            reason,
                            failure: None,
                            used: self.tick,
                        },
                        DecodeOutcome::Failed(error) => Cached::Unavailable {
                            reason: error.to_string(),
                            failure: Some(error.class),
                            used: self.tick,
                        },
                    };
                    self.cache.insert(key, cached);
                    if key.purpose == Purpose::Viewer {
                        self.ensure_document(key.asset);
                        self.set_source_identity(key.asset, source_stamp);
                        let epoch = self.input_epochs.entry(key.asset).or_insert(0);
                        *epoch = epoch.checked_add(1).expect("viewer input epoch exhausted");
                        self.edited_previews.remove(&key.asset);
                        self.schedule_render(key.asset, RenderQuality::Settled);
                    }
                    while let Some(oldest) = unavailable_to_evict(&self.cache, key.purpose, 256) {
                        self.remove_cache_entry(oldest);
                    }
                }
                Event::Decoded { .. } => {}
            }
        }
        context.request_repaint();
    }

    fn evict(&mut self, purpose: Purpose, incoming: usize) {
        loop {
            let entries: Vec<_> = self
                .cache
                .iter()
                .filter(|(key, value)| key.purpose == purpose && value.bytes() != 0)
                .collect();
            let bytes: usize = entries.iter().map(|(_, value)| value.bytes()).sum();
            let over = match purpose {
                Purpose::Thumbnail => bytes + incoming > 128 * 1024 * 1024,
                Purpose::Viewer => entries.len() >= 2,
            };
            if !over {
                break;
            }
            let oldest = entries
                .into_iter()
                .min_by_key(|(_, value)| match value {
                    Cached::Image { used, .. } => *used,
                    _ => 0,
                })
                .map(|(key, _)| *key);
            if let Some(key) = oldest {
                self.remove_cache_entry(key);
            } else {
                break;
            }
        }
    }

    fn demand(&mut self, mut keys: Vec<JobKey>) {
        keys.dedup();
        let (selected, thumbnails) = self.workspace.demand_keys(keys);
        let request = |key: JobKey| {
            self.workspace.index.get(&key.asset).map(|index| {
                let candidate = &self.workspace.assets[*index];
                PreviewRequest {
                    key,
                    path: candidate.path().to_owned(),
                    format: *candidate.kind(),
                    needed: !self.cache.contains_key(&key),
                }
            })
        };
        self.jobs.replace(PreviewDemand {
            selected: selected.and_then(request),
            thumbnails: thumbnails.into_iter().filter_map(request).collect(),
        });
    }

    fn retry_key(&self) -> Option<JobKey> {
        let asset = self.workspace.selected?;
        let purpose = match self.workspace.view {
            View::Grid => Purpose::Thumbnail,
            View::Viewer => Purpose::Viewer,
        };
        let key = self.key(asset, purpose);
        self.cache
            .get(&key)
            .is_some_and(Cached::retryable)
            .then_some(key)
    }

    fn retry_selected(&mut self) {
        if let Some(key) = self.retry_key() {
            self.jobs.forget(&[key]);
            self.remove_cache_entry(key);
        }
    }

    fn measure_selection(&mut self) {
        let key = self.workspace.selected.map(|asset| {
            self.key(
                asset,
                if self.workspace.view == View::Viewer {
                    Purpose::Viewer
                } else {
                    Purpose::Thumbnail
                },
            )
        });
        if key != self.measured_selection {
            self.measured_selection = key;
            self.drawn.clear();
            self.metrics.record("gui_selection", key, 0, 0, 1);
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> Vec<JobKey> {
        let columns = ((ui.available_width() + 12.0) / (self.thumbnail_width + 12.0))
            .floor()
            .max(1.0) as usize;
        let row_height = self.thumbnail_width * 0.78 + 50.0;
        let row_count = self.workspace.assets.len().div_ceil(columns);
        let mut demanded = Vec::new();
        let mut scroll = egui::ScrollArea::vertical().id_salt("photo-grid");
        if self.reveal_selected
            && let Some(index) = self
                .workspace
                .selected
                .and_then(|asset| self.workspace.index.get(&asset).copied())
        {
            scroll =
                scroll.vertical_scroll_offset(reveal_scroll_offset(index, columns, row_height));
        }
        scroll.show_rows(ui, row_height, row_count, |ui, rows| {
            for row in rows {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let index = row * columns + column;
                        let Some(candidate) = self.workspace.assets.get(index) else {
                            break;
                        };
                        let id = candidate.id();
                        let filename = candidate
                            .path()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let format = candidate.kind().to_string();
                        let key = self.key(id, Purpose::Thumbnail);
                        demanded.push(key);
                        let selected = self.workspace.selected == Some(id);
                        let response = ui
                            .allocate_ui_with_layout(
                                Vec2::new(self.thumbnail_width, row_height),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let (_, rect) = ui.allocate_space(Vec2::new(
                                        self.thumbnail_width,
                                        self.thumbnail_width * 0.78,
                                    ));
                                    let mut response = ui.interact(
                                        rect,
                                        thumbnail_access_id(id),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        4.0,
                                        Color32::from_rgb(20, 21, 20),
                                    );
                                    let mut unavailable = None;
                                    match self.cache.get_mut(&key) {
                                        Some(Cached::Image { texture, used, .. }) => {
                                            *used = self.tick;
                                            let size = texture.size_vec2();
                                            let scale =
                                                (rect.width() / size.x).min(rect.height() / size.y);
                                            let target = egui::Rect::from_center_size(
                                                rect.center(),
                                                size * scale,
                                            );
                                            ui.painter().image(
                                                texture.id(),
                                                target,
                                                egui::Rect::from_min_max(
                                                    egui::Pos2::ZERO,
                                                    egui::pos2(1.0, 1.0),
                                                ),
                                                Color32::WHITE,
                                            );
                                            if selected && self.drawn.insert(Purpose::Thumbnail) {
                                                self.metrics.record(
                                                    "first_selected_draw",
                                                    Some(key),
                                                    0,
                                                    0,
                                                    1,
                                                );
                                            }
                                        }
                                        Some(Cached::Unavailable { reason, used, .. }) => {
                                            *used = self.tick;
                                            unavailable = Some(concise_reason(reason));
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "Preview unavailable",
                                                egui::FontId::proportional(13.0),
                                                Color32::GRAY,
                                            );
                                        }
                                        None => {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "Loading preview",
                                                egui::FontId::proportional(13.0),
                                                Color32::GRAY,
                                            );
                                        }
                                    }
                                    if selected {
                                        ui.painter().rect_stroke(
                                            rect,
                                            4.0,
                                            egui::Stroke::new(
                                                2.0,
                                                Color32::from_rgb(161, 181, 150),
                                            ),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if selected && self.reveal_selected {
                                        response.request_focus();
                                        self.reveal_selected = false;
                                        self.metrics.record("keyboard_reveal", Some(key), 0, 0, 1);
                                    }
                                    if response.has_focus() {
                                        ui.painter().rect_stroke(
                                            rect.shrink(3.0),
                                            3.0,
                                            egui::Stroke::new(3.0, Color32::WHITE),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    let accessible_name = match unavailable {
                                        Some(reason) => {
                                            response = response.on_hover_text(&reason);
                                            format!("{filename}. Preview unavailable. {reason}")
                                        }
                                        None if !self.cache.contains_key(&key) => {
                                            format!("{filename}. Loading preview")
                                        }
                                        None => filename.clone(),
                                    };
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::Button,
                                            true,
                                            selected,
                                            &accessible_name,
                                        )
                                    });
                                    ui.ctx().accesskit_node_builder(response.id, |node| {
                                        node.set_author_id(thumbnail_author_id(id));
                                    });
                                    ui.add(egui::Label::new(&filename).truncate());
                                    ui.label(RichText::new(format).small().color(Color32::GRAY));
                                    response
                                },
                            )
                            .inner;
                        if response.clicked() {
                            self.workspace.selected = Some(id);
                            self.measure_selection();
                        }
                        if response.double_clicked() {
                            self.workspace.selected = Some(id);
                            self.workspace.view = View::Viewer;
                            self.measure_selection();
                        }
                    }
                });
            }
        });
        if let (Some(first), Some(last)) = (demanded.first(), demanded.last()) {
            self.metrics.record(
                "grid_visible",
                None,
                self.workspace.index[&first.asset] as u64,
                self.workspace.index[&last.asset] as u64,
                demanded.len() as u64,
            );
        }
        demanded
    }

    fn viewer(&mut self, ui: &mut egui::Ui) -> Vec<JobKey> {
        let Some(id) = self.workspace.selected else {
            return Vec::new();
        };
        let key = self.key(id, Purpose::Viewer);
        let thumbnail = self.key(id, Purpose::Thumbnail);
        let display_key = viewer_display_key(&self.cache, key, thumbnail);
        let filename = self.workspace.assets[self.workspace.index[&id]]
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let larger_error = match self.cache.get_mut(&key) {
            Some(Cached::Unavailable { reason, used, .. }) => {
                *used = self.tick;
                Some(concise_reason(reason))
            }
            _ => None,
        };
        if display_key == thumbnail
            && let Some(reason) = &larger_error
        {
            ui.colored_label(
                Color32::from_rgb(235, 156, 130),
                format!("Larger preview unavailable: {reason}"),
            );
        }
        let use_original = self
            .documents
            .get(&id)
            .and_then(|document| match document {
                EditorDocument::Ready { session, .. } => {
                    Some(self.comparison.uses_original(session.recipe()))
                }
                EditorDocument::ReadOnly(_) => None,
            })
            .unwrap_or(true);
        let edited_texture = (!use_original)
            .then(|| self.edited_previews.get(&id))
            .flatten()
            .filter(|preview| {
                self.render_demands
                    .get(&id)
                    .is_some_and(|demand| *demand == preview.demand)
            })
            .map(|preview| preview.texture.clone());
        match self.cache.get_mut(&display_key) {
            Some(Cached::Image {
                texture,
                metadata,
                provenance,
                used,
                ..
            }) => {
                *used = self.tick;
                ui.label(format!(
                    "{provenance}  ·  {} × {}  ·  {}",
                    metadata.decoded_dimensions[0],
                    metadata.decoded_dimensions[1],
                    metadata.orientation
                ));
                ui.label(
                    RichText::new(&metadata.limitations)
                        .small()
                        .color(Color32::from_rgb(181, 172, 145)),
                );
                if display_key != key && larger_error.is_none() {
                    ui.label("Loading larger preview");
                }
                if !use_original && edited_texture.is_none() {
                    ui.label("Rendering edit");
                }
                let texture = edited_texture.as_ref().unwrap_or(texture);
                let available = ui.available_size();
                let source = texture.size_vec2();
                if self.actual_pixels {
                    egui::ScrollArea::both()
                        .id_salt(("actual-preview", id))
                        .show(ui, |ui| {
                            let response = ui
                                .push_id(viewer_access_id(id), |ui| {
                                    ui.add(
                                        egui::Image::new((texture.id(), source))
                                            .alt_text(format!("Preview of {filename}")),
                                    )
                                })
                                .inner;
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_author_id(viewer_author_id(id));
                            });
                        });
                } else {
                    let scale = (available.x / source.x)
                        .min(available.y / source.y)
                        .max(0.01);
                    ui.centered_and_justified(|ui| {
                        let response = ui
                            .push_id(viewer_access_id(id), |ui| {
                                ui.add(
                                    egui::Image::new((texture.id(), source * scale))
                                        .alt_text(format!("Preview of {filename}")),
                                )
                            })
                            .inner;
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_author_id(viewer_author_id(id));
                        });
                    });
                }
                if self.drawn.insert(display_key.purpose) {
                    self.metrics
                        .record("first_selected_draw", Some(display_key), 0, 0, 1);
                }
            }
            Some(Cached::Unavailable { reason, used, .. }) => {
                *used = self.tick;
                ui.centered_and_justified(|ui| {
                    ui.label(concise_reason(reason));
                });
            }
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("Loading preview");
                });
            }
        }
        vec![key]
    }
}

impl eframe::App for Browser {
    fn on_exit(&mut self) {
        self.jobs.shutdown();
        self.editor.shutdown();
        if let Some(path) = self.metric_path.take() {
            self.metrics.record("gui_exit", None, 0, 0, 1);
            if let Err(error) = self.metrics.save(&path) {
                eprintln!("metrics {}: {error}", path.display());
            }
        }
    }

    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(context);
        self.receive_editor(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let frame_started = Instant::now();
        self.tick += 1;
        self.handle_close(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Crema");
                ui.separator();
                if ui
                    .selectable_label(self.workspace.view == View::Grid, "Browse")
                    .clicked()
                {
                    self.workspace.view = View::Grid;
                }
                if ui
                    .selectable_label(self.workspace.view == View::Viewer, "View photo")
                    .clicked()
                {
                    self.workspace.view = View::Viewer;
                }
                let can_retry = self.retry_key().is_some();
                if can_retry && ui.button("Retry selected preview").clicked() {
                    self.retry_selected();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.workspace.view == View::Viewer {
                        ui.toggle_value(&mut self.actual_pixels, "100% preview");
                    } else {
                        ui.add(
                            egui::Slider::new(&mut self.thumbnail_width, 140.0..=300.0)
                                .text("Thumbnails"),
                        );
                    }
                });
            });
            ui.label(
                RichText::new(self.root.to_string_lossy())
                    .small()
                    .color(Color32::GRAY),
            );
            ui.separator();
            let wants_keyboard_input = ui.ctx().egui_wants_keyboard_input();
            for key in [
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::Enter,
                egui::Key::Escape,
            ] {
                if ui.input(|input| input.key_pressed(key))
                    && self.workspace.shortcut(key, wants_keyboard_input)
                {
                    self.reveal_selected = true;
                }
            }
            ui.horizontal(|ui| {
                let suffix = if self.workspace.scanning {
                    " · Scanning folder"
                } else {
                    ""
                };
                ui.label(format!("{} photos{suffix}", self.workspace.assets.len()));
                if let Some(candidate) = self
                    .workspace
                    .selected
                    .and_then(|id| self.workspace.index.get(&id))
                    .map(|index| &self.workspace.assets[*index])
                {
                    ui.separator();
                    ui.label(
                        candidate
                            .path()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    );
                }
            });
            for failure in &self.workspace.failures {
                ui.colored_label(Color32::from_rgb(235, 156, 130), failure);
            }
            if self.workspace.assets.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.workspace.scanning {
                        "Reading folder"
                    } else if self.workspace.failures.is_empty() {
                        "No supported photo candidates in this folder"
                    } else {
                        "Folder could not be fully read"
                    });
                });
                self.demand(Vec::new());
                return;
            }
            self.measure_selection();
            let active_viewer = (self.workspace.view == View::Viewer)
                .then_some(self.workspace.selected)
                .flatten();
            self.activate_viewer(active_viewer);
            if active_viewer.is_some() {
                self.editor_controls(ui);
                ui.separator();
            }
            let demands = match self.workspace.view {
                View::Grid => self.grid(ui),
                View::Viewer => self.viewer(ui),
            };
            self.demand(demands);
        });
        self.metrics.record(
            "gui_frame_us",
            self.workspace.selected.map(|asset| {
                self.key(
                    asset,
                    if self.workspace.view == View::Viewer {
                        Purpose::Viewer
                    } else {
                        Purpose::Thumbnail
                    },
                )
            }),
            0,
            0,
            frame_started.elapsed().as_micros() as u64,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crema_core::{ScanEvent, scan_folder};
    use crema_image::classify_candidate;
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[test]
    fn progressive_inserts_and_stale_results_preserve_selection() {
        let root = std::env::temp_dir().join(format!("crema-state-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for name in ["one.jpg", "two.png", "three.tiff"] {
            fs::write(root.join(name), []).unwrap();
        }
        let mut workspace = Workspace::default();
        let mut candidates = scan_folder(&root, classify_candidate)
            .unwrap()
            .filter_map(|event| match event {
                ScanEvent::Candidate(candidate) => Some(candidate),
                _ => None,
            });
        workspace.insert(1, candidates.next().unwrap());
        let selected = workspace.selected;
        workspace.insert(1, candidates.next().unwrap());
        assert_eq!(workspace.selected, selected);
        workspace.insert(0, candidates.next().unwrap());
        assert_eq!(workspace.assets.len(), 2);
        assert!(!workspace.accepts(JobKey {
            generation: 0,
            asset: selected.unwrap(),
            purpose: Purpose::Viewer
        }));
        assert_eq!(workspace.selected, selected);
        workspace.navigate(1);
        assert_ne!(workspace.selected, selected);
        workspace.navigate(999);
        assert_eq!(workspace.selected, Some(workspace.assets[1].id()));
        workspace.navigate(-1);
        assert!(workspace.shortcut(egui::Key::ArrowRight, true));
        assert_eq!(workspace.selected, Some(workspace.assets[1].id()));
        workspace.navigate(-1);
        workspace.shortcut(egui::Key::Enter, true);
        assert_eq!(workspace.view, View::Viewer);
        let first = workspace.selected;
        assert!(!workspace.shortcut(egui::Key::ArrowRight, true));
        assert_eq!(workspace.selected, first);
        workspace.shortcut(egui::Key::Escape, true);
        assert_eq!(workspace.view, View::Grid);
        assert!(workspace.shortcut(egui::Key::ArrowRight, false));
        assert_eq!(workspace.selected, Some(workspace.assets[1].id()));
        assert!(!workspace.shortcut(egui::Key::ArrowRight, false));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accessibility_ids_and_keyboard_reveal_are_stable() {
        let first = test_asset();
        let second = test_asset();
        assert_eq!(thumbnail_access_id(first), thumbnail_access_id(first));
        assert_ne!(thumbnail_access_id(first), thumbnail_access_id(second));
        assert_ne!(thumbnail_access_id(first), viewer_access_id(first));
        assert_eq!(thumbnail_author_id(first), format!("thumbnail-{first}"));
        assert_eq!(viewer_author_id(first), format!("viewer-image-{first}"));
        assert_eq!(reveal_scroll_offset(7, 3, 100.0), 200.0);
    }

    fn test_asset() -> AssetId {
        let root = std::env::temp_dir().join(format!(
            "crema-cache-state-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("photo.jpg"), []).unwrap();
        let ScanEvent::Candidate(candidate) = scan_folder(&root, classify_candidate)
            .unwrap()
            .next()
            .unwrap()
        else {
            panic!("real candidate");
        };
        fs::remove_dir_all(root).unwrap();
        candidate.id()
    }

    fn unavailable(used: u64) -> Cached {
        Cached::Unavailable {
            reason: "timeout: worker deadline exceeded".into(),
            failure: Some(FailureClass::Timeout),
            used,
        }
    }

    #[test]
    fn unavailable_cache_is_capped_per_purpose_and_evicts_least_recently_used() {
        let asset = test_asset();
        let key = |generation, purpose| JobKey {
            generation,
            asset,
            purpose,
        };
        let oldest = key(1, Purpose::Thumbnail);
        let recent = key(2, Purpose::Thumbnail);
        let newest = key(3, Purpose::Thumbnail);
        let viewer = key(1, Purpose::Viewer);
        let mut cache = HashMap::from([
            (oldest, unavailable(1)),
            (recent, unavailable(10)),
            (newest, unavailable(20)),
            (viewer, unavailable(0)),
        ]);
        assert_eq!(
            unavailable_to_evict(&cache, Purpose::Thumbnail, 2),
            Some(oldest)
        );
        if let Cached::Unavailable { used, .. } = cache.get_mut(&oldest).unwrap() {
            *used = 30;
        }
        assert_eq!(
            unavailable_to_evict(&cache, Purpose::Thumbnail, 2),
            Some(recent)
        );
        cache.remove(&recent);
        assert_eq!(unavailable_to_evict(&cache, Purpose::Thumbnail, 2), None);
        assert_eq!(
            unavailable_to_evict(&cache, Purpose::Viewer, 0),
            Some(viewer)
        );
    }

    #[test]
    fn browser_retains_cpu_pixels_only_for_viewers_and_checks_exact_render_demand() {
        let asset = test_asset();
        let pixels = || {
            PreviewPixels::new(
                2,
                1,
                vec![10, 20, 30, 255, 40, 50, 60, 255],
                PreviewSize::new(2).unwrap(),
            )
            .unwrap()
        };
        assert!(retain_viewer_pixels(Purpose::Thumbnail, pixels()).is_none());
        let retained = retain_viewer_pixels(Purpose::Viewer, pixels()).unwrap();
        assert_eq!(retained.detail.rgba8(), retained.interactive.rgba8());

        let current = RenderKey::new(
            asset,
            crema_core::edit::EditRevision::ZERO,
            2,
            RenderQuality::Settled,
        );
        let demands = HashMap::from([(asset, RenderDemand::new(current))]);
        assert!(accepts_render(&demands, current));
        assert!(!accepts_render(
            &demands,
            RenderKey::new(
                asset,
                crema_core::edit::EditRevision::ZERO,
                1,
                RenderQuality::Settled,
            )
        ));
        let other = test_asset();
        assert!(needs_activation_render(Some(other), asset, true, false));
        assert!(!needs_activation_render(Some(asset), asset, true, false));
        assert!(!needs_activation_render(Some(other), asset, false, false));
        assert!(!needs_activation_render(Some(other), asset, true, true));
        assert!(cancel_close(true, true, CloseConfirmation::Inactive));
        assert!(cancel_close(true, true, CloseConfirmation::Prompting));
        assert!(!cancel_close(true, true, CloseConfirmation::Discarding));
        assert!(!cancel_close(true, false, CloseConfirmation::Inactive));
    }

    #[test]
    fn larger_preview_failure_preserves_thumbnail_and_retry_clears_only_selection() {
        let context = egui::Context::default();
        let wake = context.clone();
        let jobs = PreviewRuntime::new(std::env::current_exe().unwrap(), move || {
            wake.request_repaint()
        });
        let asset = test_asset();
        let mut browser = Browser {
            root: PathBuf::new(),
            workspace: Workspace {
                selected: Some(asset),
                ..Workspace::default()
            },
            jobs,
            cache: HashMap::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
            metrics: Metrics::default(),
            metric_path: None,
            measured_selection: None,
            drawn: HashSet::new(),
            editor: EditorRuntime::new(std::env::current_exe().unwrap(), || {}),
            documents: HashMap::new(),
            input_epochs: HashMap::new(),
            render_demands: HashMap::new(),
            edited_previews: HashMap::new(),
            comparison: Comparison::After,
            exports: HashMap::new(),
            active_viewer: None,
            close_confirmation: CloseConfirmation::Inactive,
            reveal_selected: false,
        };
        let thumbnail = browser.key(asset, Purpose::Thumbnail);
        let viewer = browser.key(asset, Purpose::Viewer);
        let other = JobKey {
            asset: test_asset(),
            ..viewer
        };
        let texture = context.load_texture(
            "test-thumbnail",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            TextureOptions::LINEAR,
        );
        browser.cache.insert(
            thumbnail,
            Cached::Image {
                texture,
                viewer_pixels: None,
                metadata: SourceMetadata {
                    dimensions: [1, 1],
                    decoded_dimensions: [1, 1],
                    source_bits: crema_image::Fact::Known(8),
                    decoded_bits: 8,
                    orientation: crema_image::Orientation::Exif(1),
                    icc: crema_image::Fact::Known(crema_image::Icc::Absent),
                    nclx: None,
                    camera_make: String::new(),
                    camera_model: String::new(),
                    limitations: String::new(),
                },
                provenance: Provenance::JpegDecode,
                used: 1,
            },
        );
        browser.cache.insert(viewer, unavailable(2));
        browser.cache.insert(
            other,
            Cached::Unavailable {
                reason: "unsupported".into(),
                failure: None,
                used: 0,
            },
        );
        assert_eq!(
            viewer_display_key(&browser.cache, viewer, thumbnail),
            thumbnail
        );
        assert!(browser.cache[&viewer].retryable());
        assert!(!browser.cache[&other].retryable());
        assert_eq!(
            unavailable_to_evict(&browser.cache, Purpose::Thumbnail, 0),
            None
        );
        assert_eq!(
            browser.retry_key(),
            None,
            "grid must not offer retry for a viewer failure"
        );
        browser.retry_selected();
        assert!(browser.cache.contains_key(&viewer));
        browser.workspace.view = View::Viewer;
        assert_eq!(browser.retry_key(), Some(viewer));
        browser.retry_selected();
        assert!(!browser.cache.contains_key(&viewer));
        assert!(
            browser.cache.contains_key(&thumbnail),
            "retry must preserve the valid thumbnail"
        );
        assert!(browser.cache.contains_key(&other));
        assert_eq!(browser.workspace.selected, Some(asset));
        let image = browser.cache.remove(&thumbnail).unwrap();
        browser.cache.insert(viewer, image);
        browser.cache.insert(thumbnail, unavailable(3));
        assert_eq!(
            browser.retry_key(),
            None,
            "viewer must not offer retry for a thumbnail failure"
        );
        browser.retry_selected();
        assert!(browser.cache.contains_key(&thumbnail));
        browser.workspace.view = View::Grid;
        assert_eq!(browser.retry_key(), Some(thumbnail));
        browser.retry_selected();
        assert!(
            browser.cache.contains_key(&viewer),
            "retry must preserve the valid viewer"
        );
        assert!(!browser.cache.contains_key(&thumbnail));
        browser.cache.insert(thumbnail, unavailable(4));
        browser.cache.insert(viewer, unavailable(5));
        browser.retry_selected();
        assert!(!browser.cache.contains_key(&thumbnail));
        assert!(
            browser.cache.contains_key(&viewer),
            "retry only invalidates the current view's purpose"
        );
        let render_key = RenderKey::new(
            asset,
            crema_core::edit::EditRevision::ZERO,
            1,
            RenderQuality::Settled,
        );
        let demand = RenderDemand::new(render_key);
        browser.render_demands.insert(asset, demand);
        browser.edited_previews.insert(
            asset,
            EditedPreview {
                demand,
                texture: context.load_texture(
                    "edited-viewer",
                    egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
                    TextureOptions::LINEAR,
                ),
            },
        );
        browser.remove_cache_entry(viewer);
        assert!(!browser.cache.contains_key(&viewer));
        assert!(!browser.render_demands.contains_key(&asset));
        assert!(!browser.edited_previews.contains_key(&asset));
    }

    #[test]
    fn unavailable_reasons_are_concise_and_only_transient_failures_offer_retry() {
        assert_eq!(
            concise_reason("timeout: worker deadline exceeded\nchild diagnostics"),
            "timeout: worker deadline exceeded"
        );
        assert_eq!(concise_reason(&"é".repeat(200)).chars().count(), 160);
        for class in [
            FailureClass::Io,
            FailureClass::Timeout,
            FailureClass::Cancelled,
            FailureClass::WorkerExited,
        ] {
            assert!(
                Cached::Unavailable {
                    reason: class.to_string(),
                    failure: Some(class),
                    used: 0
                }
                .retryable()
            );
        }
        for class in [
            FailureClass::Codec,
            FailureClass::InvalidInput,
            FailureClass::Protocol,
            FailureClass::LimitExceeded,
            FailureClass::UnsupportedColor,
        ] {
            assert!(
                !Cached::Unavailable {
                    reason: class.to_string(),
                    failure: Some(class),
                    used: 0
                }
                .retryable()
            );
        }
    }
    #[test]
    fn offscreen_selection_is_demanded_independently_of_visible_rows() {
        let asset = test_asset();
        let workspace = Workspace {
            selected: Some(asset),
            ..Workspace::default()
        };
        let (selected, thumbnails) = workspace.demand_keys(Vec::new());
        assert_eq!(
            selected,
            Some(JobKey {
                generation: 1,
                asset,
                purpose: Purpose::Thumbnail
            })
        );
        assert!(thumbnails.is_empty());
    }

    #[test]
    fn on_exit_saves_configured_metrics_once_without_overwriting() {
        let root = std::env::temp_dir().join(format!("crema-gui-exit-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for existing in [false, true] {
            let path = root.join(if existing {
                "existing.tsv"
            } else {
                "fresh.tsv"
            });
            if existing {
                fs::write(&path, b"existing user metrics").unwrap();
            }
            let metrics = Metrics::new(true);
            metrics.record("gui_launch", None, 0, 0, 1);
            let mut browser = Browser::with_options(
                root.clone(),
                std::env::current_exe().unwrap(),
                egui::Context::default(),
                CacheConfig {
                    root: None,
                    budget: 0,
                },
                metrics.clone(),
                Some(path.clone()),
            );
            eframe::App::on_exit(&mut browser);
            assert!(
                path.exists(),
                "native on_exit must save configured GUI metrics"
            );
            let saved = fs::read(&path).unwrap();
            if existing {
                assert_eq!(saved, b"existing user metrics");
            } else {
                assert!(String::from_utf8_lossy(&saved).contains("\tgui_launch\t"));
            }
            metrics.record("after_exit", None, 0, 0, 1);
            eframe::App::on_exit(&mut browser);
            assert_eq!(fs::read(&path).unwrap(), saved);
            drop(browser);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn idle_logic_does_not_request_repaint() {
        let context = egui::Context::default();
        let wake = context.clone();
        let jobs = PreviewRuntime::new(std::env::current_exe().unwrap(), move || {
            wake.request_repaint()
        });
        let mut browser = Browser {
            root: PathBuf::new(),
            workspace: Workspace::default(),
            jobs,
            cache: HashMap::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
            metrics: Metrics::default(),
            metric_path: None,
            measured_selection: None,
            drawn: HashSet::new(),
            editor: EditorRuntime::new(std::env::current_exe().unwrap(), || {}),
            documents: HashMap::new(),
            input_epochs: HashMap::new(),
            render_demands: HashMap::new(),
            edited_previews: HashMap::new(),
            comparison: Comparison::After,
            exports: HashMap::new(),
            active_viewer: None,
            close_confirmation: CloseConfirmation::Inactive,
            reveal_selected: false,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        context.set_request_repaint_callback(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        });
        let mut frame = eframe::Frame::_new_kittest();
        for _ in 0..100 {
            eframe::App::logic(&mut browser, &context, &mut frame);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
    }
}
