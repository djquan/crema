use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::db::{Catalog, InsertPhoto};
use crate::models::PhotoId;
use crema_metadata::exif::ExifData;

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

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if !crema_core::raw::is_supported_extension(ext) {
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

    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to stat: {}", canonical.display()))?;
    let file_size = metadata.len() as i64;

    let file_hash = {
        let mut file = fs::File::open(&canonical)
            .with_context(|| format!("failed to open: {}", canonical.display()))?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = file
                .read(&mut buf)
                .with_context(|| format!("failed to read: {}", canonical.display()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        hasher.finalize().to_hex().to_string()
    };

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

    catalog.insert_photo(&insert)
}
