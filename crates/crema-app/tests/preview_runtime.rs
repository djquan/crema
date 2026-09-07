use crema_app::{
    jobs::{Event, JobKey, PreviewDemand, PreviewRequest, PreviewRuntime, Purpose},
    metrics::Metrics,
    thumbnail_cache::CacheConfig,
};
use crema_core::{ScanEvent, scan_folder};
use crema_image::{DecodeOutcome, classify_candidate};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

struct Directory(PathBuf);
impl Directory {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crema-runtime-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn jpeg(path: &Path) {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode(&[100; 18], 2, 3, image::ExtendedColorType::Rgb8)
        .unwrap();
    fs::write(path, bytes).unwrap();
}
fn requests(root: &Path) -> Vec<PreviewRequest> {
    let mut requests: Vec<_> = scan_folder(root, classify_candidate)
        .unwrap()
        .filter_map(|event| match event {
            ScanEvent::Candidate(candidate) => Some(PreviewRequest {
                key: JobKey {
                    generation: 1,
                    asset: candidate.id(),
                    purpose: Purpose::Thumbnail,
                },
                path: candidate.path().into(),
                format: *candidate.kind(),
                needed: true,
            }),
            _ => None,
        })
        .collect();
    requests.sort_by_key(|request| request.path.clone());
    requests
}
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "real runtime deadline");
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn selected_preempts_real_child_reaps_it_and_restarts_continuous_thumbnail() {
    let root = Directory::new("preemption");
    fs::write(root.0.join("a.ORF"), b"bounded source").unwrap();
    jpeg(&root.0.join("b.jpg"));
    let helper = root.0.join("timeout");
    assert!(
        Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&helper)
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../crema-image/tests/support/worker_child.rs")
            )
            .status()
            .unwrap()
            .success()
    );
    let requests = requests(&root.0);
    let a = requests[0].clone();
    let mut b = requests[1].clone();
    b.key.purpose = Purpose::Viewer;
    let metrics = Metrics::new(true);
    let runtime = PreviewRuntime::with_options(
        helper,
        CacheConfig {
            root: None,
            budget: 0,
        },
        metrics.clone(),
        || {},
    );
    runtime.replace(PreviewDemand {
        thumbnails: vec![a.clone()],
        ..Default::default()
    });
    wait(|| {
        metrics
            .records()
            .iter()
            .any(|record| record.event == "worker_spawned")
    });
    let selected_at = Instant::now();
    runtime.replace(PreviewDemand {
        selected: Some(b.clone()),
        thumbnails: vec![a.clone()],
    });
    wait(|| match runtime.try_recv() {
        Some(Event::Decoded {
            key,
            outcome: DecodeOutcome::Decoded(_),
        }) => key == b.key,
        Some(Event::Decoded { outcome, .. }) => panic!("obsolete failure leaked: {outcome:?}"),
        _ => false,
    });
    assert!(selected_at.elapsed() < Duration::from_secs(3));
    wait(|| {
        metrics
            .records()
            .iter()
            .filter(|record| record.event == "worker_spawned")
            .count()
            == 2
    });
    let records = metrics.records();
    let attempts: Vec<_> = records
        .iter()
        .filter(|record| record.event == "attempt_started" && record.key == Some(a.key))
        .collect();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].interest, attempts[1].interest);
    assert_ne!(attempts[0].attempt, attempts[1].attempt);
    let first_pid = records
        .iter()
        .find(|record| record.event == "worker_spawned")
        .unwrap()
        .value;
    assert!(
        records
            .iter()
            .any(|record| record.event == "worker_reaped" && record.value == first_pid)
    );
    let stopped_at = Instant::now();
    drop(runtime);
    assert!(stopped_at.elapsed() < Duration::from_secs(3));
    assert_eq!(
        metrics
            .records()
            .iter()
            .filter(|record| record.event == "worker_reaped")
            .count(),
        2
    );
}
#[test]
fn shutdown_joins_while_publication_is_full() {
    let root = Directory::new("pressure");
    jpeg(&root.0.join("a.jpg"));
    jpeg(&root.0.join("b.jpg"));
    let metrics = Metrics::new(true);
    let runtime = PreviewRuntime::with_options(
        std::env::current_exe().unwrap(),
        CacheConfig {
            root: None,
            budget: 0,
        },
        metrics.clone(),
        || {},
    );
    runtime.replace(PreviewDemand {
        thumbnails: requests(&root.0),
        ..Default::default()
    });
    wait(|| {
        metrics
            .records()
            .iter()
            .any(|record| record.event == "admission_wait")
    });
    let start = Instant::now();
    drop(runtime);
    assert!(start.elapsed() < Duration::from_secs(3));
}
#[test]
fn persistent_thumbnail_is_reused_by_a_fresh_real_process() {
    let root = Directory::new("restart");
    fs::create_dir(root.0.join("originals")).unwrap();
    let source = root.0.join("originals/photo.jpg");
    jpeg(&source);
    let original = fs::read(&source).unwrap();
    for name in ["cold", "warm", "viewer"] {
        let metric_path = root.0.join(format!("{name}.tsv"));
        let output = Command::new(env!("CARGO_BIN_EXE_crema-nitro"))
            .arg(if name == "viewer" {
                "viewer"
            } else {
                "thumbnail"
            })
            .arg(root.0.join("cache"))
            .arg(&metric_path)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metrics = fs::read_to_string(metric_path).unwrap();
        if name == "viewer" {
            let thumbnail = metrics
                .lines()
                .find(|line| line.contains("\tthumbnail_ready\t"))
                .expect("provisional thumbnail metric");
            assert_eq!(thumbnail.split('\t').nth(3), Some("Thumbnail"));
        } else if name == "cold" {
            assert!(metrics.contains("\tsource_read_bytes\t"));
        } else {
            assert!(metrics.contains("\tcache_hit\t"));
            assert!(!metrics.contains("\tsource_read_bytes\t"));
            assert!(!metrics.contains("\tworker_spawned\t"));
        }
    }
    let rejected = Command::new(env!("CARGO_BIN_EXE_crema-nitro"))
        .arg("next")
        .arg(root.0.join("cache"))
        .arg(root.0.join("invalid-next.tsv"))
        .arg(&source)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("at least two fixtures"));
    assert!(!root.0.join("invalid-next.tsv").exists());
    assert_eq!(fs::read(source).unwrap(), original);
}
