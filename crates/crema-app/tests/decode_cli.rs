use crema_image::{CandidateFormat, DecodeLimits, DecodeOutcome, Decoder, PreviewSize, RawFormat};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Directory(PathBuf);
impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("crema-decode-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn probe_decodes_real_jpeg_continues_failures_and_has_optional_strict_exit() {
    let directory = Directory::new("mixed");
    let image_path = directory.0.join("valid.jpg");
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut encoded)
        .encode(&[90; 18], 2, 3, image::ExtendedColorType::Rgb8)
        .unwrap();
    fs::write(&image_path, &encoded).unwrap();
    fs::write(directory.0.join("corrupt.jpg"), b"bad jpeg").unwrap();
    fs::write(directory.0.join("unsupported.png"), b"png candidate").unwrap();
    fs::write(directory.0.join("bad.ORF"), b"bad raw").unwrap();
    for strict in [false, true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crema-probe"));
        if strict {
            command.arg("--fail-on-decode-error");
        }
        let output = command.arg(&directory.0).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(if strict { 2 } else { 0 }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        let header: Vec<_> = lines[0].split('\t').collect();
        let rows: Vec<HashMap<_, _>> = lines[1..]
            .iter()
            .map(|line| {
                let fields: Vec<_> = line.split('\t').collect();
                assert_eq!(fields.len(), header.len());
                header.iter().copied().zip(fields).collect()
            })
            .collect();
        let valid = rows
            .iter()
            .find(|row| row["path"].ends_with("valid.jpg"))
            .unwrap();
        assert_eq!(valid["outcome"], "decoded");
        assert_eq!(valid["source_width"], "2");
        assert_eq!(valid["source_height"], "3");
        assert_eq!(valid["preview_width"], "2");
        assert_eq!(valid["preview_height"], "3");
        assert_eq!(valid["peak_rss_bytes"], "unknown:not-measured");
        assert_eq!(
            rows.iter().filter(|row| row["outcome"] == "failed").count(),
            2
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row["outcome"] == "unsupported")
                .count(),
            1
        );
    }
    assert_eq!(fs::read(image_path).unwrap(), encoded);
}

#[test]
fn real_worker_rejects_truncated_input_then_a_fresh_worker_reports_codec_failure() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_crema-probe"))
        .arg("--crema-decode-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"CREMAREQ").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let directory = Directory::new("worker-restart");
    let source = directory.0.join("bad.ORF");
    fs::write(&source, b"bad raw").unwrap();
    let decoder = Decoder::new(
        PathBuf::from(env!("CARGO_BIN_EXE_crema-probe")),
        DecodeLimits::default(),
    );
    for _ in 0..2 {
        let outcome = decoder.decode(
            &source,
            CandidateFormat::Raw(RawFormat::Orf),
            PreviewSize::new(16).unwrap(),
        );
        assert!(matches!(outcome, DecodeOutcome::Failed(_)));
    }
    assert_eq!(fs::read(source).unwrap(), b"bad raw");
}

#[test]
fn report_output_never_overwrites_an_existing_file() {
    let directory = Directory::new("output");
    let output = directory.0.join("existing.tsv");
    fs::write(&output, b"original report").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_crema-probe"))
        .arg("--output")
        .arg(&output)
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(output).unwrap(), b"original report");
}
