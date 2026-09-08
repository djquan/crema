use std::{fs::File, io, path::PathBuf};

#[cfg(target_os = "windows")]
use std::{ffi::c_void, mem::MaybeUninit, os::windows::io::AsRawHandle};

#[cfg(target_os = "windows")]
#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct ByHandleFileInformation {
    attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(target_os = "windows")]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        handle: *mut c_void,
        information: *mut ByHandleFileInformation,
    ) -> i32;
}

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
        #[cfg(target_os = "windows")]
        {
            let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
            // SAFETY: the handle is borrowed from a live File and Windows initializes the
            // complete BY_HANDLE_FILE_INFORMATION value when the call succeeds.
            let succeeded = unsafe {
                GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr())
            };
            if succeeded == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: a nonzero return guarantees the output structure was initialized.
            let information = unsafe { information.assume_init() };
            let mut bytes = Vec::with_capacity(28);
            for value in [
                information.volume_serial_number,
                information.file_index_high,
                information.file_index_low,
                information.file_size_high,
                information.file_size_low,
                information.last_write_time.high,
                information.last_write_time.low,
            ] {
                bytes.extend(value.to_le_bytes());
            }
            Ok(Self(bytes))
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            let _ = metadata;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "persistent source identity unavailable on this platform",
            ))
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::SourceStamp;
    use std::{fs, fs::File};

    #[test]
    fn source_stamp_is_stable_for_one_file_and_changes_after_replacement() {
        let directory =
            std::env::temp_dir().join(format!("crema-windows-source-stamp-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("source.jpg");
        fs::write(&path, b"first").unwrap();
        let first = SourceStamp::read(&File::open(&path).unwrap()).unwrap();
        assert_eq!(
            first,
            SourceStamp::read(&File::open(&path).unwrap()).unwrap()
        );

        fs::remove_file(&path).unwrap();
        fs::write(&path, b"second version").unwrap();
        let replacement = SourceStamp::read(&File::open(&path).unwrap()).unwrap();
        assert_ne!(first, replacement);
        fs::remove_dir_all(&directory).unwrap();
    }
}
