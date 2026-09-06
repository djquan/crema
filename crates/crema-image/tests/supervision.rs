use crema_image::{
    CandidateFormat, DecodeLimits, DecodeOutcome, Decoder, FailureClass, PreviewSize, RawFormat,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct TestDirectory(PathBuf);
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn real_children_are_reaped_after_bad_frames_exit_timeout_and_unbounded_stderr() {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("crema-supervision-{}", std::process::id())),
    );
    fs::create_dir(&directory.0).unwrap();
    let helper = directory.0.join("helper");
    let status = Command::new("rustc")
        .arg("--edition=2024")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/worker_child.rs"))
        .arg("-o")
        .arg(&helper)
        .status()
        .unwrap();
    assert!(status.success());
    let original = directory.0.join("input.ORF");
    fs::write(&original, b"bounded source bytes").unwrap();
    for (mode, expected) in [
        ("truncated", FailureClass::Protocol),
        ("oversized", FailureClass::Protocol),
        ("exit", FailureClass::WorkerExited),
        ("timeout", FailureClass::Timeout),
        ("stderr", FailureClass::Timeout),
    ] {
        let executable = directory
            .0
            .join(format!("{mode}{}", std::env::consts::EXE_SUFFIX));
        fs::copy(&helper, &executable).unwrap();
        let decoder = Decoder::new(
            executable,
            DecodeLimits {
                timeout: Duration::from_millis(250),
                ..DecodeLimits::default()
            },
        );
        let started = Instant::now();
        let DecodeOutcome::Failed(error) = decoder.decode(
            &original,
            CandidateFormat::Raw(RawFormat::Orf),
            PreviewSize::new(16).unwrap(),
        ) else {
            panic!("helper must fail");
        };
        assert_eq!(error.class, expected, "mode {mode}: {error}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(error.message.len() <= 66_000);
    }
    let decoder = std::sync::Arc::new(Decoder::new(
        directory
            .0
            .join(format!("timeout{}", std::env::consts::EXE_SUFFIX)),
        DecodeLimits::default(),
    ));
    let worker_decoder = decoder.clone();
    let worker_source = original.clone();
    let worker = std::thread::spawn(move || {
        worker_decoder.decode(
            &worker_source,
            CandidateFormat::Raw(RawFormat::Orf),
            PreviewSize::new(16).unwrap(),
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    decoder.cancel();
    let DecodeOutcome::Failed(error) = worker.join().unwrap() else {
        panic!("cancelled worker must fail");
    };
    assert_eq!(error.class, FailureClass::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(fs::read(original).unwrap(), b"bounded source bytes");
}
