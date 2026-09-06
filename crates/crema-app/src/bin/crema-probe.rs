use crema_core::{ScanEvent, scan_folder};
use crema_image::{
    DecodeLimits, DecodeOutcome, Decoder, Fact, PreviewSize, UnknownReason, classify_candidate,
};
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
    process::ExitCode,
    time::Instant,
};

const HEADER: &str = "path\tcandidate\toutcome\tprovenance\tsource_width\tsource_height\tdecoded_width\tdecoded_height\tpreview_width\tpreview_height\tsource_bits\tdecoded_bits\tdisplay_bits\torientation\ticc\tnclx\tcamera_make\tcamera_model\tlimitations\telapsed_ms\tpeak_rss_bytes\tfailure_class\terror";

fn main() -> ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--crema-decode-worker"))
    {
        return match crema_image::run_worker(io::stdin().lock(), io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    match run() {
        Ok(failed) => {
            if failed {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut root = None;
    let mut output = None;
    let mut fail_on_error = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--output") => {
                output = Some(PathBuf::from(args.next().ok_or("--output needs a path")?))
            }
            Some("--fail-on-decode-error") => fail_on_error = true,
            Some(value) if value.starts_with("--") => {
                return Err(format!("unknown option {value}").into());
            }
            _ if root.is_none() => root = Some(PathBuf::from(arg)),
            _ => {
                return Err(
                    "usage: crema-probe [--output <tsv>] [--fail-on-decode-error] <file-or-folder>"
                        .into(),
                );
            }
        }
    }
    let root = root
        .ok_or("usage: crema-probe [--output <tsv>] [--fail-on-decode-error] <file-or-folder>")?;
    let mut candidates = Vec::new();
    let mut failed = false;
    if root.is_file() {
        candidates.push((
            root.clone(),
            classify_candidate(&root).ok_or("unrecognized file extension")?,
        ));
    } else {
        for event in scan_folder(&root, classify_candidate)? {
            match event {
                ScanEvent::Candidate(candidate) => {
                    candidates.push((candidate.path().to_owned(), *candidate.kind()))
                }
                ScanEvent::Failure(error) => {
                    failed = true;
                    eprintln!("{error}");
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let mut writer: Box<dyn Write> = match output {
        Some(path) => Box::new(BufWriter::new(
            File::options().write(true).create_new(true).open(path)?,
        )),
        None => Box::new(io::stdout().lock()),
    };
    writeln!(writer, "{HEADER}")?;
    let decoder = Decoder::new(std::env::current_exe()?, DecodeLimits::default());
    let size = PreviewSize::new(1600)?;
    for (path, candidate) in candidates {
        let started = Instant::now();
        let outcome = decoder.decode(&path, candidate, size);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let mut fields = vec![String::new(); 23];
        fields[0] = path.to_string_lossy().into_owned();
        fields[1] = candidate.to_string();
        fields[19] = format!("{elapsed:.3}");
        fields[20] = Fact::<u64>::Unknown(UnknownReason::NotMeasured).to_string();
        match outcome {
            DecodeOutcome::Decoded(result) => {
                fields[2] = "decoded".into();
                fields[3] = result.provenance.to_string();
                let facts = result.metadata;
                for (index, value) in [
                    facts.dimensions[0],
                    facts.dimensions[1],
                    facts.decoded_dimensions[0],
                    facts.decoded_dimensions[1],
                    result.preview.width(),
                    result.preview.height(),
                ]
                .into_iter()
                .enumerate()
                {
                    fields[index + 4] = value.to_string();
                }
                fields[10] = facts.source_bits.to_string();
                fields[11] = facts.decoded_bits.to_string();
                fields[12] = "8".into();
                fields[13] = facts.orientation.to_string();
                fields[14] = facts.icc.to_string();
                fields[15] = facts
                    .nclx
                    .map(|values| values.map(|v| v.to_string()).join(","))
                    .unwrap_or_default();
                fields[16] = facts.camera_make;
                fields[17] = facts.camera_model;
                fields[18] = facts.limitations;
            }
            DecodeOutcome::Unsupported(reason) => {
                fields[2] = "unsupported".into();
                fields[22] = reason;
            }
            DecodeOutcome::Failed(error) => {
                failed = true;
                fields[2] = "failed".into();
                fields[21] = error.class.to_string();
                fields[22] = error.message;
            }
        }
        writeln!(
            writer,
            "{}",
            fields
                .iter()
                .map(|value| escape(value))
                .collect::<Vec<_>>()
                .join("\t")
        )?;
        writer.flush()?;
    }
    Ok(failed && fail_on_error)
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}
