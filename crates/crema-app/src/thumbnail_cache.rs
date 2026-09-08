use crema_image::{
    DecodeResult, Fact, Icc, Orientation, PreviewPixels, PreviewSize, Provenance, SourceMetadata,
    UnknownReason,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

pub const DEFAULT_BUDGET: u64 = 512 * 1024 * 1024;
const MAX_RECORD: u64 = 320 * 320 * 4 + 64 * 1024;
const MAGIC: &[u8; 8] = b"CRMATHM2";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[derive(Clone)]
pub struct CacheConfig {
    pub root: Option<PathBuf>,
    pub budget: u64,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            root: crate::platform::cache_root(),
            budget: DEFAULT_BUDGET,
        }
    }
}
impl CacheConfig {
    pub fn outside_source(mut self, source_root: &std::path::Path) -> Self {
        self.root = self.root.as_deref().and_then(|root| {
            let source = fs::canonicalize(source_root).ok()?;
            let cache = resolved_cache_root(root)?;
            (!cache.starts_with(source)).then_some(cache)
        });
        self
    }
}

fn resolved_cache_root(path: &std::path::Path) -> Option<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return None;
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for component in suffix.into_iter().rev() {
                    resolved.push(component);
                }
                return Some(resolved);
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && fs::symlink_metadata(ancestor).is_err() =>
            {
                suffix.push(ancestor.file_name()?.to_owned());
                ancestor = ancestor.parent()?;
            }
            Err(_) => return None,
        }
    }
}

