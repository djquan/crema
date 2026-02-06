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

#[derive(Debug, Clone)]
pub enum View {
    Lighttable,
    Darkroom(PhotoId),
}

pub struct App {
    menu: Option<crate::menu::AppMenu>,
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

    date_filter: DateFilter,
    expanded_dates: HashSet<DateExpansionKey>,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    OpenPhoto(PhotoId),
    BackToGrid,

    // Import
    Import,
    ImportsSelected(Vec<PathBuf>),
    ImportComplete(usize, usize),

    // Thumbnails
    ThumbnailReady(PhotoId, Vec<u8>),

    // Editing
    ExposureChanged(f32),
    WbTempChanged(f32),
    WbTintChanged(f32),
    #[allow(dead_code)]
    CropXChanged(f32),
    #[allow(dead_code)]
    CropYChanged(f32),
    #[allow(dead_code)]
    CropWChanged(f32),
    #[allow(dead_code)]
    CropHChanged(f32),

    // Image loading
    ImageLoaded(PhotoId, Arc<ImageBuf>, Arc<ImageBuf>, Vec<(String, String)>),
    ImageProcessed(u64, iced::widget::image::Handle, Box<HistogramData>),

    // Catalog
    CatalogOpened(String),
    PhotosListed(Vec<Photo>),

    // Export
    Export,
    ExportPathSelected(PathBuf),
    ExportComplete(String),

    // Date sidebar
    SetDateFilter(DateFilter),
    ToggleDateExpansion(DateExpansionKey),

    Noop,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            menu: None,
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
            status_message: "Welcome to Crema. Import photos to get started.".into(),
            processing_generation: 0,
            thumbnail_cache_dir: dirs::cache_dir().map(|d| d.join("crema").join("thumbnails")),
            date_filter: DateFilter::All,
            expanded_dates: HashSet::new(),
        };

        let default_catalog = dirs_catalog_path();
        let task = Task::perform(async move { default_catalog }, Message::CatalogOpened);

        (app, task)
    }

    pub fn title(&self) -> String {
        match &self.view {
            View::Lighttable => format!("Crema - {} photos", self.photos.len()),
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
                format!("Crema - {name}")
            }
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

            Message::Import => Task::perform(
                async {
                    let dialog = rfd::AsyncFileDialog::new()
                        .set_title("Import photos")
                        .add_filter(
                            "Images",
                            &[
                                "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf",
                                "rw2", "orf", "pef", "dng", "3fr", "ari", "bay", "cap", "dcr",
                                "erf", "fff", "iiq", "k25", "kdc", "mef", "mos", "mrw", "raw",
                                "rwl", "srw", "x3f", "jpg", "jpeg", "png", "tiff", "tif",
                            ],
                        );
                    let handles = dialog.pick_files().await.unwrap_or_default();
                    handles.iter().map(|h| h.path().to_path_buf()).collect()
                },
                Message::ImportsSelected,
            ),

            Message::ImportsSelected(paths) if paths.is_empty() => Task::none(),

            Message::ImportsSelected(paths) => {
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
                        |result| match result {
                            Some((id, buf, preview, exif)) => {
                                Message::ImageLoaded(id, buf, preview, exif)
                            }
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
                if let Some(menu) = &self.menu {
                    menu.export_item.set_enabled(true);
                }
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
                if let Some(menu) = &self.menu {
                    menu.export_item.set_enabled(false);
                }
                Task::none()
            }

            Message::Export => {
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

            Message::ExportPathSelected(path) => {
                let Some(ref full_res) = self.current_image else {
                    return Task::none();
                };
                self.status_message = "Exporting...".into();
                let buf = ImageBuf::clone(full_res);
                let params = self.edit_params.clone();
                Task::perform(
                    async move { export_image(buf, &params, &path) },
                    Message::ExportComplete,
                )
            }

            Message::ExportComplete(msg) => {
                self.status_message = msg;
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

            Message::SetDateFilter(f) => {
                self.date_filter = f;
                Task::none()
            }

            Message::ToggleDateExpansion(key) => {
                if !self.expanded_dates.remove(&key) {
                    self.expanded_dates.insert(key);
                }
                Task::none()
            }

            Message::Noop => Task::none(),
        }
    }

    pub fn subscription(&self) -> iced::Subscription<Message> {
        crate::menu::subscription()
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

    pub fn date_filter(&self) -> &DateFilter {
        &self.date_filter
    }

    pub fn expanded_dates(&self) -> &HashSet<DateExpansionKey> {
        &self.expanded_dates
    }

    pub fn filtered_photos(&self) -> Vec<&Photo> {
        self.photos
            .iter()
            .filter(|p| self.date_filter.matches(p))
            .collect()
    }

    fn default_export_filename(&self) -> String {
        if let View::Darkroom(id) = &self.view
            && let Some(photo) = self.photos.iter().find(|p| p.id == *id)
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
        // 4x2 image with known pixel values in linear light
        let mut data = vec![0.0f32; 4 * 2 * 3];
        // Top-left: red-ish
        data[0] = 0.8;
        data[1] = 0.1;
        data[2] = 0.1;
        // Bottom-right: blue-ish
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

        // .JPG should still go through the JPEG encoder path
        let msg = export_image(test_image(), &EditParams::default(), &path);
        assert!(msg.starts_with("Exported to"), "unexpected: {msg}");

        // Verify it's a valid JPEG by reading magic bytes
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

        // PNG magic bytes
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

        // Export with default params (no edits)
        let path_default = dir.path().join("default.png");
        export_image(test_image(), &EditParams::default(), &path_default);

        // Export with +2 EV exposure boost
        let bright_params = EditParams {
            exposure: 2.0,
            ..EditParams::default()
        };
        let path_bright = dir.path().join("bright.png");
        export_image(test_image(), &bright_params, &path_bright);

        let img_default = image::open(&path_default).unwrap().into_rgba8();
        let img_bright = image::open(&path_bright).unwrap().into_rgba8();

        // The bright version should have higher pixel values
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

        // A uniform mid-gray image
        let buf = ImageBuf::from_data(2, 2, vec![0.5; 2 * 2 * 3]).unwrap();
        export_image(buf, &EditParams::default(), &path);

        let img = image::open(&path).unwrap().into_rgba8();
        // All pixels should be the same value (uniform input, identity edits)
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
