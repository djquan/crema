use crema_core::{ScanEvent, scan_folder};
use crema_image::classify_candidate;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let Some(root) = arguments.next() else {
        return usage_error();
    };
    if arguments.next().is_some() {
        return usage_error();
    }

    let scan = match scan_folder(PathBuf::from(root), classify_candidate) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let mut failed = false;
    for event in scan {
        match event {
            ScanEvent::Candidate(candidate) => println!(
                "{}\t{}\t{}",
                candidate.id(),
                candidate.kind(),
                candidate.path().display()
            ),
            ScanEvent::Failure(error) => {
                failed = true;
                eprintln!("{error}");
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn usage_error() -> ExitCode {
    eprintln!("usage: crema-scan <folder>");
    ExitCode::FAILURE
}
