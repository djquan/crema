use std::path::PathBuf;
use std::sync::Arc;

use iced::{Element, Task, Theme};
use tracing::{error, info};

use photors_catalog::db::Catalog;
use photors_catalog::models::{Photo, PhotoId};
use photors_core::image_buf::{EditParams, ImageBuf};
use photors_thumbnails::cache::ThumbnailCache;

use crate::views;
use crate::widgets::histogram::HistogramData;

#[derive(Debug, Clone)]
pub enum View {
    Lighttable,
    Darkroom(PhotoId),
}

pub struct App {
    view: View,
    catalog: Option<Catalog>,
    catalog_path: Option<String>,
    photos: Vec<Photo>,
    thumbnails: std::collections::HashMap<PhotoId, iced::widget::image::Handle>,

    // Darkroom state
    current_image: Option<Arc<ImageBuf>>,
    preview_image: Option<Arc<ImageBuf>>,
    processed_image: Option<iced::widget::image::Handle>,
    histogram: Option<Box<HistogramData>>,
    edit_params: EditParams,
    current_exif: Vec<(String, String)>,

    status_message: String,

    processing_generation: u64,
    thumbnail_cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Message {
    // Navigation
    OpenPhoto(PhotoId),
    BackToGrid,

    // Import
    ImportFolder,
    FolderSelected(Option<PathBuf>),
    ImportComplete(usize, usize),

    // Thumbnails
    ThumbnailReady(PhotoId, Vec<u8>),

    // Editing
    ExposureChanged(f32),
    WbTempChanged(f32),
    WbTintChanged(f32),
    CropXChanged(f32),
    CropYChanged(f32),
    CropWChanged(f32),
    CropHChanged(f32),

    // Image loading
    ImageLoaded(PhotoId, Arc<ImageBuf>, Arc<ImageBuf>, Vec<(String, String)>),
    ImageProcessed(u64, iced::widget::image::Handle, Box<HistogramData>),

    // Catalog
    CatalogOpened(String),
    PhotosListed(Vec<Photo>),

    Noop,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            view: View::Lighttable,
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
            status_message: "Welcome to Photors. Import a folder to get started.".into(),
            processing_generation: 0,
            thumbnail_cache_dir: dirs::cache_dir()
                .map(|d| d.join("photors").join("thumbnails")),
        };

        let default_catalog = dirs_catalog_path();
        let task = Task::perform(
            async move { default_catalog },
            Message::CatalogOpened,
        );

