use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use iced::{Element, Task, Theme};
use tracing::{error, info};

use crema_catalog::db::Catalog;
use crema_catalog::models::{Photo, PhotoId};
use crema_core::image_buf::{EditParams, ImageBuf};
use crema_thumbnails::cache::ThumbnailCache;

use crate::views;
use crate::widgets::date_sidebar::{DateExpansionKey, DateFilter};
use crate::widgets::histogram::HistogramData;

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
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSection {
    Light,
    Color,
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
}

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

    status_message: String,

    processing_generation: u64,
    thumbnail_cache_dir: Option<PathBuf>,
    is_importing: bool,
    is_exporting: bool,
    is_loading_photo: bool,
    is_processing: bool,

    date_filter: DateFilter,
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
    AutoEnhance,
    AutoEnhanceComplete(EditParams),
    ResetEdits,
    ResetControl(EditControl),
    ResetSection(EditSection),

    ImageLoaded(PhotoId, Arc<ImageBuf>, Arc<ImageBuf>, Vec<(String, String)>),
    ImageProcessed(u64, iced::widget::image::Handle, Box<HistogramData>),
    ImageLoadFailed(PhotoId),

    CatalogOpened(String),
    PhotosListed(Vec<Photo>),

    Export,
    ExportPathSelected(PathBuf),
    ExportComplete(String),

    SetDateFilter(DateFilter),
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
            status_message: "Welcome to Crema. Import photos to get started.".into(),
            processing_generation: 0,
            thumbnail_cache_dir: dirs::cache_dir().map(|d| d.join("crema").join("thumbnails")),
            is_importing: false,
            is_exporting: false,
            is_loading_photo: false,
            is_processing: false,
            date_filter: DateFilter::All,
            expanded_dates: HashSet::new(),
            panel_sections: HashSet::from([
                PanelSection::Histogram,
                PanelSection::Light,
                PanelSection::Color,
            ]),
        };

        let default_catalog = dirs_catalog_path();
        let task = Task::perform(async move { default_catalog }, Message::CatalogOpened);

        (app, task)
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
                self.edit_params.exposure = v;
                self.reprocess_image()
            }
            Message::ContrastChanged(v) => {
                self.edit_params.contrast = v;
                self.reprocess_image()
            }
            Message::HighlightsChanged(v) => {
                self.edit_params.highlights = v;
                self.reprocess_image()
            }
            Message::ShadowsChanged(v) => {
                self.edit_params.shadows = v;
                self.reprocess_image()
            }
            Message::BlacksChanged(v) => {
                self.edit_params.blacks = v;
                self.reprocess_image()
            }
            Message::WbTempChanged(v) => {
                self.edit_params.wb_temp = v;
                self.reprocess_image()
            }
            Message::WbTintChanged(v) => {
                self.edit_params.wb_tint = v;
                self.reprocess_image()
            }
            Message::VibranceChanged(v) => {
                self.edit_params.vibrance = v;
                self.reprocess_image()
            }
            Message::SaturationChanged(v) => {
                self.edit_params.saturation = v;
                self.reprocess_image()
            }
            Message::AutoEnhance => self.handle_auto_enhance(),
            Message::AutoEnhanceComplete(params) => {
                self.edit_params = params;
                self.reprocess_image()
            }
            Message::ResetEdits => {
                self.edit_params = EditParams::default();
                self.reprocess_image()
            }
            Message::ResetControl(control) => self.reset_control(control),
            Message::ResetSection(section) => self.reset_section(section),
            Message::SetDateFilter(filter) => {
                self.date_filter = filter;
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

        let cache_dir = self.thumbnail_cache_dir.clone();
        let tasks: Vec<_> = self
            .photos
            .iter()
            .filter(|p| !self.thumbnails.contains_key(&p.id))
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

    fn handle_thumbnail_ready(&mut self, id: PhotoId, bytes: Vec<u8>) -> Task<Message> {
        let handle = iced::widget::image::Handle::from_bytes(bytes);
        self.thumbnails.insert(id, handle);
        Task::none()
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
        self.preview_image = Some(preview);
        self.current_exif = exif;
        self.is_loading_photo = false;
        self.status_message = format!("Rendering {}...", self.current_photo_label());
        self.update_export_enabled();
        self.reprocess_image()
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
        crate::menu::subscription()
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
        let params = self.edit_params.clone();

        Task::perform(
            async move {
                let pipeline = crema_core::pipeline::Pipeline::new();
                let owned = ImageBuf::clone(&buf);
                let result = pipeline.process_cpu(owned, &params);
                let (w, h, rgba) = match result {
                    Ok(processed) => {
                        let rgba = processed.to_rgba_u8_srgb();
                        (processed.width, processed.height, rgba)
                    }
                    Err(_) => {
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

    fn save_current_edits(&self) {
        if let (Some(id), Some(catalog)) = (self.loaded_photo, &self.catalog)
            && let Err(err) = catalog.save_edits(id, &self.edit_params)
        {
            error!(%err, "failed to save edits");
        }
    }

    fn update_export_enabled(&self) {
        if let Some(menu) = &self.menu {
            let enabled =
                self.selected_photo.is_some() && self.current_image.is_some() && self.loaded_photo == self.selected_photo;
            menu.export_item.set_enabled(enabled);
        }
    }

    fn reset_control(&mut self, control: EditControl) -> Task<Message> {
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
        }

        self.reprocess_image()
    }

    fn reset_section(&mut self, section: EditSection) -> Task<Message> {
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
        }

        self.reprocess_image()
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

    pub fn processed_image(&self) -> Option<&iced::widget::image::Handle> {
        self.processed_image.as_ref()
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
        self.photos
            .iter()
            .filter(|photo| self.date_filter.matches(photo))
            .collect()
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
