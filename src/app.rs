use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use iced::{Element, Task, Theme};
use tracing::{error, info};

use crema_catalog::db::Catalog;
use crema_catalog::models::{Photo, PhotoId};
use crema_core::image_buf::{EditParams, ImageBuf};
use crema_gpu::context::GpuContext;
use crema_gpu::pipeline::GpuPipeline;
use crema_thumbnails::cache::ThumbnailCache;

type GpuHandle = Arc<std::sync::Mutex<(GpuContext, GpuPipeline)>>;

#[derive(Clone)]
pub(crate) struct GpuReady(GpuHandle);
impl std::fmt::Debug for GpuReady {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GpuReady")
    }
}

use crate::views;
use crate::widgets::date_sidebar::{DateExpansionKey, DateFilter, RatingFilter, SortOrder};
use crate::widgets::histogram::HistogramData;
use crate::widgets::zoomable_image::ZoomState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Library,
    Develop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelSection {
    Histogram,
    Light,
    Color,
    Hsl,
    SplitTone,
    Detail,
    Crop,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSection {
    Light,
    Color,
    Hsl,
    SplitTone,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditControl {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Blacks,
    WbTemp,
    WbTint,
    Vibrance,
    Saturation,
    HslHue,
    HslSaturation,
    HslLightness,
    SplitShadowHue,
    SplitShadowSat,
    SplitHighlightHue,
    SplitHighlightSat,
    SplitBalance,
    SharpenAmount,
    SharpenRadius,
}

const MAX_UNDO_HISTORY: usize = 100;

pub struct App {
    menu: Option<crate::menu::AppMenu>,
    workspace: Workspace,
    selected_photo: Option<PhotoId>,
    loaded_photo: Option<PhotoId>,
    right_panel_open: bool,
    catalog: Option<Catalog>,
    catalog_path: Option<String>,
    photos: Vec<Photo>,
    thumbnails: std::collections::HashMap<PhotoId, iced::widget::image::Handle>,

    current_image: Option<Arc<ImageBuf>>,
    preview_image: Option<Arc<ImageBuf>>,
    processed_image: Option<iced::widget::image::Handle>,
    histogram: Option<Box<HistogramData>>,
    edit_params: EditParams,
    current_exif: Vec<(String, String)>,

    undo_stack: Vec<EditParams>,
    redo_stack: Vec<EditParams>,
    edit_clipboard: Option<EditParams>,

    zoom_state: ZoomState,
    preview_dimensions: (u32, u32),
    original_display: Option<iced::widget::image::Handle>,
    showing_before: bool,
    crop_mode: bool,
    crop_aspect: Option<f32>,

    status_message: String,

    processing_generation: u64,
    thumbnail_cache_dir: Option<PathBuf>,
    is_importing: bool,
    is_exporting: bool,
    is_loading_photo: bool,
    is_processing: bool,

    gpu: Option<GpuHandle>,

    date_filter: DateFilter,
    rating_filter: RatingFilter,
    sort_order: SortOrder,
    expanded_dates: HashSet<DateExpansionKey>,
    panel_sections: HashSet<PanelSection>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPhoto(PhotoId),
    OpenPhoto(PhotoId),
    SetWorkspace(Workspace),
    ToggleRightPanel,

    Import,
    ImportsSelected(Vec<PathBuf>),
    ImportComplete(usize, usize),

    ThumbnailReady(PhotoId, Vec<u8>),

    ExposureChanged(f32),
    ContrastChanged(f32),
    HighlightsChanged(f32),
    ShadowsChanged(f32),
    BlacksChanged(f32),
    WbTempChanged(f32),
    WbTintChanged(f32),
    VibranceChanged(f32),
    SaturationChanged(f32),
    HslHueChanged(f32),
    HslSaturationChanged(f32),
    HslLightnessChanged(f32),
    SplitShadowHueChanged(f32),
    SplitShadowSatChanged(f32),
    SplitHighlightHueChanged(f32),
    SplitHighlightSatChanged(f32),
    SplitBalanceChanged(f32),
    SharpenAmountChanged(f32),
    SharpenRadiusChanged(f32),
    AutoEnhance,
    AutoEnhanceComplete(EditParams),
    ResetEdits,
    ResetControl(EditControl),
    ResetSection(EditSection),
    Undo,
    Redo,
    CopyEdits,
    PasteEdits,
    ZoomAtPoint(f32, f32, f32, f32, f32),
    PanDelta(f32, f32),
    ResetZoom,
    ToggleBeforeAfter,
    OriginalReady(iced::widget::image::Handle),
    NextPhoto,
    PrevPhoto,
    NudgeExposure(f32),
    RateAndAdvance(i32),
    DeletePhoto,
    ToggleCropMode,
    ExitCropMode,
    SetCropAspect(Option<f32>),
    UpdateCrop(f32, f32, f32, f32),
    ResetCrop,

    ImageLoaded(PhotoId, Arc<ImageBuf>, Arc<ImageBuf>, Vec<(String, String)>),
    ImageProcessed(u64, iced::widget::image::Handle, Box<HistogramData>),
    ImageLoadFailed(PhotoId),

    GpuInitDone(Option<GpuReady>),

    CatalogOpened(String),
    PhotosListed(Vec<Photo>),

    Export,
    ExportPathSelected(PathBuf),
    ExportComplete(String),

    SetDateFilter(DateFilter),
    SetRatingFilter(RatingFilter),
    SetSortOrder(SortOrder),
    ToggleDateExpansion(DateExpansionKey),
    TogglePanelSection(PanelSection),

    Noop,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            menu: None,
            workspace: Workspace::Library,
            selected_photo: None,
            loaded_photo: None,
            right_panel_open: true,
            catalog: None,
            catalog_path: None,
            photos: Vec::new(),
            thumbnails: std::collections::HashMap::new(),
            current_image: None,
            preview_image: None,
            processed_image: None,
            histogram: None,
            edit_params: EditParams::default(),
            current_exif: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            edit_clipboard: None,
            zoom_state: ZoomState::default(),
            preview_dimensions: (0, 0),
            original_display: None,
            showing_before: false,
            crop_mode: false,
            crop_aspect: None,
            status_message: "Welcome to Crema. Import photos to get started.".into(),
            processing_generation: 0,
            thumbnail_cache_dir: dirs::cache_dir().map(|d| d.join("crema").join("thumbnails")),
            is_importing: false,
            is_exporting: false,
            is_loading_photo: false,
            is_processing: false,
            gpu: None,

            date_filter: DateFilter::All,
            rating_filter: RatingFilter::All,
            sort_order: SortOrder::default(),
            expanded_dates: HashSet::new(),
            panel_sections: HashSet::from([
                PanelSection::Histogram,
                PanelSection::Light,
                PanelSection::Color,
            ]),
        };

        let default_catalog = dirs_catalog_path();
        let catalog_task = Task::perform(async move { default_catalog }, Message::CatalogOpened);

        let gpu_task = Task::perform(
            async {
                match GpuContext::new().await {
                    Ok(ctx) => {
                        let pipeline = GpuPipeline::new(&ctx);
                        Some(GpuReady(Arc::new(std::sync::Mutex::new((ctx, pipeline)))))
                    }
                    Err(e) => {
                        tracing::warn!("GPU init failed, using CPU pipeline: {e}");
                        None
                    }
                }
            },
            Message::GpuInitDone,
        );

        (app, Task::batch([catalog_task, gpu_task]))
    }

    pub fn title(&self) -> String {
        if let Some(id) = self.selected_photo {
            let name = self
                .photos
                .iter()
                .find(|p| p.id == id)
                .map(|p| {
                    std::path::Path::new(&p.file_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .unwrap_or_default();
            format!("Crema - {name}")
        } else {
            format!("Crema - {} photos", self.photos.len())
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.menu.is_none() {
            self.menu = Some(crate::menu::build());
            #[cfg(target_os = "macos")]
            crate::icon::set_dock_icon();
        }

        match message {
            Message::GpuInitDone(ready) => {
                if let Some(GpuReady(handle)) = ready {
                    info!("GPU pipeline ready");
                    self.gpu = Some(handle);
                }
                Task::none()
            }
            Message::CatalogOpened(path) => self.handle_catalog_opened(path),
            Message::Import => self.handle_import(),
            Message::ImportsSelected(paths) => self.handle_imports_selected(paths),
            Message::ImportComplete(imported, errors) => {
                self.handle_import_complete(imported, errors)
            }
            Message::PhotosListed(photos) => self.handle_photos_listed(photos),
            Message::ThumbnailReady(id, bytes) => self.handle_thumbnail_ready(id, bytes),
            Message::SelectPhoto(id) => self.handle_select_photo(id),
            Message::OpenPhoto(id) => self.open_photo(id),
            Message::SetWorkspace(workspace) => self.handle_set_workspace(workspace),
            Message::ToggleRightPanel => {
                self.right_panel_open = !self.right_panel_open;
                Task::none()
            }
            Message::ImageLoaded(id, buf, preview, exif) => {
                self.handle_image_loaded(id, buf, preview, exif)
            }
            Message::ImageProcessed(generation, handle, hist) => {
                self.handle_image_processed(generation, handle, hist)
            }
            Message::ImageLoadFailed(id) => self.handle_image_load_failed(id),
            Message::Export => self.handle_export(),
            Message::ExportPathSelected(path) => self.handle_export_path_selected(path),
            Message::ExportComplete(msg) => self.handle_export_complete(msg),
            Message::ExposureChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.exposure = v;
                self.reprocess_image()
            }
            Message::ContrastChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.contrast = v;
                self.reprocess_image()
            }
            Message::HighlightsChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.highlights = v;
                self.reprocess_image()
            }
            Message::ShadowsChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.shadows = v;
                self.reprocess_image()
            }
            Message::BlacksChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.blacks = v;
                self.reprocess_image()
            }
            Message::WbTempChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.wb_temp = v;
                self.reprocess_image()
            }
            Message::WbTintChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.wb_tint = v;
                self.reprocess_image()
            }
            Message::VibranceChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.vibrance = v;
                self.reprocess_image()
            }
            Message::SaturationChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.saturation = v;
                self.reprocess_image()
            }
            Message::HslHueChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.hsl_hue = v;
                self.reprocess_image()
            }
            Message::HslSaturationChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.hsl_saturation = v;
                self.reprocess_image()
            }
            Message::HslLightnessChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.hsl_lightness = v;
                self.reprocess_image()
            }
            Message::SplitShadowHueChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.split_shadow_hue = v;
                self.reprocess_image()
            }
            Message::SplitShadowSatChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.split_shadow_sat = v;
                self.reprocess_image()
            }
            Message::SplitHighlightHueChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.split_highlight_hue = v;
                self.reprocess_image()
            }
            Message::SplitHighlightSatChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.split_highlight_sat = v;
                self.reprocess_image()
            }
            Message::SplitBalanceChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.split_balance = v;
                self.reprocess_image()
            }
            Message::SharpenAmountChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.sharpen_amount = v;
                self.reprocess_image()
            }
            Message::SharpenRadiusChanged(v) => {
                self.snapshot_for_undo();
                self.edit_params.sharpen_radius = v;
                self.reprocess_image()
            }
            Message::AutoEnhance => self.handle_auto_enhance(),
            Message::AutoEnhanceComplete(params) => {
                self.snapshot_for_undo();
                self.edit_params = params;
                self.reprocess_image()
            }
            Message::ResetEdits => {
                self.snapshot_for_undo();
                self.edit_params = EditParams::default();
                self.reprocess_image()
            }
            Message::ResetControl(control) => self.reset_control(control),
            Message::ResetSection(section) => self.reset_section(section),
            Message::Undo => self.handle_undo(),
            Message::Redo => self.handle_redo(),
            Message::CopyEdits => {
                self.edit_clipboard = Some(self.edit_params.clone());
                self.status_message = "Copied edits".into();
                self.update_paste_menu_state();
                Task::none()
            }
            Message::PasteEdits => {
                if let Some(clipboard) = self.edit_clipboard.clone() {
                    self.snapshot_for_undo();
                    let crop = (
                        self.edit_params.crop_x,
                        self.edit_params.crop_y,
                        self.edit_params.crop_w,
                        self.edit_params.crop_h,
                    );
                    self.edit_params = clipboard;
                    self.edit_params.crop_x = crop.0;
                    self.edit_params.crop_y = crop.1;
                    self.edit_params.crop_w = crop.2;
                    self.edit_params.crop_h = crop.3;
                    self.status_message = "Pasted edits".into();
                    self.reprocess_image()
                } else {
                    Task::none()
                }
            }
            Message::ZoomAtPoint(factor, cx, cy, vw, vh) => {
                self.handle_zoom_at_point(factor, cx, cy, vw, vh);
                Task::none()
            }
            Message::PanDelta(dx, dy) => {
                self.zoom_state.pan.x += dx;
                self.zoom_state.pan.y += dy;
                Task::none()
            }
            Message::ResetZoom => {
                self.zoom_state = ZoomState::default();
                Task::none()
            }
            Message::ToggleBeforeAfter => {
                if self.original_display.is_some() {
                    self.showing_before = !self.showing_before;
                }
                Task::none()
            }
            Message::OriginalReady(handle) => {
                self.original_display = Some(handle);
                Task::none()
            }
            Message::NextPhoto => self.navigate_photo(1),
            Message::PrevPhoto => self.navigate_photo(-1),
            Message::NudgeExposure(delta) => {
                self.snapshot_for_undo();
                self.edit_params.exposure = (self.edit_params.exposure + delta).clamp(-5.0, 5.0);
                self.reprocess_image()
            }
            Message::RateAndAdvance(rating) => {
                let rate_task = self.handle_set_rating(rating);
                let advance_task = self.navigate_photo(1);
                Task::batch([rate_task, advance_task])
            }
            Message::DeletePhoto => self.handle_delete_photo(),
            Message::ToggleCropMode => self.handle_toggle_crop_mode(),
            Message::ExitCropMode => {
                if self.crop_mode {
                    self.handle_toggle_crop_mode()
                } else {
                    Task::none()
                }
            }
            Message::SetCropAspect(aspect) => {
                self.crop_aspect = aspect;
                if let Some(ratio) = aspect {
                    self.apply_aspect_ratio(ratio);
                }
                Task::none()
            }
            Message::UpdateCrop(x, y, w, h) => {
                self.edit_params.crop_x = x;
                self.edit_params.crop_y = y;
                self.edit_params.crop_w = w;
                self.edit_params.crop_h = h;
                Task::none()
            }
            Message::ResetCrop => {
                self.snapshot_for_undo();
                self.edit_params.crop_x = 0.0;
                self.edit_params.crop_y = 0.0;
                self.edit_params.crop_w = 1.0;
                self.edit_params.crop_h = 1.0;
                if self.crop_mode {
                    Task::none()
                } else {
                    self.reprocess_image()
                }
            }
            Message::SetDateFilter(filter) => {
                self.date_filter = filter;
                Task::none()
            }
            Message::SetRatingFilter(filter) => {
                self.rating_filter = filter;
                Task::none()
            }
            Message::SetSortOrder(order) => {
                self.sort_order = order;
                Task::none()
            }
            Message::ToggleDateExpansion(key) => {
                if !self.expanded_dates.remove(&key) {
                    self.expanded_dates.insert(key);
                }
                Task::none()
            }
            Message::TogglePanelSection(section) => {
                if !self.panel_sections.remove(&section) {
                    self.panel_sections.insert(section);
                }
                Task::none()
            }
            Message::Noop => Task::none(),
        }
    }

    fn handle_catalog_opened(&mut self, path: String) -> Task<Message> {
        match Catalog::open(&path) {
            Ok(catalog) => {
                info!(%path, "catalog opened");
                self.catalog = Some(catalog);
                self.catalog_path = Some(path);
                self.refresh_photos()
            }
            Err(err) => {
                error!(%err, "failed to open catalog");
                self.status_message = format!("Error opening catalog: {err}");
                Task::none()
            }
        }
    }

    fn handle_import(&self) -> Task<Message> {
        Task::perform(
            async {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Import photos")
                    .add_filter(
                        "Images",
                        &[
                            "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "rw2",
                            "orf", "pef", "dng", "3fr", "ari", "bay", "cap", "dcr", "erf", "fff",
                            "iiq", "k25", "kdc", "mef", "mos", "mrw", "raw", "rwl", "srw", "x3f",
                            "jpg", "jpeg", "png", "tiff", "tif",
                        ],
                    );
                let handles = dialog.pick_files().await.unwrap_or_default();
                handles.iter().map(|h| h.path().to_path_buf()).collect()
            },
            Message::ImportsSelected,
        )
    }

    fn handle_imports_selected(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        if paths.is_empty() {
            return Task::none();
        }

        self.is_importing = true;
        self.status_message = format!("Importing {} file(s)...", paths.len());
        let catalog_path = self.catalog_path.clone().unwrap_or_default();
        Task::perform(
            async move {
                let catalog = Catalog::open(&catalog_path).ok();
                if let Some(catalog) = catalog {
                    match crema_catalog::import::import_paths(&catalog, &paths) {
                        Ok(result) => (result.imported.len(), result.errors.len()),
                        Err(_) => (0, 1),
                    }
                } else {
                    (0, 1)
                }
            },
            |(imported, errors)| Message::ImportComplete(imported, errors),
        )
    }

    fn handle_import_complete(&mut self, imported: usize, errors: usize) -> Task<Message> {
        self.is_importing = false;
        self.status_message = format!("Imported {imported} photos ({errors} errors)");
        self.refresh_photos()
    }

    fn handle_photos_listed(&mut self, photos: Vec<Photo>) -> Task<Message> {
        self.photos = photos;
        self.status_message = format!("{} photos in catalog", self.photos.len());

        if self
            .selected_photo
            .is_some_and(|id| !self.photos.iter().any(|photo| photo.id == id))
        {
            self.selected_photo = None;
        }

        if self
            .loaded_photo
            .is_some_and(|id| !self.photos.iter().any(|photo| photo.id == id))
        {
            self.loaded_photo = None;
            self.current_image = None;
            self.preview_image = None;
            self.processed_image = None;
            self.histogram = None;
            self.current_exif.clear();
        }

        self.update_export_enabled();
        self.load_next_thumbnail_batch()
    }

    fn handle_thumbnail_ready(&mut self, id: PhotoId, bytes: Vec<u8>) -> Task<Message> {
        let handle = iced::widget::image::Handle::from_bytes(bytes);
        self.thumbnails.insert(id, handle);
        self.load_next_thumbnail_batch()
    }

    fn handle_select_photo(&mut self, id: PhotoId) -> Task<Message> {
        self.selected_photo = Some(id);
        self.update_export_enabled();

        if let Some(photo) = self.photos.iter().find(|photo| photo.id == id) {
            let name = std::path::Path::new(&photo.file_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            self.status_message = format!("Selected {name}. Open Develop to edit.");
        }

        Task::none()
    }

    fn open_photo(&mut self, id: PhotoId) -> Task<Message> {
        if self.loaded_photo == Some(id) && self.preview_image.is_some() {
            self.selected_photo = Some(id);
            self.workspace = Workspace::Develop;
            self.right_panel_open = true;
            self.update_export_enabled();
            return Task::none();
        }

        self.save_current_edits();
        self.clear_undo_history();
        self.zoom_state = ZoomState::default();
        self.original_display = None;
        self.showing_before = false;
        self.crop_mode = false;

        self.selected_photo = Some(id);
        self.workspace = Workspace::Develop;
        self.right_panel_open = true;
        self.current_image = None;
        self.preview_image = None;
        self.processed_image = None;
        self.histogram = None;
        self.current_exif.clear();
        self.loaded_photo = None;
        self.is_loading_photo = true;
        self.is_processing = false;

        if let Some(ref catalog) = self.catalog {
            if let Ok(Some(edit)) = catalog.get_edits(id) {
                self.edit_params = edit.to_edit_params();
            } else {
                self.edit_params = EditParams::default();
            }
        }

        self.update_export_enabled();

        let photo = self.photos.iter().find(|p| p.id == id).cloned();
        let Some(photo) = photo else {
            self.is_loading_photo = false;
            self.status_message = "Unable to locate that photo.".into();
            return Task::none();
        };

        let name = std::path::Path::new(&photo.file_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.status_message = format!("Loading {name}...");

        let path = photo.file_path.clone();
        Task::perform(
            async move {
                let t0 = std::time::Instant::now();
                let p = std::path::Path::new(&path);
                let buf = crema_core::raw::load_any(p).ok()?;
                let preview = buf.downsample(2048);
                let exif = crema_metadata::exif::ExifData::from_file(p)
                    .ok()
                    .map(|e| e.summary_lines())
                    .unwrap_or_default();
                info!(
                    elapsed_ms = t0.elapsed().as_millis(),
                    w = buf.width,
                    h = buf.height,
                    "image loaded"
                );
                Some((id, Arc::new(buf), Arc::new(preview), exif))
            },
            move |result| match result {
                Some((id, buf, preview, exif)) => Message::ImageLoaded(id, buf, preview, exif),
                None => Message::ImageLoadFailed(id),
            },
        )
    }

    fn handle_set_workspace(&mut self, workspace: Workspace) -> Task<Message> {
        if workspace == Workspace::Library && self.workspace == Workspace::Develop {
            self.save_current_edits();
        }

        self.workspace = workspace;
        self.update_export_enabled();

        if workspace == Workspace::Develop {
            self.right_panel_open = true;
            if let Some(id) = self.selected_photo {
                if self.loaded_photo != Some(id) || self.preview_image.is_none() {
                    return self.open_photo(id);
                }
            } else {
                self.status_message = "Select a photo in Library to open Develop.".into();
            }
        }

        Task::none()
    }

    fn handle_image_loaded(
        &mut self,
        id: PhotoId,
        buf: Arc<ImageBuf>,
        preview: Arc<ImageBuf>,
        exif: Vec<(String, String)>,
    ) -> Task<Message> {
        if self.selected_photo != Some(id) {
            return Task::none();
        }

        self.loaded_photo = Some(id);
        self.current_image = Some(buf);
        self.preview_image = Some(preview.clone());
        self.current_exif = exif;
        self.is_loading_photo = false;
        self.original_display = None;
        self.showing_before = false;
        self.status_message = format!("Rendering {}...", self.current_photo_label());
        self.update_export_enabled();

        let original_task = Task::perform(
            async move {
                let rgba = preview.to_rgba_u8_srgb();
                iced::widget::image::Handle::from_rgba(preview.width, preview.height, rgba)
            },
            Message::OriginalReady,
        );

        Task::batch([self.reprocess_image(), original_task])
    }

    fn handle_image_processed(
        &mut self,
        generation: u64,
        handle: iced::widget::image::Handle,
        hist: Box<HistogramData>,
    ) -> Task<Message> {
        if generation != self.processing_generation {
            return Task::none();
        }

        self.processed_image = Some(handle);
        self.histogram = Some(hist);
        self.is_processing = false;
        if let Some(ref preview) = self.preview_image {
            self.preview_dimensions = (preview.width, preview.height);
        }
        self.status_message = format!("Ready to edit {}", self.current_photo_label());
        self.save_current_edits();
        Task::none()
    }

    fn handle_image_load_failed(&mut self, id: PhotoId) -> Task<Message> {
        if self.selected_photo == Some(id) {
            self.is_loading_photo = false;
            self.is_processing = false;
            self.status_message = "Unable to load that photo.".into();
        }
        Task::none()
    }

    fn handle_export(&self) -> Task<Message> {
        let default_name = self.default_export_filename();
        Task::perform(
            async move {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Export photo")
                    .set_file_name(&default_name)
                    .add_filter("JPEG", &["jpg", "jpeg"])
                    .add_filter("PNG", &["png"])
                    .add_filter("TIFF", &["tiff", "tif"]);
                dialog.save_file().await.map(|h| h.path().to_path_buf())
            },
            |result| match result {
                Some(path) => Message::ExportPathSelected(path),
                None => Message::Noop,
            },
        )
    }

    fn handle_export_path_selected(&mut self, path: PathBuf) -> Task<Message> {
        let Some(ref full_res) = self.current_image else {
            return Task::none();
        };

        self.is_exporting = true;
        self.status_message = format!("Exporting {}...", self.current_photo_label());
        let buf = ImageBuf::clone(full_res);
        let params = self.edit_params.clone();
        Task::perform(
            async move { export_image(buf, &params, &path) },
            Message::ExportComplete,
        )
    }

    fn handle_export_complete(&mut self, msg: String) -> Task<Message> {
        self.is_exporting = false;
        self.status_message = msg;
        Task::none()
    }

    fn handle_auto_enhance(&self) -> Task<Message> {
        let Some(ref preview) = self.preview_image else {
            return Task::none();
        };
        let buf = preview.clone();
        Task::perform(
            async move { crema_core::pipeline::auto_enhance::auto_enhance(&buf) },
            Message::AutoEnhanceComplete,
        )
    }

    pub fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::batch([
            crate::menu::subscription(),
            iced::keyboard::listen().map(|event| match event {
                iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    handle_key_press(key, modifiers).unwrap_or(Message::Noop)
                }
                _ => Message::Noop,
            }),
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        views::unified::view(self)
    }

    fn refresh_photos(&self) -> Task<Message> {
        let catalog_path = self.catalog_path.clone().unwrap_or_default();
        Task::perform(
            async move {
                let catalog = Catalog::open(&catalog_path).ok();
                catalog
                    .map(|c| c.list_photos().unwrap_or_default())
                    .unwrap_or_default()
            },
            Message::PhotosListed,
        )
    }

    fn reprocess_image(&mut self) -> Task<Message> {
        let Some(ref preview) = self.preview_image else {
            self.is_processing = false;
            return Task::none();
        };

        self.processing_generation += 1;
        self.is_processing = true;
        let generation = self.processing_generation;
        let buf = preview.clone();
        let mut params = self.edit_params.clone();
        if self.crop_mode {
            params.crop_x = 0.0;
            params.crop_y = 0.0;
            params.crop_w = 1.0;
            params.crop_h = 1.0;
        }

        let gpu = self.gpu.clone();

        Task::perform(
            async move {
                let gpu_result = gpu.and_then(|g| process_gpu(&g, &buf, &params));

                let processed = gpu_result.or_else(|| {
                    let pipeline = crema_core::pipeline::Pipeline::new();
                    let owned = ImageBuf::clone(&buf);
                    pipeline.process_cpu(owned, &params).ok()
                });

                let (w, h, rgba) = match processed {
                    Some(img) => {
                        let rgba = img.to_rgba_u8_srgb();
                        (img.width, img.height, rgba)
                    }
                    None => {
                        let rgba = buf.to_rgba_u8_srgb();
                        (buf.width, buf.height, rgba)
                    }
                };
                let histogram = crate::widgets::histogram::compute_histogram(&rgba);
                let handle = iced::widget::image::Handle::from_rgba(w, h, rgba);
                (generation, handle, histogram)
            },
            |(generation, handle, histogram)| {
                Message::ImageProcessed(generation, handle, Box::new(histogram))
            },
        )
    }

    fn load_next_thumbnail_batch(&self) -> Task<Message> {
        const THUMBNAIL_BATCH_SIZE: usize = 16;
        let cache_dir = self.thumbnail_cache_dir.clone();
        let tasks: Vec<_> = self
            .photos
            .iter()
            .filter(|p| !self.thumbnails.contains_key(&p.id))
            .take(THUMBNAIL_BATCH_SIZE)
            .map(|p| {
                let id = p.id;
                let path = p.file_path.clone();
                let cache_dir = cache_dir.clone();
                Task::perform(
                    async move {
                        match load_thumbnail_bytes(&path, cache_dir.as_deref()) {
                            Ok(bytes) => Some((id, bytes)),
                            Err(_) => None,
                        }
                    },
                    |result| match result {
                        Some((id, bytes)) => Message::ThumbnailReady(id, bytes),
                        None => Message::Noop,
                    },
                )
            })
            .collect();
        Task::batch(tasks)
    }

    fn save_current_edits(&self) {
        if let (Some(id), Some(catalog)) = (self.loaded_photo, &self.catalog)
            && let Err(err) = catalog.save_edits(id, &self.edit_params)
        {
            error!(%err, "failed to save edits");
        }
    }

    fn update_export_enabled(&self) {
        if let Some(menu) = &self.menu {
            let enabled = self.selected_photo.is_some()
                && self.current_image.is_some()
                && self.loaded_photo == self.selected_photo;
            menu.export_item.set_enabled(enabled);
        }
    }

    fn reset_control(&mut self, control: EditControl) -> Task<Message> {
        self.snapshot_for_undo();
        let defaults = EditParams::default();

        match control {
            EditControl::Exposure => self.edit_params.exposure = defaults.exposure,
            EditControl::Contrast => self.edit_params.contrast = defaults.contrast,
            EditControl::Highlights => self.edit_params.highlights = defaults.highlights,
            EditControl::Shadows => self.edit_params.shadows = defaults.shadows,
            EditControl::Blacks => self.edit_params.blacks = defaults.blacks,
            EditControl::WbTemp => self.edit_params.wb_temp = defaults.wb_temp,
            EditControl::WbTint => self.edit_params.wb_tint = defaults.wb_tint,
            EditControl::Vibrance => self.edit_params.vibrance = defaults.vibrance,
            EditControl::Saturation => self.edit_params.saturation = defaults.saturation,
            EditControl::HslHue => self.edit_params.hsl_hue = defaults.hsl_hue,
            EditControl::HslSaturation => self.edit_params.hsl_saturation = defaults.hsl_saturation,
            EditControl::HslLightness => self.edit_params.hsl_lightness = defaults.hsl_lightness,
            EditControl::SplitShadowHue => {
                self.edit_params.split_shadow_hue = defaults.split_shadow_hue
            }
            EditControl::SplitShadowSat => {
                self.edit_params.split_shadow_sat = defaults.split_shadow_sat
            }
            EditControl::SplitHighlightHue => {
                self.edit_params.split_highlight_hue = defaults.split_highlight_hue
            }
            EditControl::SplitHighlightSat => {
                self.edit_params.split_highlight_sat = defaults.split_highlight_sat
            }
            EditControl::SplitBalance => self.edit_params.split_balance = defaults.split_balance,
            EditControl::SharpenAmount => self.edit_params.sharpen_amount = defaults.sharpen_amount,
            EditControl::SharpenRadius => self.edit_params.sharpen_radius = defaults.sharpen_radius,
        }

        self.reprocess_image()
    }

    fn reset_section(&mut self, section: EditSection) -> Task<Message> {
        self.snapshot_for_undo();
        let defaults = EditParams::default();

        match section {
            EditSection::Light => {
                self.edit_params.exposure = defaults.exposure;
                self.edit_params.contrast = defaults.contrast;
                self.edit_params.highlights = defaults.highlights;
                self.edit_params.shadows = defaults.shadows;
                self.edit_params.blacks = defaults.blacks;
            }
            EditSection::Color => {
                self.edit_params.wb_temp = defaults.wb_temp;
                self.edit_params.wb_tint = defaults.wb_tint;
                self.edit_params.vibrance = defaults.vibrance;
                self.edit_params.saturation = defaults.saturation;
            }
            EditSection::Hsl => {
                self.edit_params.hsl_hue = defaults.hsl_hue;
                self.edit_params.hsl_saturation = defaults.hsl_saturation;
                self.edit_params.hsl_lightness = defaults.hsl_lightness;
            }
            EditSection::SplitTone => {
                self.edit_params.split_shadow_hue = defaults.split_shadow_hue;
                self.edit_params.split_shadow_sat = defaults.split_shadow_sat;
                self.edit_params.split_highlight_hue = defaults.split_highlight_hue;
                self.edit_params.split_highlight_sat = defaults.split_highlight_sat;
                self.edit_params.split_balance = defaults.split_balance;
            }
            EditSection::Detail => {
                self.edit_params.sharpen_amount = defaults.sharpen_amount;
                self.edit_params.sharpen_radius = defaults.sharpen_radius;
            }
        }

        self.reprocess_image()
    }

    fn snapshot_for_undo(&mut self) {
        if self.undo_stack.last() == Some(&self.edit_params) {
            return;
        }
        self.undo_stack.push(self.edit_params.clone());
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.update_undo_menu_state();
    }

    fn handle_undo(&mut self) -> Task<Message> {
        let Some(prev) = self.undo_stack.pop() else {
            return Task::none();
        };
        self.redo_stack.push(self.edit_params.clone());
        self.edit_params = prev;
        self.update_undo_menu_state();
        self.reprocess_image()
    }

    fn handle_redo(&mut self) -> Task<Message> {
        let Some(next) = self.redo_stack.pop() else {
            return Task::none();
        };
        self.undo_stack.push(self.edit_params.clone());
        self.edit_params = next;
        self.update_undo_menu_state();
        self.reprocess_image()
    }

    fn clear_undo_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.update_undo_menu_state();
    }

    fn update_undo_menu_state(&self) {
        if let Some(menu) = &self.menu {
            menu.undo_item.set_enabled(!self.undo_stack.is_empty());
            menu.redo_item.set_enabled(!self.redo_stack.is_empty());
        }
    }

    fn update_paste_menu_state(&self) {
        if let Some(menu) = &self.menu {
            menu.paste_edits_item
                .set_enabled(self.edit_clipboard.is_some());
        }
    }

    fn handle_zoom_at_point(&mut self, factor: f32, cx: f32, cy: f32, vw: f32, vh: f32) {
        let old_zoom = self.zoom_state.zoom;
        let new_zoom = (old_zoom * factor).clamp(1.0, 8.0);
        if (new_zoom - old_zoom).abs() < 0.001 {
            return;
        }

        // Zoom centered on cursor: adjust pan so the point under the cursor stays fixed.
        // The cursor is at (cx, cy) relative to the viewport.
        // The image center (without pan) is at (vw/2, vh/2).
        let cursor_from_center_x = cx - vw / 2.0;
        let cursor_from_center_y = cy - vh / 2.0;

        let ratio = 1.0 - new_zoom / old_zoom;
        self.zoom_state.pan.x += (cursor_from_center_x - self.zoom_state.pan.x) * ratio;
        self.zoom_state.pan.y += (cursor_from_center_y - self.zoom_state.pan.y) * ratio;
        self.zoom_state.zoom = new_zoom;

        // Reset pan when returning to fit
        if new_zoom <= 1.0 {
            self.zoom_state.pan = iced::Vector::ZERO;
        }
    }

    fn navigate_photo(&mut self, delta: i32) -> Task<Message> {
        let filtered = self.filtered_photos();
        if filtered.is_empty() {
            return Task::none();
        }

        let current_idx = self
            .selected_photo
            .and_then(|id| filtered.iter().position(|p| p.id == id));

        let new_idx = match current_idx {
            Some(idx) => {
                let len = filtered.len() as i32;
                ((idx as i32 + delta).rem_euclid(len)) as usize
            }
            None => 0,
        };

        let new_id = filtered[new_idx].id;
        drop(filtered);

        if self.workspace == Workspace::Develop {
            self.open_photo(new_id)
        } else {
            self.selected_photo = Some(new_id);
            self.update_export_enabled();
            Task::none()
        }
    }

    fn handle_toggle_crop_mode(&mut self) -> Task<Message> {
        if self.preview_image.is_none() {
            return Task::none();
        }
        self.crop_mode = !self.crop_mode;
        if self.crop_mode {
            self.snapshot_for_undo();
        }
        self.reprocess_image()
    }

    fn apply_aspect_ratio(&mut self, ratio: f32) {
        let iw = self.preview_dimensions.0 as f32;
        let ih = self.preview_dimensions.1 as f32;
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }

        let (cx, cy, cw, ch) = (
            self.edit_params.crop_x,
            self.edit_params.crop_y,
            self.edit_params.crop_w,
            self.edit_params.crop_h,
        );
        let center_x = cx + cw / 2.0;
        let center_y = cy + ch / 2.0;

        let pw = cw * iw;
        let ph = ch * ih;
        let current = pw / ph;

        let (new_w, new_h) = if current > ratio {
            (ph * ratio / iw, ch)
        } else {
            (cw, pw / ratio / ih)
        };

        let new_x = (center_x - new_w / 2.0).clamp(0.0, 1.0 - new_w);
        let new_y = (center_y - new_h / 2.0).clamp(0.0, 1.0 - new_h);

        self.edit_params.crop_x = new_x;
        self.edit_params.crop_y = new_y;
        self.edit_params.crop_w = new_w;
        self.edit_params.crop_h = new_h;
    }

    fn handle_set_rating(&mut self, rating: i32) -> Task<Message> {
        let Some(id) = self.selected_photo else {
            return Task::none();
        };
        let rating = rating.clamp(0, 5);
        if let Some(catalog) = &self.catalog
            && let Err(err) = catalog.set_rating(id, rating)
        {
            error!(%err, "failed to set rating");
            return Task::none();
        }
        if let Some(photo) = self.photos.iter_mut().find(|p| p.id == id) {
            photo.rating = rating;
        }
        Task::none()
    }

    fn handle_delete_photo(&mut self) -> Task<Message> {
        let Some(id) = self.selected_photo else {
            return Task::none();
        };
        if let Some(catalog) = &self.catalog
            && let Err(err) = catalog.delete_photo(id)
        {
            error!(%err, "failed to delete photo");
            return Task::none();
        }
        self.photos.retain(|p| p.id != id);
        self.thumbnails.remove(&id);
        if self.loaded_photo == Some(id) {
            self.loaded_photo = None;
            self.current_image = None;
            self.preview_image = None;
            self.processed_image = None;
            self.histogram = None;
            self.current_exif.clear();
        }
        self.selected_photo = None;
        self.update_export_enabled();
        self.status_message = "Photo removed from catalog.".into();
        Task::none()
    }

    pub fn crop_mode(&self) -> bool {
        self.crop_mode
    }

    pub fn crop_aspect(&self) -> Option<f32> {
        self.crop_aspect
    }

    pub fn zoom_state(&self) -> &ZoomState {
        &self.zoom_state
    }

    pub fn preview_dimensions(&self) -> (u32, u32) {
        self.preview_dimensions
    }

    pub fn showing_before(&self) -> bool {
        self.showing_before
    }

    pub fn display_image(&self) -> Option<&iced::widget::image::Handle> {
        if self.showing_before {
            self.original_display.as_ref()
        } else {
            self.processed_image.as_ref()
        }
    }

    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn current_photo(&self) -> Option<&Photo> {
        self.selected_photo
            .and_then(|id| self.photos.iter().find(|photo| photo.id == id))
    }

    pub fn thumbnails(&self) -> &std::collections::HashMap<PhotoId, iced::widget::image::Handle> {
        &self.thumbnails
    }

    pub fn edit_params(&self) -> &EditParams {
        &self.edit_params
    }

    pub fn preview_image(&self) -> Option<&Arc<ImageBuf>> {
        self.preview_image.as_ref()
    }

    pub fn histogram(&self) -> Option<&HistogramData> {
        self.histogram.as_deref()
    }

    pub fn current_exif(&self) -> &[(String, String)] {
        &self.current_exif
    }

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn footer_status(&self) -> String {
        let mut states = Vec::new();

        if self.is_importing {
            states.push("Importing");
        }
        if self.is_loading_photo {
            states.push("Loading photo");
        }
        if self.is_processing {
            states.push("Rendering preview");
        }
        if self.is_exporting {
            states.push("Exporting");
        }

        if states.is_empty() {
            self.status_message.clone()
        } else {
            format!("{}  |  {}", states.join(" · "), self.status_message)
        }
    }

    pub fn date_filter(&self) -> &DateFilter {
        &self.date_filter
    }

    pub fn expanded_dates(&self) -> &HashSet<DateExpansionKey> {
        &self.expanded_dates
    }

    pub fn filtered_photos(&self) -> Vec<&Photo> {
        let mut photos: Vec<&Photo> = self
            .photos
            .iter()
            .filter(|photo| self.date_filter.matches(photo) && self.rating_filter.matches(photo))
            .collect();
        self.sort_order.sort(&mut photos);
        photos
    }

    pub fn rating_filter(&self) -> RatingFilter {
        self.rating_filter
    }

    pub fn sort_order(&self) -> SortOrder {
        self.sort_order
    }

    pub fn workspace(&self) -> Workspace {
        self.workspace
    }

    pub fn selected_photo(&self) -> Option<PhotoId> {
        self.selected_photo
    }

    pub fn right_panel_open(&self) -> bool {
        self.right_panel_open
    }

    pub fn is_panel_open(&self, section: PanelSection) -> bool {
        self.panel_sections.contains(&section)
    }

    pub fn is_loading_photo(&self) -> bool {
        self.is_loading_photo
    }

    pub fn is_processing(&self) -> bool {
        self.is_processing
    }

    pub fn has_selection(&self) -> bool {
        self.selected_photo.is_some()
    }

    pub fn can_export(&self) -> bool {
        self.selected_photo.is_some()
            && self.current_image.is_some()
            && self.loaded_photo == self.selected_photo
    }

    pub fn current_photo_label(&self) -> String {
        let Some(photo) = self.current_photo() else {
            return "No photo selected".into();
        };

        std::path::Path::new(&photo.file_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }

    pub fn current_photo_summary(&self) -> String {
        let Some(photo) = self.current_photo() else {
            return "Select a photo in Library to edit it in Develop.".into();
        };

        let mut parts = Vec::new();

        if let Some(model) = &photo.camera_model {
            parts.push(model.clone());
        }
        if let Some(lens) = &photo.lens {
            parts.push(lens.clone());
        }
        if let Some(focal_length) = photo.focal_length {
            parts.push(format!("{focal_length:.0}mm"));
        }
        if let Some(aperture) = photo.aperture {
            parts.push(format!("f/{aperture:.1}"));
        }
        if let Some(shutter) = &photo.shutter_speed {
            parts.push(shutter.clone());
        }
        if let Some(iso) = photo.iso {
            parts.push(format!("ISO {iso}"));
        }

        if parts.is_empty() {
            "Preview fit".into()
        } else {
            parts.join(" · ")
        }
    }

    pub fn is_control_adjusted(&self, control: EditControl) -> bool {
        let defaults = EditParams::default();

        match control {
            EditControl::Exposure => self.edit_params.exposure != defaults.exposure,
            EditControl::Contrast => self.edit_params.contrast != defaults.contrast,
            EditControl::Highlights => self.edit_params.highlights != defaults.highlights,
            EditControl::Shadows => self.edit_params.shadows != defaults.shadows,
            EditControl::Blacks => self.edit_params.blacks != defaults.blacks,
            EditControl::WbTemp => self.edit_params.wb_temp != defaults.wb_temp,
            EditControl::WbTint => self.edit_params.wb_tint != defaults.wb_tint,
            EditControl::Vibrance => self.edit_params.vibrance != defaults.vibrance,
            EditControl::Saturation => self.edit_params.saturation != defaults.saturation,
            EditControl::HslHue => self.edit_params.hsl_hue != defaults.hsl_hue,
            EditControl::HslSaturation => {
                self.edit_params.hsl_saturation != defaults.hsl_saturation
            }
            EditControl::HslLightness => self.edit_params.hsl_lightness != defaults.hsl_lightness,
            EditControl::SplitShadowHue => {
                self.edit_params.split_shadow_hue != defaults.split_shadow_hue
            }
            EditControl::SplitShadowSat => {
                self.edit_params.split_shadow_sat != defaults.split_shadow_sat
            }
            EditControl::SplitHighlightHue => {
                self.edit_params.split_highlight_hue != defaults.split_highlight_hue
            }
            EditControl::SplitHighlightSat => {
                self.edit_params.split_highlight_sat != defaults.split_highlight_sat
            }
            EditControl::SplitBalance => self.edit_params.split_balance != defaults.split_balance,
            EditControl::SharpenAmount => {
                self.edit_params.sharpen_amount != defaults.sharpen_amount
            }
            EditControl::SharpenRadius => {
                self.edit_params.sharpen_radius != defaults.sharpen_radius
            }
        }
    }

    fn default_export_filename(&self) -> String {
        if let Some(id) = self.loaded_photo
            && let Some(photo) = self.photos.iter().find(|p| p.id == id)
            && let Some(stem) = std::path::Path::new(&photo.file_path)
                .file_stem()
                .and_then(|s| s.to_str())
        {
            return format!("{stem}.jpg");
        }

        "export.jpg".into()
    }
}

