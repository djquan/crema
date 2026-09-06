use crate::jobs::{Event, Job, JobKey, Jobs, Purpose};
use crema_core::{AssetCandidate, AssetId};
use crema_image::{CandidateFormat, DecodeOutcome, FailureClass, Provenance, SourceMetadata};
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions, Vec2};
use std::{collections::HashMap, path::PathBuf};

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

    fn shortcut(&mut self, key: egui::Key, wants_keyboard_input: bool) {
        if key == egui::Key::Escape && self.view == View::Viewer {
            self.view = View::Grid;
            return;
        }
        if wants_keyboard_input {
            return;
        }
        match key {
            egui::Key::ArrowLeft => self.navigate(-1),
            egui::Key::ArrowRight => self.navigate(1),
            egui::Key::Enter => self.view = View::Viewer,
            _ => {}
        }
    }
}

enum Cached {
    Image {
        texture: TextureHandle,
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

impl Cached {
    fn bytes(&self) -> usize {
        match self {
            Self::Image { texture, .. } => texture.size()[0] * texture.size()[1] * 4,
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

pub struct Browser {
    root: PathBuf,
    workspace: Workspace,
    jobs: Jobs,
    cache: HashMap<JobKey, Cached>,
    last_demand: Vec<JobKey>,
    tick: u64,
    actual_pixels: bool,
    thumbnail_width: f32,
}

impl Browser {
    pub fn new(root: PathBuf, executable: PathBuf, context: egui::Context) -> Self {
        context.set_visuals(egui::Visuals::dark());
        let mut style = (*context.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.visuals.selection.bg_fill = Color32::from_rgb(99, 116, 93);
        style.visuals.panel_fill = Color32::from_rgb(27, 28, 27);
        context.set_style_of(egui::Theme::Dark, style);
        let jobs = Jobs::new(executable, context);
        jobs.scan(root.clone(), 1);
        Self {
            root,
            workspace: Workspace::default(),
            jobs,
            cache: HashMap::new(),
            last_demand: Vec::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
        }
    }

    fn key(&self, asset: AssetId, purpose: Purpose) -> JobKey {
        JobKey {
            generation: self.workspace.generation,
            asset,
            purpose,
        }
    }

    fn receive(&mut self, context: &egui::Context) {
        for _ in 0..32 {
            let Ok(event) = self.jobs.receiver.try_recv() else {
                return;
            };
            match event {
                Event::Candidate {
                    generation,
                    candidate,
                } => self.workspace.insert(generation, candidate),
                Event::ScanFinished {
                    generation,
                    failures,
                } if generation == self.workspace.generation => {
                    self.workspace.scanning = false;
                    self.workspace.failures = failures;
                }
                Event::ScanFinished { .. } => {}
                Event::Decoded { key, outcome } if self.workspace.accepts(key) => {
                    let cached = match outcome {
                        DecodeOutcome::Decoded(result) => {
                            self.evict(key.purpose, result.preview.rgba8().len());
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                result.preview.dimensions_usize(),
                                result.preview.rgba8(),
                            );
                            let texture = context.load_texture(
                                format!("{}-{:?}", key.asset, key.purpose),
                                image,
                                TextureOptions::LINEAR,
                            );
                            Cached::Image {
                                texture,
                                metadata: result.metadata,
                                provenance: result.provenance,
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
                    while let Some(oldest) = unavailable_to_evict(&self.cache, key.purpose, 256) {
                        self.cache.remove(&oldest);
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
                self.cache.remove(&key);
            } else {
                break;
            }
        }
    }

    fn demand(&mut self, mut keys: Vec<JobKey>) {
        keys.retain(|key| !self.cache.contains_key(key));
        keys.dedup();
        if keys == self.last_demand {
            return;
        }
        let jobs = keys
            .iter()
            .filter_map(|key| {
                self.workspace.index.get(&key.asset).map(|index| {
                    let candidate = &self.workspace.assets[*index];
                    Job {
                        key: *key,
                        path: candidate.path().to_owned(),
                        format: *candidate.kind(),
                    }
                })
            })
            .collect();
        self.jobs.replace(jobs);
        self.last_demand = keys;
    }

    fn retry_selected(&mut self) {
        if let Some(asset) = self.workspace.selected {
            for purpose in [Purpose::Thumbnail, Purpose::Viewer] {
                self.cache.remove(&self.key(asset, purpose));
            }
            self.last_demand.clear();
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> Vec<JobKey> {
        let columns = ((ui.available_width() + 12.0) / (self.thumbnail_width + 12.0))
            .floor()
            .max(1.0) as usize;
        let row_height = self.thumbnail_width * 0.78 + 50.0;
        let row_count = self.workspace.assets.len().div_ceil(columns);
        let mut demanded = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("photo-grid")
            .show_rows(ui, row_height, row_count, |ui, rows| {
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
                                        let (rect, mut response) = ui.allocate_exact_size(
                                            Vec2::new(
                                                self.thumbnail_width,
                                                self.thumbnail_width * 0.78,
                                            ),
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
                                                let scale = (rect.width() / size.x)
                                                    .min(rect.height() / size.y);
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
                                        let accessible_name = match unavailable {
                                            Some(reason) => {
                                                response = response.on_hover_text(&reason);
                                                format!("{filename}. Preview unavailable. {reason}")
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
                                        ui.add(egui::Label::new(&filename).truncate());
                                        ui.label(
                                            RichText::new(format).small().color(Color32::GRAY),
                                        );
                                        response
                                    },
                                )
                                .inner;
                            if response.clicked() {
                                self.workspace.selected = Some(id);
                            }
                            if response.double_clicked() {
                                self.workspace.selected = Some(id);
                                self.workspace.view = View::Viewer;
                            }
                        }
                    });
                }
            });
        demanded
    }

    fn viewer(&mut self, ui: &mut egui::Ui) -> Vec<JobKey> {
        let Some(id) = self.workspace.selected else {
            return Vec::new();
        };
        let key = self.key(id, Purpose::Viewer);
        let thumbnail = self.key(id, Purpose::Thumbnail);
        let display_key = viewer_display_key(&self.cache, key, thumbnail);
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
        match self.cache.get_mut(&display_key) {
            Some(Cached::Image {
                texture,
                metadata,
                provenance,
                used,
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
                let available = ui.available_size();
                let source = texture.size_vec2();
                if self.actual_pixels {
                    egui::ScrollArea::both()
                        .id_salt(("actual-preview", id))
                        .show(ui, |ui| {
                            ui.add(egui::Image::new((texture.id(), source)));
                        });
                } else {
                    let scale = (available.x / source.x)
                        .min(available.y / source.y)
                        .max(0.01);
                    ui.centered_and_justified(|ui| {
                        ui.add(egui::Image::new((texture.id(), source * scale)));
                    });
                }
            }
            Some(Cached::Unavailable { reason, used, .. }) => {
                *used = self.tick;
                ui.centered_and_justified(|ui| {
                    ui.label(reason.as_str());
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
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick += 1;
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
                let can_retry = self.workspace.selected.is_some_and(|asset| {
                    [Purpose::Thumbnail, Purpose::Viewer]
                        .into_iter()
                        .any(|purpose| {
                            self.cache
                                .get(&self.key(asset, purpose))
                                .is_some_and(Cached::retryable)
                        })
                });
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
                if ui.input(|input| input.key_pressed(key)) {
                    self.workspace.shortcut(key, wants_keyboard_input);
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
            let demands = match self.workspace.view {
                View::Grid => self.grid(ui),
                View::Viewer => self.viewer(ui),
            };
            self.demand(demands);
        });
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
        let first = workspace.selected;
        for key in [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::Enter,
        ] {
            workspace.shortcut(key, true);
        }
        assert_eq!(workspace.selected, first);
        assert_eq!(workspace.view, View::Grid);
        workspace.shortcut(egui::Key::Enter, false);
        assert_eq!(workspace.view, View::Viewer);
        workspace.shortcut(egui::Key::Escape, true);
        assert_eq!(workspace.view, View::Grid);
        workspace.shortcut(egui::Key::ArrowRight, false);
        assert_eq!(workspace.selected, Some(workspace.assets[1].id()));
        fs::remove_dir_all(root).unwrap();
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
    fn larger_preview_failure_preserves_thumbnail_and_retry_clears_only_selection() {
        let context = egui::Context::default();
        let jobs = Jobs::new(std::env::current_exe().unwrap(), context.clone());
        let asset = test_asset();
        let mut browser = Browser {
            root: PathBuf::new(),
            workspace: Workspace {
                selected: Some(asset),
                ..Workspace::default()
            },
            jobs,
            cache: HashMap::new(),
            last_demand: Vec::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
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
        browser.last_demand.push(viewer);
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
        browser.retry_selected();
        assert!(!browser.cache.contains_key(&viewer));
        assert!(!browser.cache.contains_key(&thumbnail));
        assert!(browser.cache.contains_key(&other));
        assert!(browser.last_demand.is_empty());
        assert_eq!(browser.workspace.selected, Some(asset));
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
    fn idle_logic_does_not_request_repaint() {
        let context = egui::Context::default();
        let jobs = Jobs::new(std::env::current_exe().unwrap(), context.clone());
        let mut browser = Browser {
            root: PathBuf::new(),
            workspace: Workspace::default(),
            jobs,
            cache: HashMap::new(),
            last_demand: Vec::new(),
            tick: 0,
            actual_pixels: false,
            thumbnail_width: 220.0,
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