pub struct ThumbnailCache {
    root: Option<PathBuf>,
    budget: u64,
}
fn remove_if_same_file(path: &std::path::Path, opened: &fs::Metadata) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(current) = fs::symlink_metadata(path)
            && current.is_file()
            && current.dev() == opened.dev()
            && current.ino() == opened.ino()
        {
            let _ = fs::remove_file(path);
        }
    }
    #[cfg(not(unix))]
    let _ = (path, opened);
}
impl ThumbnailCache {
    pub fn new(root: Option<PathBuf>, budget: u64) -> Self {
        Self {
            root: root.filter(|path| path.is_absolute()),
            budget,
        }
    }
    fn path(&self, key: &[u8]) -> Option<PathBuf> {
        if self.budget == 0 {
            return None;
        }
        Some(
            self.root
                .as_ref()?
                .join(format!("{:016x}.thumb", checksum(key))),
        )
    }
    pub fn load(&self, key: &[u8]) -> Option<DecodeResult> {
        let path = self.path(key)?;
        if !fs::symlink_metadata(&path).ok()?.file_type().is_file() {
            return None;
        }
        let mut file = File::open(&path).ok()?;
        let opened = file.metadata().ok()?;
        let len = opened.len();
        if !(24..=MAX_RECORD).contains(&len) {
            drop(file);
            remove_if_same_file(&path, &opened);
            return None;
        }
        let mut bytes = Vec::with_capacity(len as usize);
        let read = (&mut file).take(MAX_RECORD + 1).read_to_end(&mut bytes);
        drop(file);
        read.ok()?;
        if bytes.len() as u64 != len {
            remove_if_same_file(&path, &opened);
            return None;
        }
        let result = decode_record(&bytes, key);
        if result.is_none() {
            remove_if_same_file(&path, &opened);
        }
        result
    }
    pub fn store(&self, key: &[u8], result: &DecodeResult) -> bool {
        let Some(path) = self.path(key) else {
            return false;
        };
        let Some(bytes) = encode_record(key, result) else {
            return false;
        };
        if bytes.len() as u64 > self.budget {
            return false;
        }
        if self.load(key).is_some() {
            return true;
        }
        let root = path.parent().expect("cache entry parent");
        if fs::create_dir_all(root).is_err() {
            return false;
        }
        let temporary = root.join(format!(
            ".crema-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let publish = || -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            match fs::rename(&temporary, &path) {
                Ok(()) => Ok(()),
                Err(error) => {
                    if self.load(key).is_some() {
                        return Ok(());
                    }
                    fs::rename(&temporary, &path).or_else(|_| {
                        if self.load(key).is_some() {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    })
                }
            }
        };
        let success = publish().is_ok();
        let _ = fs::remove_file(temporary);
        success
    }
    pub fn maintain(&self, still_current: impl Fn() -> bool) {
        let Some(root) = &self.root else {
            return;
        };
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        let mut records = Vec::new();
        let mut actual_bytes = 0u64;
        for entry in entries.flatten() {
            if !still_current() {
                return;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if name.starts_with(".crema-") && name.ends_with(".tmp") {
                if modified.elapsed().unwrap_or_default() > Duration::from_secs(3600) {
                    let _ = fs::remove_file(entry.path());
                }
            } else if owned_entry(name) {
                actual_bytes = actual_bytes.saturating_add(metadata.len());
                records.push((modified, entry.path(), metadata.len()));
            }
        }
        records.sort_unstable_by_key(|(modified, _, _)| *modified);
        for (_, path, len) in records {
            if !still_current() {
                return;
            }
            if actual_bytes <= self.budget {
                break;
            }
            if fs::remove_file(path).is_ok() {
                actual_bytes = actual_bytes.saturating_sub(len);
            }
        }
    }
}

fn owned_entry(name: &str) -> bool {
    name.len() == 22
        && name.ends_with(".thumb")
        && name[..16].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn put_string(bytes: &mut Vec<u8>, value: &str) -> Option<()> {
    if value.len() > 8192 {
        return None;
    }
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value.as_bytes());
    Some(())
}
fn unknown(reason: UnknownReason) -> u8 {
    match reason {
        UnknownReason::DecoderDoesNotExpose => 1,
        UnknownReason::MetadataMissing => 2,
        UnknownReason::MetadataInvalid => 3,
        UnknownReason::NotMeasured => 4,
    }
}
fn reason(value: u8) -> Option<UnknownReason> {
    Some(match value {
        1 => UnknownReason::DecoderDoesNotExpose,
        2 => UnknownReason::MetadataMissing,
        3 => UnknownReason::MetadataInvalid,
        4 => UnknownReason::NotMeasured,
        _ => return None,
    })
}

fn encode_record(key: &[u8], result: &DecodeResult) -> Option<Vec<u8>> {
    if key.len() > 4096 || result.preview.width().max(result.preview.height()) > 320 {
        return None;
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend((key.len() as u32).to_le_bytes());
    bytes.extend(key);
    for value in [result.preview.width(), result.preview.height()]
        .into_iter()
        .chain(result.metadata.dimensions)
        .chain(result.metadata.decoded_dimensions)
    {
        bytes.extend(value.to_le_bytes());
    }
    bytes.push(match result.provenance {
        Provenance::JpegDecode => 1,
        Provenance::RawlerDevelopment => 2,
        Provenance::HeifDecode => 3,
    });
    match result.metadata.source_bits {
        Fact::Known(value) => bytes.extend([0, value]),
        Fact::Unknown(value) => bytes.extend([unknown(value), 0]),
    }
    bytes.push(result.metadata.decoded_bits);
    match result.metadata.orientation {
        Orientation::Exif(value) => bytes.extend([0, value]),
        Orientation::ContainerApplied => bytes.extend([5, 0]),
        Orientation::Unknown(value) => bytes.extend([unknown(value), 0]),
    }
    match result.metadata.icc {
        Fact::Known(Icc::Absent) => bytes.extend([0, 0]),
        Fact::Known(Icc::PresentNotApplied) => bytes.extend([0, 1]),
        Fact::Known(Icc::AppliedToSrgb) => bytes.extend([0, 2]),
        Fact::Unknown(value) => bytes.extend([unknown(value), 0]),
    }
    bytes.push(u8::from(result.metadata.nclx.is_some()));
    if let Some(nclx) = result.metadata.nclx {
        for value in nclx {
            bytes.extend(value.to_le_bytes());
        }
    }
    for value in [
        &result.metadata.camera_make,
        &result.metadata.camera_model,
        &result.metadata.limitations,
    ] {
        put_string(&mut bytes, value)?;
    }
    bytes.extend(result.preview.rgba8());
    bytes.extend(checksum(&bytes).to_le_bytes());
    (bytes.len() as u64 <= MAX_RECORD).then_some(bytes)
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (value, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(value)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        if len > 8192 {
            return None;
        }
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}
fn decode_record(bytes: &[u8], key: &[u8]) -> Option<DecodeResult> {
    let (body, hash) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if checksum(body) != u64::from_le_bytes(hash.try_into().ok()?) {
        return None;
    }
    let mut reader = Reader(body);
    if reader.take(8)? != MAGIC || reader.u32()? != 2 {
        return None;
    }
    let key_len = reader.u32()? as usize;
    if key_len > 4096 || reader.take(key_len)? != key {
        return None;
    }
    let width = reader.u32()?;
    let height = reader.u32()?;
    if width == 0 || height == 0 || width.max(height) > 320 {
        return None;
    }
    let dimensions = [reader.u32()?, reader.u32()?];
    let decoded_dimensions = [reader.u32()?, reader.u32()?];
    if dimensions.contains(&0)
        || decoded_dimensions.contains(&0)
        || dimensions.into_iter().any(|value| value > 100_000_000)
        || decoded_dimensions
            .into_iter()
            .any(|value| value > 100_000_000)
    {
        return None;
    }
    let provenance = match reader.byte()? {
        1 => Provenance::JpegDecode,
        2 => Provenance::RawlerDevelopment,
        3 => Provenance::HeifDecode,
        _ => return None,
    };
    let tag = reader.byte()?;
    let value = reader.byte()?;
    let source_bits = if tag == 0 {
        if !(1..=64).contains(&value) {
            return None;
        }
        Fact::Known(value)
    } else {
        Fact::Unknown(reason(tag)?)
    };
    let decoded_bits = reader.byte()?;
    if !(1..=64).contains(&decoded_bits) {
        return None;
    }
    let tag = reader.byte()?;
    let value = reader.byte()?;
    let orientation = match tag {
        0 if (1..=8).contains(&value) => Orientation::Exif(value),
        5 => Orientation::ContainerApplied,
        _ => Orientation::Unknown(reason(tag)?),
    };
    let tag = reader.byte()?;
    let value = reader.byte()?;
    let icc = if tag == 0 {
        Fact::Known(match value {
            0 => Icc::Absent,
            1 => Icc::PresentNotApplied,
            2 => Icc::AppliedToSrgb,
            _ => return None,
        })
    } else {
        Fact::Unknown(reason(tag)?)
    };
    let nclx = match reader.byte()? {
        0 => None,
        1 => {
            let mut values = [0; 4];
            for value in &mut values {
                *value = u16::from_le_bytes(reader.take(2)?.try_into().ok()?);
            }
            Some(values)
        }
        _ => return None,
    };
    let metadata = SourceMetadata {
        dimensions,
        decoded_dimensions,
        source_bits,
        decoded_bits,
        orientation,
        icc,
        nclx,
        camera_make: reader.string()?,
        camera_model: reader.string()?,
        limitations: reader.string()?,
    };
    let pixels = reader.take(width as usize * height as usize * 4)?.to_vec();
    if !reader.0.is_empty() {
        return None;
    }
    Some(DecodeResult {
        preview: PreviewPixels::new(width, height, pixels, PreviewSize::new(320).ok()?).ok()?,
        metadata,
        provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crema_image::{CancelToken, CandidateFormat, DecodeLimits, DecodeOutcome, Decoder};
    fn fixture() -> (PathBuf, DecodeResult) {
        let root = std::env::temp_dir().join(format!(
            "crema-disk-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("source.jpg");
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode(
                &vec![90; 400 * 300 * 3],
                400,
                300,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        fs::write(&path, bytes).unwrap();
        let decoder = Decoder::new(std::env::current_exe().unwrap(), DecodeLimits::default());
        let DecodeOutcome::Decoded(result) = decoder.decode(
            &path,
            CandidateFormat::Raster(crema_image::RasterFormat::Jpeg),
            PreviewSize::new(320).unwrap(),
            &CancelToken::new(),
        ) else {
            panic!("JPEG");
        };
        (root, result)
    }
    #[test]
    fn immutable_record_survives_cache_restart_with_identical_pixels() {
        let (root, result) = fixture();
        let cache_root = root.join("cache");
        let cache = ThumbnailCache::new(Some(cache_root.clone()), 1024 * 1024);
        assert!(
            cache.store(b"source-key", &result),
            "valid thumbnail must be persisted"
        );
        drop(cache);
        let cache = ThumbnailCache::new(Some(cache_root), 1024 * 1024);
        let loaded = cache.load(b"source-key").unwrap();
        assert_eq!(loaded.preview.rgba8(), result.preview.rgba8());
        assert_eq!(loaded.metadata, result.metadata);
        assert!(cache.load(b"other-source").is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corruption_truncation_oversize_and_obsolete_schema_are_misses() {
        let (root, result) = fixture();
        let cache = ThumbnailCache::new(Some(root.join("cache")), DEFAULT_BUDGET);
        assert!(cache.store(b"key", &result));
        let path = cache.path(b"key").unwrap();
        let original = fs::read(&path).unwrap();
        for len in [0, 8, 16, original.len() - 1] {
            fs::write(&path, &original[..len]).unwrap();
            assert!(cache.load(b"key").is_none());
            assert!(!path.exists(), "invalid owned entries must be removed");
            assert!(cache.store(b"key", &result));
            assert_eq!(
                cache.load(b"key").unwrap().preview.rgba8(),
                result.preview.rgba8()
            );
        }
        let mut corrupt = original.clone();
        corrupt[30] ^= 1;
        fs::write(&path, corrupt).unwrap();
        assert!(cache.load(b"key").is_none());
        let mut obsolete = original.clone();
        obsolete[8] = 1;
        let end = obsolete.len() - 8;
        let hash = checksum(&obsolete[..end]);
        obsolete[end..].copy_from_slice(&hash.to_le_bytes());
        fs::write(&path, obsolete).unwrap();
        assert!(cache.load(b"key").is_none());
        assert!(!path.exists());
        fs::write(&path, b"corrupt existing destination").unwrap();
        assert!(cache.store(b"key", &result));
        assert_eq!(
            cache.load(b"key").unwrap().preview.rgba8(),
            result.preview.rgba8()
        );
        let file = File::create(&path).unwrap();
        file.set_len(MAX_RECORD + 1).unwrap();
        assert!(cache.load(b"key").is_none());
        fs::write(&path, original).unwrap();
        assert!(cache.load(b"key").is_some());
        assert!(decode_record(&fs::read(path).unwrap(), b"collision").is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn applied_icc_state_survives_cache_restart() {
        let (root, mut result) = fixture();
        result.metadata.icc = Fact::Known(Icc::AppliedToSrgb);
        let cache = ThumbnailCache::new(Some(root.join("cache")), DEFAULT_BUDGET);
        assert!(cache.store(b"profiled", &result));
        assert_eq!(
            cache.load(b"profiled").unwrap().metadata.icc,
            Fact::Known(Icc::AppliedToSrgb)
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn out_of_range_records_can_be_republished() {
        let (root, result) = fixture();
        let cache = ThumbnailCache::new(Some(root.join("cache")), DEFAULT_BUDGET);
        assert!(cache.store(b"key", &result));
        let path = cache.path(b"key").unwrap();
        for len in [0, 8, 23, MAX_RECORD + 1] {
            let file = File::create(&path).unwrap();
            file.set_len(len).unwrap();
            drop(file);
            assert!(
                cache.store(b"key", &result),
                "writer must recover an out-of-range entry"
            );
            assert_eq!(
                cache.load(b"key").unwrap().preview.rgba8(),
                result.preview.rgba8()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn corrupt_cleanup_preserves_a_concurrently_published_replacement() {
        let (root, result) = fixture();
        let cache = ThumbnailCache::new(Some(root.join("cache")), DEFAULT_BUDGET);
        assert!(cache.store(b"key", &result));
        let path = cache.path(b"key").unwrap();
        fs::write(&path, b"corrupt").unwrap();
        let opened = File::open(&path).unwrap();
        let stale_identity = opened.metadata().unwrap();
        drop(opened);
        let replacement = path.with_extension("replacement");
        fs::write(&replacement, encode_record(b"key", &result).unwrap()).unwrap();
        fs::rename(&replacement, &path).unwrap();
        remove_if_same_file(&path, &stale_identity);
        assert!(
            path.exists(),
            "stale corrupt-reader cleanup must preserve the replacement"
        );
        assert_eq!(
            cache.load(b"key").unwrap().preview.rgba8(),
            result.preview.rgba8()
        );
        let opened = File::open(&path).unwrap();
        let current_identity = opened.metadata().unwrap();
        drop(opened);
        remove_if_same_file(&path, &current_identity);
        assert!(
            !path.exists(),
            "same-identity cleanup must still remove the entry"
        );
        assert!(cache.store(b"key", &result));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reconciliation_counts_actual_bytes_and_preserves_foreign_files() {
        let (root, result) = fixture();
        let cache_root = root.join("cache");
        let cache = ThumbnailCache::new(Some(cache_root.clone()), DEFAULT_BUDGET);
        for key in [b"one", b"two", b"tri"] {
            assert!(cache.store(key, &result));
        }
        let record_len = fs::metadata(cache.path(b"one").unwrap()).unwrap().len();
        fs::write(cache_root.join("foreign.txt"), b"keep").unwrap();
        let old = cache_root.join(".crema-dead-1.tmp");
        fs::write(&old, b"partial").unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        let live = cache_root.join(".crema-live-1.tmp");
        fs::write(&live, b"partial").unwrap();
        let bounded = ThumbnailCache::new(Some(cache_root.clone()), record_len);
        bounded.maintain(|| false);
        assert!(old.exists());
        bounded.maintain(|| true);
        let bytes: u64 = fs::read_dir(&cache_root)
            .unwrap()
            .flatten()
            .filter(|entry| owned_entry(&entry.file_name().to_string_lossy()))
            .map(|entry| entry.metadata().unwrap().len())
            .sum();
        assert!(bytes <= record_len);
        assert!(!old.exists());
        assert!(live.exists());
        assert!(cache_root.join("foreign.txt").exists());
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .flatten()
                .all(|entry| entry.file_name() == "source.jpg" || entry.file_name() == "cache")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn atomic_concurrent_publication_and_disabled_unwritable_fallback() {
        let (root, result) = fixture();
        let cache = ThumbnailCache::new(Some(root.join("cache")), DEFAULT_BUDGET);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| assert!(cache.store(b"same", &result)));
            }
        });
        assert_eq!(
            cache.load(b"same").unwrap().preview.rgba8(),
            result.preview.rgba8()
        );
        assert_eq!(fs::read_dir(root.join("cache")).unwrap().count(), 1);
        let published = cache.path(b"same").unwrap();
        File::options()
            .write(true)
            .open(&published)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        assert!(cache.store(b"same", &result));
        assert_eq!(
            fs::metadata(&published).unwrap().modified().unwrap(),
            SystemTime::UNIX_EPOCH,
            "matching published entries must remain immutable"
        );
        assert!(!ThumbnailCache::new(None, DEFAULT_BUDGET).store(b"key", &result));
        assert!(
            !ThumbnailCache::new(Some(root.join("source.jpg")), DEFAULT_BUDGET)
                .store(b"key", &result)
        );
        assert!(!ThumbnailCache::new(Some(root.join("disabled")), 0).store(b"key", &result));
        assert!(!root.join("disabled").exists());
        assert_eq!(checksum(b"hello"), 0xa430d84680aabd0b);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_root_rejects_source_and_nested_symlink_aliases_without_touching_originals() {
        let (root, result) = fixture();
        let source_root = root.join("originals");
        fs::create_dir(&source_root).unwrap();
        let original = source_root.join("0123456789abcdef.thumb");
        fs::write(&original, b"original user bytes").unwrap();
        let unsafe_roots = {
            let roots = vec![
                source_root.clone(),
                source_root.join("new/cache"),
                root.join("missing/../originals/cache"),
            ];
            #[cfg(unix)]
            {
                let alias = root.join("alias");
                std::os::unix::fs::symlink(&source_root, &alias).unwrap();
                let mut roots = roots;
                roots.extend([alias.clone(), alias.join("nested")]);
                roots
            }
            #[cfg(not(unix))]
            roots
        };
        assert!(
            resolved_cache_root(&root.join("originals/../outside-cache")).is_none(),
            "parent-directory components must be rejected even before suffix creation"
        );
        for candidate in unsafe_roots {
            let config = CacheConfig {
                root: Some(candidate),
                budget: 1,
            }
            .outside_source(&source_root);
            assert!(
                config.root.is_none(),
                "cache root inside originals must be rejected"
            );
            let cache = ThumbnailCache::new(config.root, config.budget);
            assert!(!cache.store(b"key", &result));
            cache.maintain(|| true);
            assert_eq!(fs::read(&original).unwrap(), b"original user bytes");
        }
        assert_eq!(fs::read_dir(&source_root).unwrap().count(), 1);
        let config = CacheConfig {
            root: Some(root.join("safe/cache")),
            budget: DEFAULT_BUDGET,
        }
        .outside_source(&source_root);
        assert!(config.root.is_some());
        assert!(ThumbnailCache::new(config.root, config.budget).store(b"safe", &result));
        fs::remove_dir_all(root).unwrap();
    }
}
