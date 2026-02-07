use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use tracing::info;

use crate::models::{EditRecord, Photo, PhotoId};

pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).context("failed to open catalog database")?;
        let catalog = Self { conn };
        catalog.migrate()?;
        Ok(catalog)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let catalog = Self { conn };
        catalog.migrate()?;
        Ok(catalog)
    }

    fn migrate(&self) -> Result<()> {
        info!("running catalog migrations");
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS photos (
                id           INTEGER PRIMARY KEY,
                file_path    TEXT NOT NULL UNIQUE,
                file_hash    TEXT NOT NULL,
                file_size    INTEGER NOT NULL,
                width        INTEGER,
                height       INTEGER,
                camera_make  TEXT,
                camera_model TEXT,
                lens         TEXT,
                focal_length REAL,
                aperture     REAL,
                shutter_speed TEXT,
                iso          INTEGER,
                date_taken   TEXT,
                imported_at  TEXT NOT NULL DEFAULT (datetime('now')),
                thumbnail_path TEXT
            );

            CREATE TABLE IF NOT EXISTS edits (
                id         INTEGER PRIMARY KEY,
                photo_id   INTEGER NOT NULL UNIQUE REFERENCES photos(id),
                exposure   REAL NOT NULL DEFAULT 0.0,
                wb_temp    REAL NOT NULL DEFAULT 5500.0,
                wb_tint    REAL NOT NULL DEFAULT 0.0,
                crop_x     REAL NOT NULL DEFAULT 0.0,
                crop_y     REAL NOT NULL DEFAULT 0.0,
                crop_w     REAL NOT NULL DEFAULT 1.0,
                crop_h     REAL NOT NULL DEFAULT 1.0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_photos_hash ON photos(file_hash);
            ",
        )?;

        let alter_stmts = [
            "ALTER TABLE edits ADD COLUMN contrast REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE edits ADD COLUMN highlights REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE edits ADD COLUMN shadows REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE edits ADD COLUMN blacks REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE edits ADD COLUMN vibrance REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE edits ADD COLUMN saturation REAL NOT NULL DEFAULT 0.0",
        ];
        for stmt in alter_stmts {
            match self.conn.execute(stmt, []) {
                Ok(_) => {}
                Err(e) if e.to_string().contains("duplicate column") => {}
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    /// Insert a photo, returning `Some(id)` if inserted, `None` if the path already exists.
    pub fn insert_photo(&self, photo: &InsertPhoto) -> Result<Option<PhotoId>> {
        self.conn.execute(
            "INSERT OR IGNORE INTO photos (
                file_path, file_hash, file_size, width, height,
                camera_make, camera_model, lens, focal_length, aperture,
                shutter_speed, iso, date_taken, thumbnail_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                photo.file_path,
                photo.file_hash,
                photo.file_size,
                photo.width,
                photo.height,
                photo.camera_make,
                photo.camera_model,
                photo.lens,
                photo.focal_length,
                photo.aperture,
                photo.shutter_speed,
                photo.iso,
                photo.date_taken,
                photo.thumbnail_path,
            ],
        )?;
        if self.conn.changes() == 0 {
            Ok(None)
        } else {
            Ok(Some(self.conn.last_insert_rowid()))
        }
    }

    pub fn get_photo(&self, id: PhotoId) -> Result<Option<Photo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, file_hash, file_size, width, height,
                    camera_make, camera_model, lens, focal_length, aperture,
                    shutter_speed, iso, date_taken, imported_at, thumbnail_path
             FROM photos WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_photo)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_photos(&self) -> Result<Vec<Photo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, file_hash, file_size, width, height,
                    camera_make, camera_model, lens, focal_length, aperture,
                    shutter_speed, iso, date_taken, imported_at, thumbnail_path
             FROM photos ORDER BY date_taken DESC, id DESC",
        )?;
        let photos = stmt
            .query_map([], row_to_photo)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(photos)
    }

    pub fn update_thumbnail(&self, id: PhotoId, thumbnail_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE photos SET thumbnail_path = ?1 WHERE id = ?2",
            params![thumbnail_path, id],
        )?;
        Ok(())
    }

    pub fn get_edits(&self, photo_id: PhotoId) -> Result<Option<EditRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, photo_id, exposure, wb_temp, wb_tint,
                    contrast, highlights, shadows, blacks, vibrance, saturation,
                    crop_x, crop_y, crop_w, crop_h, updated_at
             FROM edits WHERE photo_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![photo_id], |row| {
            Ok(EditRecord {
                id: row.get(0)?,
                photo_id: row.get(1)?,
                exposure: row.get(2)?,
                wb_temp: row.get(3)?,
                wb_tint: row.get(4)?,
                contrast: row.get(5)?,
                highlights: row.get(6)?,
                shadows: row.get(7)?,
                blacks: row.get(8)?,
                vibrance: row.get(9)?,
                saturation: row.get(10)?,
                crop_x: row.get(11)?,
                crop_y: row.get(12)?,
                crop_w: row.get(13)?,
                crop_h: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn save_edits(
        &self,
        photo_id: PhotoId,
        params: &crema_core::image_buf::EditParams,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO edits (photo_id, exposure, wb_temp, wb_tint,
                                contrast, highlights, shadows, blacks, vibrance, saturation,
                                crop_x, crop_y, crop_w, crop_h)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(photo_id) DO UPDATE SET
                exposure = excluded.exposure,
                wb_temp = excluded.wb_temp,
                wb_tint = excluded.wb_tint,
                contrast = excluded.contrast,
                highlights = excluded.highlights,
                shadows = excluded.shadows,
                blacks = excluded.blacks,
                vibrance = excluded.vibrance,
                saturation = excluded.saturation,
                crop_x = excluded.crop_x,
                crop_y = excluded.crop_y,
                crop_w = excluded.crop_w,
                crop_h = excluded.crop_h,
                updated_at = datetime('now')",
            params![
                photo_id,
                params.exposure,
                params.wb_temp,
                params.wb_tint,
                params.contrast,
                params.highlights,
                params.shadows,
                params.blacks,
                params.vibrance,
                params.saturation,
                params.crop_x,
                params.crop_y,
                params.crop_w,
                params.crop_h,
            ],
        )?;
        Ok(())
    }

    pub fn photo_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?)
    }
}

