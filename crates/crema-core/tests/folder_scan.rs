use crema_core::{FolderOpenErrorKind, ScanEvent, scan_folder};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);
static LAZY_CLASSIFY_CALLS: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("crema-core-{label}-{}-{sequence}", process::id()));
        fs::create_dir(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove test directory");
    }
}

fn classify_fixture(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("raf")
        || extension.eq_ignore_ascii_case("orf")
        || extension.eq_ignore_ascii_case("jpg")
    {
        Some("image")
    } else {
        None
    }
}

fn count_classification(_path: &Path) -> Option<()> {
    LAZY_CLASSIFY_CALLS.fetch_add(1, Ordering::Relaxed);
    Some(())
}

#[test]
fn defers_classification_until_iteration() {
    let directory = TestDirectory::new("lazy");
    fs::write(directory.path().join("candidate.any"), b"candidate").expect("write fixture");
    LAZY_CLASSIFY_CALLS.store(0, Ordering::Relaxed);

    let scan = scan_folder(directory.path(), count_classification).expect("open fixture directory");

    assert_eq!(LAZY_CLASSIFY_CALLS.load(Ordering::Relaxed), 0);
    assert_eq!(scan.count(), 1);
    assert_eq!(LAZY_CLASSIFY_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn scans_recognized_files_without_writing_or_sorting() {
    let directory = TestDirectory::new("mixed");
    let fixture = [
        ("first.RAF", b"raf bytes".as_slice()),
        ("second.orf", b"orf bytes".as_slice()),
        ("preview.JPG", b"jpeg bytes".as_slice()),
        ("notes.txt", b"notes".as_slice()),
    ];

    for (name, bytes) in fixture {
        fs::write(directory.path().join(name), bytes).expect("write fixture");
    }
    fs::create_dir(directory.path().join("album.jpg")).expect("create ignored directory");

    let before = fixture.map(|(name, _)| {
        fs::read(directory.path().join(name)).expect("snapshot fixture before scan")
    });
    let events: Vec<_> = scan_folder(directory.path(), classify_fixture)
        .expect("open fixture directory")
        .collect();
    let after = fixture.map(|(name, _)| {
        fs::read(directory.path().join(name)).expect("snapshot fixture after scan")
    });

    assert_eq!(before, after, "folder scanning changed fixture bytes");

    let mut candidates = events
        .into_iter()
        .map(|event| match event {
            ScanEvent::Candidate(candidate) => candidate,
            ScanEvent::Failure(failure) => panic!("unexpected scan failure: {failure}"),
        })
        .collect::<Vec<_>>();
    let names = candidates
        .iter()
        .map(|candidate| {
            candidate
                .path()
                .file_name()
                .expect("candidate filename")
                .to_os_string()
        })
        .collect::<HashSet<_>>();
    let expected = ["first.RAF", "second.orf", "preview.JPG"]
        .into_iter()
        .map(OsString::from)
        .collect::<HashSet<_>>();

    assert_eq!(names, expected);
    assert!(
        candidates
            .iter()
            .all(|candidate| *candidate.kind() == "image")
    );

    let distinct_ids = candidates
        .iter()
        .map(|candidate| candidate.id())
        .collect::<HashSet<_>>();
    assert_eq!(distinct_ids.len(), candidates.len());

    let ids_by_path = candidates
        .iter()
        .map(|candidate| (candidate.path().to_owned(), candidate.id()))
        .collect::<HashMap<_, _>>();
    candidates.reverse();
    for candidate in candidates {
        assert_eq!(ids_by_path[&candidate.path().to_owned()], candidate.id());
    }
}

#[test]
fn distinguishes_missing_and_non_directory_roots() {
    let directory = TestDirectory::new("root-errors");
    let missing = directory.path().join("missing");
    let missing_error = scan_folder(missing, classify_fixture)
        .err()
        .expect("missing root must fail");
    assert_eq!(missing_error.kind(), FolderOpenErrorKind::NotFound);

    let file = directory.path().join("root.jpg");
    fs::write(&file, b"not a directory").expect("write file root");
    let file_error = scan_folder(&file, classify_fixture)
        .err()
        .expect("file root must fail");
    assert_eq!(file_error.root(), file);
    assert_eq!(file_error.kind(), FolderOpenErrorKind::NotDirectory);
}

#[cfg(unix)]
#[test]
fn continues_after_a_dangling_recognized_symlink() {
    use crema_core::EntryFailure;
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("dangling-link");
    let broken = directory.path().join("broken.jpg");
    let available = directory.path().join("available.RAF");
    symlink(directory.path().join("missing-target"), &broken).expect("create dangling symlink");
    fs::write(&available, b"available").expect("write available fixture");

    let events = scan_folder(directory.path(), classify_fixture)
        .expect("open fixture directory")
        .collect::<Vec<_>>();

    assert!(events.iter().any(|event| {
        matches!(
            event,
            ScanEvent::Failure(EntryFailure::InspectCandidate { path, .. }) if path == &broken
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(event, ScanEvent::Candidate(candidate) if candidate.path() == available)
    }));
}

#[cfg(unix)]
#[test]
fn follows_a_recognized_symlink_to_a_regular_file() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("file-link");
    let target = directory.path().join("original.bin");
    let link = directory.path().join("linked.jpg");
    fs::write(&target, b"linked fixture").expect("write symlink target");
    symlink(&target, &link).expect("create file symlink");

    let events = scan_folder(directory.path(), classify_fixture)
        .expect("open fixture directory")
        .collect::<Vec<_>>();

    assert!(events.iter().any(|event| {
        matches!(event, ScanEvent::Candidate(candidate) if candidate.path() == link)
    }));
}

#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn preserves_non_utf8_filenames_with_ascii_extensions() {
    use std::os::unix::ffi::OsStringExt;

    let directory = TestDirectory::new("non-utf8");
    let filename = OsString::from_vec(b"photo-\xff.RAF".to_vec());
    let path = directory.path().join(&filename);
    fs::write(&path, b"raw bytes").expect("write non-UTF-8 fixture");

    let events = scan_folder(directory.path(), classify_fixture)
        .expect("open fixture directory")
        .collect::<Vec<_>>();

    assert!(events.iter().any(|event| {
        matches!(event, ScanEvent::Candidate(candidate) if candidate.path() == path)
    }));
}