fn handle_key_press(
    key: iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Message> {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;

    // Avoid conflicts with muda menu accelerators (Cmd+Z, Cmd+Shift+Z handled there)
    if modifiers.command() {
        return None;
    }

    match key {
        Key::Named(Named::Escape) => Some(Message::ExitCropMode),
        Key::Named(Named::ArrowRight) => Some(Message::NextPhoto),
        Key::Named(Named::ArrowLeft) => Some(Message::PrevPhoto),
        Key::Named(Named::Delete | Named::Backspace) => Some(Message::DeletePhoto),
        Key::Character(c) if c.as_str() == "\\" => Some(Message::ToggleBeforeAfter),
        Key::Character(c) if c.as_str() == "[" => Some(Message::NudgeExposure(-0.5)),
        Key::Character(c) if c.as_str() == "]" => Some(Message::NudgeExposure(0.5)),
        Key::Character(c) if c.as_str() == "c" && !modifiers.shift() => {
            Some(Message::ToggleCropMode)
        }
        Key::Character(c) if c.as_str() == "r" && !modifiers.shift() => Some(Message::ResetEdits),
        Key::Character(c) if c.as_str() == "f" && !modifiers.shift() => Some(Message::ResetZoom),
        Key::Character(c) if matches!(c.as_str(), "0" | "1" | "2" | "3" | "4" | "5") => {
            let rating = c.as_str().parse::<i32>().unwrap_or(0);
            Some(Message::RateAndAdvance(rating))
        }
        Key::Character(c) if c.as_str() == "p" => Some(Message::RateAndAdvance(1)),
        Key::Character(c) if c.as_str() == "x" => Some(Message::RateAndAdvance(-1)),
        Key::Character(c) if c.as_str() == "u" => Some(Message::RateAndAdvance(0)),
        _ => None,
    }
}

fn process_gpu(
    gpu: &Arc<std::sync::Mutex<(GpuContext, GpuPipeline)>>,
    buf: &Arc<ImageBuf>,
    params: &EditParams,
) -> Option<ImageBuf> {
    let mut lock = gpu.lock().ok()?;
    let (ctx, pipeline) = &mut *lock;

    let input =
        crema_gpu::texture::GpuTexture::from_image_buf(&ctx.device, &ctx.queue, buf, "input");
    let output = pipeline.process(ctx, &input, params).ok()?;
    let result = output.download(&ctx.device, &ctx.queue).ok()?;
    Some(result)
}

fn dirs_catalog_path() -> String {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crema");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("catalog.db").to_string_lossy().to_string()
}

fn thumbnail_cache_key(path: &std::path::Path) -> String {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.to_string_lossy().as_bytes());
    if let Some(t) = mtime {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        hasher.update(&dur.as_nanos().to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn export_image(buf: ImageBuf, params: &EditParams, path: &std::path::Path) -> String {
    let pipeline = crema_core::pipeline::Pipeline::new();
    let processed = match pipeline.process_cpu(buf, params) {
        Ok(p) => p,
        Err(e) => return format!("Export failed: {e}"),
    };

    let w = processed.width;
    let h = processed.height;
    let rgba = processed.to_rgba_u8_srgb();

    let img = match image::RgbaImage::from_raw(w, h, rgba) {
        Some(img) => image::DynamicImage::ImageRgba8(img),
        None => return "Export failed: could not construct image buffer".into(),
    };

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let result = if ext == "jpg" || ext == "jpeg" {
        let file = match std::fs::File::create(path) {
            Ok(f) => f,
            Err(e) => return format!("Export failed: {e}"),
        };
        let writer = std::io::BufWriter::new(file);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 92);
        img.to_rgb8().write_with_encoder(encoder)
    } else {
        img.save(path)
    };

    match result {
        Ok(()) => format!("Exported to {}", path.display()),
        Err(e) => format!("Export failed: {e}"),
    }
}

fn load_thumbnail_bytes(
    path: &str,
    cache_dir: Option<&std::path::Path>,
) -> anyhow::Result<Vec<u8>> {
    let p = std::path::Path::new(path);

    if let Some(dir) = cache_dir
        && let Ok(cache) = ThumbnailCache::new(dir.to_path_buf())
    {
        let key = thumbnail_cache_key(p);
        if let Some(bytes) = cache.load(&key) {
            return Ok(bytes);
        }
        let bytes = crema_thumbnails::generator::fast_thumbnail(p)?;
        cache.store(&key, &bytes).ok();
        return Ok(bytes);
    }

    crema_thumbnails::generator::fast_thumbnail(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_image() -> ImageBuf {
        let mut data = vec![0.0f32; 4 * 2 * 3];
        data[0] = 0.8;
        data[1] = 0.1;
        data[2] = 0.1;
        let last = data.len() - 3;
        data[last] = 0.1;
        data[last + 1] = 0.1;
        data[last + 2] = 0.8;
        ImageBuf::from_data(4, 2, data).unwrap()
    }

    fn test_params() -> EditParams {
        EditParams {
            exposure: 0.5,
            wb_temp: 6000.0,
            wb_tint: 5.0,
            ..EditParams::default()
        }
    }

    #[test]
    fn export_jpeg_writes_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jpg");

        let msg = export_image(test_image(), &test_params(), &path);

        assert!(msg.starts_with("Exported to"), "unexpected: {msg}");
        assert!(path.exists());

        let img = image::open(&path).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
    }

    #[test]
    fn export_jpeg_uppercase_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.JPG");

        let msg = export_image(test_image(), &EditParams::default(), &path);
        assert!(msg.starts_with("Exported to"), "unexpected: {msg}");

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "not a JPEG file");
    }

    #[test]
    fn export_png_writes_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");

        let msg = export_image(test_image(), &test_params(), &path);

        assert!(msg.starts_with("Exported to"), "unexpected: {msg}");
        assert!(path.exists());

        let img = image::open(&path).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn export_tiff_writes_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.tiff");

        let msg = export_image(test_image(), &test_params(), &path);

        assert!(msg.starts_with("Exported to"), "unexpected: {msg}");
        assert!(path.exists());

        let img = image::open(&path).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
    }

    #[test]
    fn export_applies_edits() {
        let dir = tempfile::tempdir().unwrap();

        let path_default = dir.path().join("default.png");
        export_image(test_image(), &EditParams::default(), &path_default);

        let bright_params = EditParams {
            exposure: 2.0,
            ..EditParams::default()
        };
        let path_bright = dir.path().join("bright.png");
        export_image(test_image(), &bright_params, &path_bright);

        let img_default = image::open(&path_default).unwrap().into_rgba8();
        let img_bright = image::open(&path_bright).unwrap().into_rgba8();

        let avg_default: u32 = img_default.pixels().map(|p| p.0[0] as u32).sum();
        let avg_bright: u32 = img_bright.pixels().map(|p| p.0[0] as u32).sum();
        assert!(
            avg_bright > avg_default,
            "exposure boost should brighten: default={avg_default} bright={avg_bright}"
        );
    }

    #[test]
    fn export_default_params_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.png");

        let buf = ImageBuf::from_data(2, 2, vec![0.5; 2 * 2 * 3]).unwrap();
        export_image(buf, &EditParams::default(), &path);

        let img = image::open(&path).unwrap().into_rgba8();
        let first = img.pixels().next().unwrap().0;
        for px in img.pixels() {
            assert_eq!(px.0, first, "all pixels should be identical");
        }
    }

    #[test]
    fn export_to_nonexistent_dir_fails_gracefully() {
        let path = Path::new("/nonexistent/dir/photo.jpg");
        let msg = export_image(test_image(), &EditParams::default(), path);
        assert!(msg.starts_with("Export failed:"), "unexpected: {msg}");
    }

    #[test]
    fn export_message_contains_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.jpg");

        let msg = export_image(test_image(), &EditParams::default(), &path);
        assert!(
            msg.contains("result.jpg"),
            "success message should contain filename: {msg}"
        );
    }
}
