use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

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

        match import_file(catalog, &path) {
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

/// Import a mixed list of files and directories into the catalog.
///
/// Directories are scanned for supported images. Files are imported directly
/// if they have a supported extension.
pub fn import_paths(catalog: &Catalog, paths: &[PathBuf]) -> Result<ImportResult> {
    info!(count = paths.len(), "importing paths");

    let mut result = ImportResult {
        imported: Vec::new(),
        skipped: 0,
        errors: Vec::new(),
    };

    for path in paths {
        if path.is_dir() {
            match import_folder(catalog, path) {
                Ok(sub) => {
                    result.imported.extend(sub.imported);
                    result.skipped += sub.skipped;
                    result.errors.extend(sub.errors);
                }
                Err(err) => {
                    result.errors.push(format!("{}: {err}", path.display()));
                }
            }
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !crema_core::raw::is_supported_extension(ext) {
                continue;
            }
            match import_file(catalog, path) {
                Ok(Some(id)) => result.imported.push(id),
                Ok(None) => result.skipped += 1,
                Err(err) => {
                    warn!(?path, %err, "failed to import");
                    result.errors.push(format!("{}: {err}", path.display()));
                }
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

fn sidecar_path(photo_path: &Path) -> PathBuf {
    photo_path.with_extension("crema.json")
}

pub fn import_file(catalog: &Catalog, path: &Path) -> Result<Option<PhotoId>> {
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

    let photo_id = catalog.insert_photo(&insert)?;

    if let Some(id) = photo_id {
        let sidecar = sidecar_path(&canonical);
        if sidecar.is_file() {
            match fs::read_to_string(&sidecar) {
                Ok(json) => {
                    match serde_json::from_str::<crema_core::image_buf::EditParams>(&json) {
                        Ok(params) => {
                            if let Err(err) = catalog.save_edits(id, &params) {
                                warn!(?sidecar, %err, "failed to save sidecar edits");
                            } else {
                                info!(?sidecar, "loaded sidecar edits on import");
                            }
                        }
                        Err(err) => {
                            warn!(?sidecar, %err, "failed to parse sidecar JSON");
                        }
                    }
                }
                Err(err) => {
                    warn!(?sidecar, %err, "failed to read sidecar file");
                }
            }
        }
    }

    Ok(photo_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_minimal_jpeg(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        // Minimal valid JPEG: SOI + content bytes + EOI
        f.write_all(&[0xFF, 0xD8]).unwrap();
        f.write_all(content).unwrap();
        f.write_all(&[0xFF, 0xD9]).unwrap();
        f.flush().unwrap();
        path
    }

    fn create_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn import_file_creates_photo_record() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg_path = create_minimal_jpeg(dir.path(), "photo.jpg", b"test data");
        let catalog = Catalog::open_in_memory().unwrap();

        let id = import_file(&catalog, &jpeg_path).unwrap();
        assert!(id.is_some(), "should return Some(id) for new import");

        let photo = catalog.get_photo(id.unwrap()).unwrap().unwrap();
        assert!(photo.file_path.ends_with("photo.jpg"));
        assert!(!photo.file_hash.is_empty());
        assert!(photo.file_size > 0);
    }

    #[test]
    fn import_file_duplicate_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg_path = create_minimal_jpeg(dir.path(), "dup.jpg", b"same content");
        let catalog = Catalog::open_in_memory().unwrap();

        let first = import_file(&catalog, &jpeg_path).unwrap();
        assert!(first.is_some());

        let second = import_file(&catalog, &jpeg_path).unwrap();
        assert!(second.is_none(), "duplicate path should return None");

        assert_eq!(catalog.photo_count().unwrap(), 1);
    }

    #[test]
    fn import_file_nonexistent_path_errors() {
        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_file(&catalog, Path::new("/does/not/exist.jpg"));
        assert!(result.is_err());
    }

    #[test]
    fn import_file_computes_blake3_hash_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"\xFF\xD8test payload\xFF\xD9";
        let path = create_file(dir.path(), "hashed.jpg", content);
        let catalog = Catalog::open_in_memory().unwrap();

        let id = import_file(&catalog, &path).unwrap().unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();

        let expected_hash = blake3::hash(content).to_hex().to_string();
        assert_eq!(photo.file_hash, expected_hash);
    }

    #[test]
    fn import_file_hash_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"\xFF\xD8deterministic\xFF\xD9";
        let path1 = create_file(dir.path(), "a.jpg", content);
        let path2 = create_file(dir.path(), "b.jpg", content);
        let catalog = Catalog::open_in_memory().unwrap();

        let id1 = import_file(&catalog, &path1).unwrap().unwrap();
        let id2 = import_file(&catalog, &path2).unwrap().unwrap();

        let photo1 = catalog.get_photo(id1).unwrap().unwrap();
        let photo2 = catalog.get_photo(id2).unwrap().unwrap();

        assert_eq!(photo1.file_hash, photo2.file_hash);
    }

    #[test]
    fn import_file_stores_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let payload = vec![0u8; 1000];
        let path = create_file(dir.path(), "sized.jpg", &payload);
        let catalog = Catalog::open_in_memory().unwrap();

        let id = import_file(&catalog, &path).unwrap().unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();
        assert_eq!(photo.file_size, 1000);
    }

    #[test]
    fn import_file_uses_canonical_path() {
        let dir = tempfile::tempdir().unwrap();
        create_minimal_jpeg(dir.path(), "canonical.jpg", b"data");
        let catalog = Catalog::open_in_memory().unwrap();

        // Use a relative-style path that canonicalize will resolve
        let path = dir.path().join("./canonical.jpg");
        let id = import_file(&catalog, &path).unwrap().unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();
        let stored = PathBuf::from(&photo.file_path);
        let expected = path.canonicalize().unwrap();

        assert!(stored.is_absolute());
        assert_eq!(stored, expected);
    }

    #[test]
    fn import_folder_imports_supported_extensions() {
        let dir = tempfile::tempdir().unwrap();
        create_minimal_jpeg(dir.path(), "a.jpg", b"aaa");
        create_minimal_jpeg(dir.path(), "b.jpeg", b"bbb");
        create_minimal_jpeg(dir.path(), "c.png", b"ccc");
        create_file(dir.path(), "readme.txt", b"not an image");
        create_file(dir.path(), "notes.md", b"also not an image");

        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_folder(&catalog, dir.path()).unwrap();

        assert_eq!(result.imported.len(), 3);
        assert_eq!(result.skipped, 0);
        assert!(result.errors.is_empty());
        assert_eq!(catalog.photo_count().unwrap(), 3);
    }

    #[test]
    fn import_folder_skips_unsupported_files() {
        let dir = tempfile::tempdir().unwrap();
        create_file(dir.path(), "doc.pdf", b"pdf content");
        create_file(dir.path(), "data.csv", b"csv content");

        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_folder(&catalog, dir.path()).unwrap();

        assert_eq!(result.imported.len(), 0);
        assert_eq!(catalog.photo_count().unwrap(), 0);
    }

    #[test]
    fn import_folder_skips_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        create_minimal_jpeg(dir.path(), "top.jpg", b"top");
        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        create_minimal_jpeg(&subdir, "nested.jpg", b"nested");

        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_folder(&catalog, dir.path()).unwrap();

        // Only top-level files should be imported
        assert_eq!(result.imported.len(), 1);
    }

    #[test]
    fn import_folder_nonexistent_errors() {
        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_folder(&catalog, Path::new("/nonexistent/dir"));
        assert!(result.is_err());
    }

    #[test]
    fn import_folder_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_folder(&catalog, dir.path()).unwrap();

        assert_eq!(result.imported.len(), 0);
        assert_eq!(result.skipped, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn import_folder_handles_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        create_minimal_jpeg(dir.path(), "photo.jpg", b"data");

        let catalog = Catalog::open_in_memory().unwrap();

        let r1 = import_folder(&catalog, dir.path()).unwrap();
        assert_eq!(r1.imported.len(), 1);

        let r2 = import_folder(&catalog, dir.path()).unwrap();
        assert_eq!(r2.imported.len(), 0);
        assert_eq!(r2.skipped, 1);

        assert_eq!(catalog.photo_count().unwrap(), 1);
    }

    #[test]
    fn import_paths_mixed_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();

        // A directory with images
        let subdir = dir.path().join("photos");
        fs::create_dir(&subdir).unwrap();
        create_minimal_jpeg(&subdir, "from_dir.jpg", b"dir_img");

        // A standalone file
        let standalone = create_minimal_jpeg(dir.path(), "standalone.png", b"standalone");

        let catalog = Catalog::open_in_memory().unwrap();
        let paths = vec![subdir.clone(), standalone];
        let result = import_paths(&catalog, &paths).unwrap();

        assert_eq!(result.imported.len(), 2);
        assert_eq!(catalog.photo_count().unwrap(), 2);
    }

    #[test]
    fn import_paths_skips_unsupported_standalone_files() {
        let dir = tempfile::tempdir().unwrap();
        let txt = create_file(dir.path(), "notes.txt", b"text");
        let jpg = create_minimal_jpeg(dir.path(), "real.jpg", b"image");

        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_paths(&catalog, &[txt, jpg]).unwrap();

        assert_eq!(result.imported.len(), 1);
    }

    #[test]
    fn import_paths_empty_list() {
        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_paths(&catalog, &[]).unwrap();

        assert_eq!(result.imported.len(), 0);
        assert_eq!(result.skipped, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn import_file_no_exif_still_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        // A minimal JPEG with no EXIF data
        let path = create_minimal_jpeg(dir.path(), "no_exif.jpg", b"plain jpeg content");
        let catalog = Catalog::open_in_memory().unwrap();

        let id = import_file(&catalog, &path).unwrap().unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();

        // EXIF fields should be None since there's no valid EXIF
        assert!(photo.camera_make.is_none());
        assert!(photo.camera_model.is_none());
        assert!(photo.focal_length.is_none());
        assert!(photo.iso.is_none());
    }

    #[test]
    fn import_file_loads_sidecar_edits() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg_path = create_minimal_jpeg(dir.path(), "edited.jpg", b"has edits");

        let mut params = crema_core::image_buf::EditParams::default();
        params.exposure = 1.5;
        params.wb_temp = 5500.0;
        params.contrast = 0.3;
        let sidecar = dir.path().join("edited.crema.json");
        fs::write(&sidecar, serde_json::to_string_pretty(&params).unwrap()).unwrap();

        let catalog = Catalog::open_in_memory().unwrap();
        let id = import_file(&catalog, &jpeg_path).unwrap().unwrap();

        let edits = catalog.get_edits(id).unwrap();
        assert!(edits.is_some(), "sidecar edits should be imported");
        let edits = edits.unwrap();
        assert!((edits.exposure - 1.5).abs() < f32::EPSILON);
        assert!((edits.wb_temp - 5500.0).abs() < f32::EPSILON);
        assert!((edits.contrast - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn import_file_no_sidecar_no_edits() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg_path = create_minimal_jpeg(dir.path(), "plain.jpg", b"no sidecar");
        let catalog = Catalog::open_in_memory().unwrap();

        let id = import_file(&catalog, &jpeg_path).unwrap().unwrap();
        let edits = catalog.get_edits(id).unwrap();
        assert!(edits.is_none(), "no sidecar means no edits");
    }

    #[test]
    fn import_file_invalid_sidecar_still_imports_photo() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg_path = create_minimal_jpeg(dir.path(), "bad_sidecar.jpg", b"data");

        let sidecar = dir.path().join("bad_sidecar.crema.json");
        fs::write(&sidecar, "{ not valid json!!!").unwrap();

        let catalog = Catalog::open_in_memory().unwrap();
        let id = import_file(&catalog, &jpeg_path).unwrap();
        assert!(
            id.is_some(),
            "photo should still import despite bad sidecar"
        );

        let edits = catalog.get_edits(id.unwrap()).unwrap();
        assert!(edits.is_none(), "bad sidecar should not produce edits");
    }

    #[test]
    fn import_result_accumulates_across_import_paths() {
        let dir = tempfile::tempdir().unwrap();

        let dir_a = dir.path().join("a");
        let dir_b = dir.path().join("b");
        fs::create_dir(&dir_a).unwrap();
        fs::create_dir(&dir_b).unwrap();

        create_minimal_jpeg(&dir_a, "1.jpg", b"one");
        create_minimal_jpeg(&dir_a, "2.jpg", b"two");
        create_minimal_jpeg(&dir_b, "3.jpg", b"three");

        let standalone = create_minimal_jpeg(dir.path(), "4.png", b"four");

        let catalog = Catalog::open_in_memory().unwrap();
        let result = import_paths(&catalog, &[dir_a, dir_b, standalone]).unwrap();

        assert_eq!(result.imported.len(), 4);
        assert_eq!(catalog.photo_count().unwrap(), 4);
    }
}