fn row_to_photo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: row.get(0)?,
        file_path: row.get(1)?,
        file_hash: row.get(2)?,
        file_size: row.get(3)?,
        width: row.get(4)?,
        height: row.get(5)?,
        camera_make: row.get(6)?,
        camera_model: row.get(7)?,
        lens: row.get(8)?,
        focal_length: row.get(9)?,
        aperture: row.get(10)?,
        shutter_speed: row.get(11)?,
        iso: row.get(12)?,
        date_taken: row.get(13)?,
        imported_at: row.get(14)?,
        thumbnail_path: row.get(15)?,
    })
}

pub struct InsertPhoto {
    pub file_path: String,
    pub file_hash: String,
    pub file_size: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub iso: Option<u32>,
    pub date_taken: Option<String>,
    pub thumbnail_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list_photos() {
        let catalog = Catalog::open_in_memory().unwrap();

        let photo = InsertPhoto {
            file_path: "/test/photo.jpg".to_string(),
            file_hash: "abc123".to_string(),
            file_size: 1024,
            width: Some(4000),
            height: Some(3000),
            camera_make: Some("Canon".to_string()),
            camera_model: Some("EOS R5".to_string()),
            lens: None,
            focal_length: Some(50.0),
            aperture: Some(2.8),
            shutter_speed: Some("1/200".to_string()),
            iso: Some(400),
            date_taken: Some("2024-01-15T10:30:00".to_string()),
            thumbnail_path: None,
        };

        let id = catalog
            .insert_photo(&photo)
            .unwrap()
            .expect("should insert");
        assert!(id > 0);

        let photos = catalog.list_photos().unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, "/test/photo.jpg");
        assert_eq!(photos[0].camera_make.as_deref(), Some("Canon"));
    }

    #[test]
    fn duplicate_path_ignored() {
        let catalog = Catalog::open_in_memory().unwrap();

        let photo = InsertPhoto {
            file_path: "/test/dup.jpg".to_string(),
            file_hash: "hash1".to_string(),
            file_size: 100,
            width: None,
            height: None,
            camera_make: None,
            camera_model: None,
            lens: None,
            focal_length: None,
            aperture: None,
            shutter_speed: None,
            iso: None,
            date_taken: None,
            thumbnail_path: None,
        };

        let first = catalog.insert_photo(&photo).unwrap();
        assert!(first.is_some());
        let second = catalog.insert_photo(&photo).unwrap();
        assert!(second.is_none());

        assert_eq!(catalog.photo_count().unwrap(), 1);
    }

    #[test]
    fn save_and_load_edits() {
        let catalog = Catalog::open_in_memory().unwrap();

        let photo = InsertPhoto {
            file_path: "/test/edit.jpg".to_string(),
            file_hash: "hash2".to_string(),
            file_size: 200,
            width: None,
            height: None,
            camera_make: None,
            camera_model: None,
            lens: None,
            focal_length: None,
            aperture: None,
            shutter_speed: None,
            iso: None,
            date_taken: None,
            thumbnail_path: None,
        };

        let id = catalog.insert_photo(&photo).unwrap().unwrap();

        let params = crema_core::image_buf::EditParams {
            exposure: 1.5,
            wb_temp: 6500.0,
            wb_tint: -5.0,
            ..Default::default()
        };
        catalog.save_edits(id, &params).unwrap();

        let edit = catalog.get_edits(id).unwrap().unwrap();
        assert!((edit.exposure - 1.5).abs() < 1e-6);
        assert!((edit.wb_temp - 6500.0).abs() < 1e-6);
    }

    fn minimal_photo(path: &str) -> InsertPhoto {
        InsertPhoto {
            file_path: path.to_string(),
            file_hash: format!("hash_{path}"),
            file_size: 100,
            width: None,
            height: None,
            camera_make: None,
            camera_model: None,
            lens: None,
            focal_length: None,
            aperture: None,
            shutter_speed: None,
            iso: None,
            date_taken: None,
            thumbnail_path: None,
        }
    }

    #[test]
    fn get_photo_by_id() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/a.jpg"))
            .unwrap()
            .unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();
        assert_eq!(photo.file_path, "/a.jpg");
    }

    #[test]
    fn get_nonexistent_photo() {
        let catalog = Catalog::open_in_memory().unwrap();
        assert!(catalog.get_photo(999).unwrap().is_none());
    }

    #[test]
    fn get_edits_nonexistent() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/no_edits.jpg"))
            .unwrap()
            .unwrap();
        assert!(catalog.get_edits(id).unwrap().is_none());
    }

    #[test]
    fn update_thumbnail_path() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/thumb.jpg"))
            .unwrap()
            .unwrap();
        catalog.update_thumbnail(id, "/cache/abc.jpg").unwrap();
        let photo = catalog.get_photo(id).unwrap().unwrap();
        assert_eq!(photo.thumbnail_path.as_deref(), Some("/cache/abc.jpg"));
    }

    #[test]
    fn save_edits_overwrites_previous() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/overwrite.jpg"))
            .unwrap()
            .unwrap();

        let params1 = crema_core::image_buf::EditParams {
            exposure: 1.0,
            ..Default::default()
        };
        catalog.save_edits(id, &params1).unwrap();

        let params2 = crema_core::image_buf::EditParams {
            exposure: 2.0,
            ..Default::default()
        };
        catalog.save_edits(id, &params2).unwrap();

        let edit = catalog.get_edits(id).unwrap().unwrap();
        assert!((edit.exposure - 2.0).abs() < 1e-6);
    }

    #[test]
    fn multiple_photos_ordering() {
        let catalog = Catalog::open_in_memory().unwrap();

        let mut p1 = minimal_photo("/first.jpg");
        p1.date_taken = Some("2024-01-01T00:00:00".to_string());

        let mut p2 = minimal_photo("/second.jpg");
        p2.date_taken = Some("2024-06-01T00:00:00".to_string());

        catalog.insert_photo(&p1).unwrap().unwrap();
        catalog.insert_photo(&p2).unwrap().unwrap();

        let photos = catalog.list_photos().unwrap();
        assert_eq!(photos.len(), 2);
        // Ordered by date_taken DESC
        assert_eq!(photos[0].file_path, "/second.jpg");
        assert_eq!(photos[1].file_path, "/first.jpg");
    }

    #[test]
    fn photo_count_empty() {
        let catalog = Catalog::open_in_memory().unwrap();
        assert_eq!(catalog.photo_count().unwrap(), 0);
    }

    #[test]
    fn edit_record_to_edit_params() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/convert.jpg"))
            .unwrap()
            .unwrap();

        let params = crema_core::image_buf::EditParams {
            exposure: -2.0,
            wb_temp: 8000.0,
            wb_tint: 15.0,
            contrast: 30.0,
            highlights: -50.0,
            shadows: 25.0,
            blacks: -15.0,
            vibrance: 20.0,
            saturation: -10.0,
            crop_x: 0.1,
            crop_y: 0.2,
            crop_w: 0.5,
            crop_h: 0.6,
        };
        catalog.save_edits(id, &params).unwrap();

        let edit = catalog.get_edits(id).unwrap().unwrap();
        let converted = edit.to_edit_params();
        assert!((converted.exposure - (-2.0)).abs() < 1e-6);
        assert!((converted.wb_temp - 8000.0).abs() < 1e-6);
        assert!((converted.contrast - 30.0).abs() < 1e-6);
        assert!((converted.highlights - (-50.0)).abs() < 1e-6);
        assert!((converted.shadows - 25.0).abs() < 1e-6);
        assert!((converted.blacks - (-15.0)).abs() < 1e-6);
        assert!((converted.vibrance - 20.0).abs() < 1e-6);
        assert!((converted.saturation - (-10.0)).abs() < 1e-6);
        assert!((converted.crop_x - 0.1).abs() < 1e-6);
        assert!((converted.crop_h - 0.6).abs() < 1e-6);
    }

    #[test]
    fn roundtrip_new_fields() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/new_fields.jpg"))
            .unwrap()
            .unwrap();

        let params = crema_core::image_buf::EditParams {
            contrast: 25.0,
            highlights: -30.0,
            shadows: 40.0,
            blacks: -10.0,
            vibrance: 15.0,
            saturation: -20.0,
            ..Default::default()
        };
        catalog.save_edits(id, &params).unwrap();

        let edit = catalog.get_edits(id).unwrap().unwrap();
        assert!((edit.contrast - 25.0).abs() < 1e-6);
        assert!((edit.highlights - (-30.0)).abs() < 1e-6);
        assert!((edit.shadows - 40.0).abs() < 1e-6);
        assert!((edit.blacks - (-10.0)).abs() < 1e-6);
        assert!((edit.vibrance - 15.0).abs() < 1e-6);
        assert!((edit.saturation - (-20.0)).abs() < 1e-6);
    }

    #[test]
    fn idempotent_migration() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_catalog.db");
        let path_str = db_path.to_str().unwrap();

        let _catalog1 = Catalog::open(path_str).unwrap();
        drop(_catalog1);
        let _catalog2 = Catalog::open(path_str).unwrap();
    }

    #[test]
    fn old_edits_get_defaults() {
        let catalog = Catalog::open_in_memory().unwrap();
        let id = catalog
            .insert_photo(&minimal_photo("/old_style.jpg"))
            .unwrap()
            .unwrap();

        let params = crema_core::image_buf::EditParams {
            exposure: 1.0,
            wb_temp: 5500.0,
            wb_tint: 0.0,
            ..Default::default()
        };
        catalog.save_edits(id, &params).unwrap();

        let edit = catalog.get_edits(id).unwrap().unwrap();
        assert!((edit.contrast - 0.0).abs() < 1e-6);
        assert!((edit.highlights - 0.0).abs() < 1e-6);
        assert!((edit.shadows - 0.0).abs() < 1e-6);
        assert!((edit.blacks - 0.0).abs() < 1e-6);
        assert!((edit.vibrance - 0.0).abs() < 1e-6);
        assert!((edit.saturation - 0.0).abs() < 1e-6);
    }
}
