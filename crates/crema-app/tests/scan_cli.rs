use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("crema-app-{label}-{}-{sequence}", process::id()));
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

fn run(arguments: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crema-scan"))
        .args(arguments)
        .output()
        .expect("run crema-scan")
}

#[test]
fn prints_recognized_candidates_from_a_real_folder_without_writing() {
    let directory = TestDirectory::new("mixed");
    let fixture = [
        ("first.RAF", b"raf bytes".as_slice()),
        ("second.orf", b"orf bytes".as_slice()),
        ("preview.JpG", b"jpeg bytes".as_slice()),
        ("notes.txt", b"notes".as_slice()),
    ];
    for (name, bytes) in fixture {
        fs::write(directory.path().join(name), bytes).expect("write fixture");
    }
    let before = fixture.map(|(name, _)| fs::read(directory.path().join(name)).unwrap());

    let output = run(&[directory.path()]);

    let after = fixture.map(|(name, _)| fs::read(directory.path().join(name)).unwrap());
    assert_eq!(before, after, "the binary changed fixture bytes");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let mut formats_by_name = HashMap::new();
    let mut ids = HashSet::new();
    for line in stdout.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        let [id, format, path] = fields.as_slice() else {
            panic!("unexpected output line: {line}");
        };
        let id = id.parse::<u64>().expect("numeric asset ID");
        assert_ne!(id, 0);
        assert!(ids.insert(id), "duplicate asset ID: {id}");
        let name = Path::new(path)
            .file_name()
            .expect("output filename")
            .to_str()
            .expect("UTF-8 output filename");
        formats_by_name.insert(name.to_owned(), (*format).to_owned());
    }

    assert_eq!(
        formats_by_name,
        HashMap::from([
            ("first.RAF".to_owned(), "RAF".to_owned()),
            ("second.orf".to_owned(), "ORF".to_owned()),
            ("preview.JpG".to_owned(), "JPEG".to_owned()),
        ])
    );
}

#[test]
fn rejects_missing_and_surplus_arguments() {
    let missing = run(&[]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("usage: crema-scan <folder>"));

    let first = Path::new("first");
    let second = Path::new("second");
    let surplus = run(&[first, second]);
    assert!(!surplus.status.success());
    assert!(String::from_utf8_lossy(&surplus.stderr).contains("usage: crema-scan <folder>"));
}

#[test]
fn exits_with_an_error_when_the_root_cannot_open() {
    let directory = TestDirectory::new("missing-root");
    let missing = directory.path().join("missing");

    let output = run(&[&missing]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("cannot open folder"));
    assert!(stderr.contains("missing"));
}

#[cfg(unix)]
#[test]
fn reports_an_entry_failure_but_prints_other_candidates() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("entry-failure");
    let broken = directory.path().join("broken.jpg");
    symlink(directory.path().join("missing-target"), &broken).expect("create dangling symlink");
    fs::write(directory.path().join("available.RAF"), b"available").expect("write fixture");

    let output = run(&[directory.path()]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("available.RAF"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("broken.jpg"));
}
