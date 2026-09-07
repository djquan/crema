use std::{fs::File, io, path::PathBuf};

pub fn cache_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
    root.filter(|path| path.is_absolute())
        .map(|path| path.join("crema/thumbnails-v1"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceStamp(pub Vec<u8>);
impl SourceStamp {
    pub fn read(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("source is not a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let mut bytes = Vec::new();
            for value in [
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.mtime() as u64,
                metadata.mtime_nsec() as u64,
                metadata.ctime() as u64,
                metadata.ctime_nsec() as u64,
            ] {
                bytes.extend(value.to_le_bytes());
            }
            Ok(Self(bytes))
        }
        #[cfg(not(unix))]
        {
            // Without an opened-file identity, persistent reuse is disabled.
            let _ = metadata;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "persistent source identity unavailable on this platform",
            ))
        }
    }
}
