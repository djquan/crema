use std::error::Error;
use std::fmt;
use std::fs::{self, ReadDir};
use std::io;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ASSET_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AssetId(NonZeroU64);

impl AssetId {
    fn allocate() -> Self {
        let value = NEXT_ASSET_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("process-local asset ID space exhausted");
        Self(NonZeroU64::new(value).expect("asset ID allocator must start at one"))
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug)]
pub struct AssetCandidate<K> {
    id: AssetId,
    path: PathBuf,
    kind: K,
}

impl<K> AssetCandidate<K> {
    pub fn id(&self) -> AssetId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> &K {
        &self.kind
    }
}

#[derive(Debug)]
pub enum ScanEvent<K> {
    Candidate(AssetCandidate<K>),
    Failure(EntryFailure),
}

#[derive(Debug)]
pub enum EntryFailure {
    ReadDirectoryEntry { root: PathBuf, source: io::Error },
    InspectCandidate { path: PathBuf, source: io::Error },
}

impl fmt::Display for EntryFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadDirectoryEntry { root, source } => write!(
                formatter,
                "cannot read an entry in {}: {source}",
                root.display()
            ),
            Self::InspectCandidate { path, source } => write!(
                formatter,
                "cannot inspect candidate {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for EntryFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ReadDirectoryEntry { source, .. } | Self::InspectCandidate { source, .. } => {
                Some(source)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FolderOpenErrorKind {
    NotFound,
    NotDirectory,
    PermissionDenied,
    Io,
}

impl fmt::Display for FolderOpenErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::NotFound => "folder does not exist",
            Self::NotDirectory => "path is not a folder",
            Self::PermissionDenied => "permission denied",
            Self::Io => "filesystem error",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug)]
pub struct FolderOpenError {
    root: PathBuf,
    kind: FolderOpenErrorKind,
    source: io::Error,
}

impl FolderOpenError {
    fn new(root: PathBuf, source: io::Error) -> Self {
        let kind = match source.kind() {
            io::ErrorKind::NotFound => FolderOpenErrorKind::NotFound,
            io::ErrorKind::NotADirectory => FolderOpenErrorKind::NotDirectory,
            io::ErrorKind::PermissionDenied => FolderOpenErrorKind::PermissionDenied,
            _ => FolderOpenErrorKind::Io,
        };
        Self { root, kind, source }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind(&self) -> FolderOpenErrorKind {
        self.kind
    }
}

impl fmt::Display for FolderOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot open folder {}: {} ({})",
            self.root.display(),
            self.kind,
            self.source
        )
    }
}

impl Error for FolderOpenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub struct FolderScan<K> {
    root: PathBuf,
    entries: ReadDir,
    classify: fn(&Path) -> Option<K>,
}

pub fn scan_folder<K>(
    root: impl AsRef<Path>,
    classify: fn(&Path) -> Option<K>,
) -> Result<FolderScan<K>, FolderOpenError> {
    let root = root.as_ref().to_owned();
    let entries =
        fs::read_dir(&root).map_err(|source| FolderOpenError::new(root.clone(), source))?;
    Ok(FolderScan {
        root,
        entries,
        classify,
    })
}

impl<K> Iterator for FolderScan<K> {
    type Item = ScanEvent<K>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = match self.entries.next()? {
                Ok(entry) => entry,
                Err(source) => {
                    return Some(ScanEvent::Failure(EntryFailure::ReadDirectoryEntry {
                        root: self.root.clone(),
                        source,
                    }));
                }
            };
            let path = entry.path();
            let Some(kind) = (self.classify)(&path) else {
                continue;
            };
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(source) => {
                    return Some(ScanEvent::Failure(EntryFailure::InspectCandidate {
                        path,
                        source,
                    }));
                }
            };
            if !metadata.is_file() {
                continue;
            }
            return Some(ScanEvent::Candidate(AssetCandidate {
                id: AssetId::allocate(),
                path,
                kind,
            }));
        }
    }
}
