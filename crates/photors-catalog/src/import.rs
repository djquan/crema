use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::db::{Catalog, InsertPhoto};
use crate::models::PhotoId;
use photors_metadata::exif::ExifData;

pub struct ImportResult {
    pub imported: Vec<PhotoId>,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Scan a folder for supported images and import them into the catalog.
pub fn import_folder(catalog: &Catalog, folder: &Path) -> Result<ImportResult> {
    info!(?folder, "importing folder");

    let mut result = ImportResult {
        imported: Vec::new(),
        skipped: 0,
        errors: Vec::new(),
    };

    let entries: Vec<_> = fs::read_dir(folder)
        .with_context(|| format!("failed to read directory: {}", folder.display()))?
        .collect();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                result.errors.push(format!("readdir error: {err}"));
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if !photors_core::raw::is_supported_extension(ext) {
            continue;
        }

        match import_single_file(catalog, &path) {
            Ok(Some(id)) => result.imported.push(id),
            Ok(None) => result.skipped += 1,
            Err(err) => {
                warn!(?path, %err, "failed to import");
                result.errors.push(format!("{}: {err}", path.display()));
            }
        }
    }

    info!(
        imported = result.imported.len(),
        skipped = result.skipped,
        errors = result.errors.len(),
        "import complete"
    );

    Ok(result)
}

fn import_single_file(catalog: &Catalog, path: &Path) -> Result<Option<PhotoId>> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize: {}", path.display()))?;
    let file_path = canonical.to_string_lossy().to_string();

    let file_bytes = fs::read(&canonical)
        .with_context(|| format!("failed to read: {}", canonical.display()))?;
    let file_size = file_bytes.len() as i64;
    let file_hash = blake3::hash(&file_bytes).to_hex().to_string();

    let exif = ExifData::from_file(&canonical).ok();

    let insert = InsertPhoto {
        file_path,
        file_hash,
        file_size,
        width: exif.as_ref().and_then(|e| e.width),
        height: exif.as_ref().and_then(|e| e.height),
        camera_make: exif.as_ref().and_then(|e| e.camera_make.clone()),
        camera_model: exif.as_ref().and_then(|e| e.camera_model.clone()),
        lens: exif.as_ref().and_then(|e| e.lens.clone()),
        focal_length: exif.as_ref().and_then(|e| e.focal_length),
        aperture: exif.as_ref().and_then(|e| e.aperture),
        shutter_speed: exif.as_ref().and_then(|e| e.shutter_speed.clone()),
        iso: exif.as_ref().and_then(|e| e.iso),
        date_taken: exif.as_ref().and_then(|e| e.date_taken.clone()),
        thumbnail_path: None,
    };

    let id = catalog.insert_photo(&insert)?;
    if id == 0 {
        return Ok(None); // duplicate, was ignored
    }

    Ok(Some(id))
}