        (app, task)
    }

    pub fn title(&self) -> String {
        match &self.view {
            View::Lighttable => format!("Photors - {} photos", self.photos.len()),
            View::Darkroom(id) => {
                let name = self
                    .photos
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| {
                        std::path::Path::new(&p.file_path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    })
                    .unwrap_or_default();
                format!("Photors - {name}")
            }
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CatalogOpened(path) => {
                match Catalog::open(&path) {
                    Ok(catalog) => {
                        info!(%path, "catalog opened");
                        self.catalog = Some(catalog);
                        self.catalog_path = Some(path);
                        return self.refresh_photos();
                    }
                    Err(err) => {
                        error!(%err, "failed to open catalog");
                        self.status_message = format!("Error opening catalog: {err}");
                    }
                }
                Task::none()
            }

            Message::ImportFolder => {
                Task::perform(
                    async {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select folder to import")
                            .pick_folder()
                            .await;
                        handle.map(|h| h.path().to_path_buf())
                    },
                    Message::FolderSelected,
                )
            }

            Message::FolderSelected(Some(folder)) => {
                self.status_message = format!("Importing from {}...", folder.display());
                let catalog_path = self.catalog_path.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let catalog = Catalog::open(&catalog_path).ok();
                        if let Some(catalog) = catalog {
                            match photors_catalog::import::import_folder(&catalog, &folder) {
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

            Message::FolderSelected(None) => Task::none(),

            Message::ImportComplete(imported, errors) => {
                self.status_message = format!("Imported {imported} photos ({errors} errors)");
                self.refresh_photos()
            }

            Message::PhotosListed(photos) => {
                self.photos = photos;
                self.status_message = format!("{} photos in catalog", self.photos.len());
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

            Message::ThumbnailReady(id, bytes) => {
                let handle = iced::widget::image::Handle::from_bytes(bytes);
                self.thumbnails.insert(id, handle);
                Task::none()
            }

            Message::OpenPhoto(id) => {
                self.view = View::Darkroom(id);
                let photo = self.photos.iter().find(|p| p.id == id).cloned();

                // Load edits from catalog
                if let Some(ref catalog) = self.catalog {
                    if let Ok(Some(edit)) = catalog.get_edits(id) {
                        self.edit_params = edit.to_edit_params();
                    } else {
                        self.edit_params = EditParams::default();
                    }
                }

                if let Some(photo) = photo {
                    let path = photo.file_path.clone();
                    Task::perform(
                        async move {
                            let t0 = std::time::Instant::now();
                            let p = std::path::Path::new(&path);
                            let buf = photors_core::raw::load_any(p).ok()?;
                            let preview = buf.downsample(2048);
                            let exif = photors_metadata::exif::ExifData::from_file(p)
                                .ok()
                                .map(|e| e.summary_lines())
                                .unwrap_or_default();
                            info!(elapsed_ms = t0.elapsed().as_millis(), w = buf.width, h = buf.height, "image loaded");
                            Some((id, Arc::new(buf), Arc::new(preview), exif))
                        },
                        |result| match result {
                            Some((id, buf, preview, exif)) => Message::ImageLoaded(id, buf, preview, exif),
                            None => Message::Noop,
                        },
                    )
                } else {
                    Task::none()
                }
            }

            Message::ImageLoaded(_id, buf, preview, exif) => {
                self.current_image = Some(buf);
                self.preview_image = Some(preview);
                self.current_exif = exif;
                self.reprocess_image()
            }

            Message::ImageProcessed(generation, handle, hist) => {
                if generation != self.processing_generation {
                    return Task::none();
                }
                self.processed_image = Some(handle);
                self.histogram = Some(hist);
                self.save_current_edits();
                Task::none()
            }

            Message::BackToGrid => {
                self.save_current_edits();
                self.view = View::Lighttable;
                self.current_image = None;
                self.preview_image = None;
                self.processed_image = None;
                self.histogram = None;
                Task::none()
            }

            Message::ExposureChanged(v) => {
                self.edit_params.exposure = v;
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
            Message::CropXChanged(v) => {
                self.edit_params.crop_x = v;
                self.reprocess_image()
            }
            Message::CropYChanged(v) => {
                self.edit_params.crop_y = v;
                self.reprocess_image()
            }
            Message::CropWChanged(v) => {
                self.edit_params.crop_w = v;
                self.reprocess_image()
            }
            Message::CropHChanged(v) => {
                self.edit_params.crop_h = v;
                self.reprocess_image()
            }

            Message::Noop => Task::none(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.view {
            View::Lighttable => views::lighttable::view(self),
            View::Darkroom(_) => views::darkroom::view(self),
        }
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
            return Task::none();
        };

        self.processing_generation += 1;
        let generation = self.processing_generation;
        let buf = preview.clone();
        let params = self.edit_params.clone();

        Task::perform(
            async move {
                let pipeline = photors_core::pipeline::Pipeline::new();
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
            |(generation, handle, histogram)| Message::ImageProcessed(generation, handle, Box::new(histogram)),
        )
    }

    fn save_current_edits(&self) {
        if let (View::Darkroom(id), Some(catalog)) = (&self.view, &self.catalog)
            && let Err(err) = catalog.save_edits(*id, &self.edit_params)
        {
            error!(%err, "failed to save edits");
        }
    }

    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn thumbnails(&self) -> &std::collections::HashMap<PhotoId, iced::widget::image::Handle> {
        &self.thumbnails
    }

    pub fn edit_params(&self) -> &EditParams {
        &self.edit_params
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
}

fn dirs_catalog_path() -> String {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("photors");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("catalog.db").to_string_lossy().to_string()
}

fn thumbnail_cache_key(path: &std::path::Path) -> String {
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok();
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.to_string_lossy().as_bytes());
    if let Some(t) = mtime {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        hasher.update(&dur.as_nanos().to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn load_thumbnail_bytes(path: &str, cache_dir: Option<&std::path::Path>) -> anyhow::Result<Vec<u8>> {
    let p = std::path::Path::new(path);

    if let Some(dir) = cache_dir
        && let Ok(cache) = ThumbnailCache::new(dir.to_path_buf())
    {
        let key = thumbnail_cache_key(p);
        if let Some(bytes) = cache.load(&key) {
            return Ok(bytes);
        }
        let bytes = photors_thumbnails::generator::fast_thumbnail(p)?;
        cache.store(&key, &bytes).ok();
        return Ok(bytes);
    }

    photors_thumbnails::generator::fast_thumbnail(p)
}
