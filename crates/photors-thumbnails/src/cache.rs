use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::debug;

/// Disk-backed thumbnail cache keyed by blake3 content hash.
pub struct ThumbnailCache {
    cache_dir: PathBuf,
}

impl ThumbnailCache {
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create cache dir: {}", cache_dir.display()))?;
        Ok(Self { cache_dir })
    }

    /// Get the path where a thumbnail for this content hash would be stored.
    pub fn thumbnail_path(&self, content_hash: &str) -> PathBuf {
        // Use first 2 chars as subdirectory to avoid too many files in one dir
        let subdir = &content_hash[..2.min(content_hash.len())];
        self.cache_dir
            .join(subdir)
            .join(format!("{content_hash}.jpg"))
    }

    /// Check if a thumbnail already exists in the cache.
    pub fn has_thumbnail(&self, content_hash: &str) -> bool {
        self.thumbnail_path(content_hash).exists()
    }

    /// Store thumbnail bytes in the cache, returns the path.
    pub fn store(&self, content_hash: &str, data: &[u8]) -> Result<PathBuf> {
        let path = self.thumbnail_path(content_hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, data).with_context(|| format!("write thumbnail: {}", path.display()))?;
        debug!(?path, "cached thumbnail");
        Ok(path)
    }

    /// Read cached thumbnail bytes, if present.
    pub fn load(&self, content_hash: &str) -> Option<Vec<u8>> {
        let path = self.thumbnail_path(content_hash);
        fs::read(&path).ok()
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn store_and_load_thumbnail() {
        let dir = env::temp_dir().join("photors_cache_test");
        let _ = fs::remove_dir_all(&dir);
        let cache = ThumbnailCache::new(dir.clone()).unwrap();

        let hash = "abcdef1234567890";
        assert!(!cache.has_thumbnail(hash));

        let data = b"fake jpeg data";
        let path = cache.store(hash, data).unwrap();
        assert!(path.exists());
        assert!(cache.has_thumbnail(hash));

        let loaded = cache.load(hash).unwrap();
        assert_eq!(loaded, data);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = env::temp_dir().join("photors_cache_test_miss");
        let _ = fs::remove_dir_all(&dir);
        let cache = ThumbnailCache::new(dir.clone()).unwrap();

        assert!(cache.load("nonexistent").is_none());
        assert!(!cache.has_thumbnail("nonexistent"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn subdirectory_bucketing() {
        let dir = env::temp_dir().join("photors_cache_test_bucket");
        let _ = fs::remove_dir_all(&dir);
        let cache = ThumbnailCache::new(dir.clone()).unwrap();

        let path = cache.thumbnail_path("ff1234");
        assert!(path.to_string_lossy().contains("/ff/"));
        assert!(path.to_string_lossy().ends_with("ff1234.jpg"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overwrite_existing_thumbnail() {
        let dir = env::temp_dir().join("photors_cache_test_overwrite");
        let _ = fs::remove_dir_all(&dir);
        let cache = ThumbnailCache::new(dir.clone()).unwrap();

        let hash = "overwrite_test";
        cache.store(hash, b"version1").unwrap();
        cache.store(hash, b"version2").unwrap();

        let loaded = cache.load(hash).unwrap();
        assert_eq!(loaded, b"version2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_dir_accessor() {
        let dir = env::temp_dir().join("photors_cache_test_accessor");
        let _ = fs::remove_dir_all(&dir);
        let cache = ThumbnailCache::new(dir.clone()).unwrap();
        assert_eq!(cache.cache_dir(), dir.as_path());
        let _ = fs::remove_dir_all(&dir);
    }
}
