use crema_app::{
    jobs::{Event, JobKey, PreviewDemand, PreviewRequest, PreviewRuntime, Purpose},
    metrics::Metrics,
    thumbnail_cache::{CacheConfig, DEFAULT_BUDGET},
};
use crema_core::{ScanEvent, scan_folder};
use crema_image::{DecodeOutcome, classify_candidate};
use std::{
    io,
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

fn wait_until(mut predicate: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !predicate() {
        if Instant::now() >= deadline {
            return Err("scenario deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
fn select(runtime: &PreviewRuntime, metrics: &Metrics, request: PreviewRequest) {
    metrics.record("operation_selected", Some(request.key), 0, 0, 1);
    runtime.replace(PreviewDemand {
        selected: Some(request),
        ..Default::default()
    });
}
fn ready(
    runtime: &PreviewRuntime,
    metrics: &Metrics,
    request: &PreviewRequest,
) -> Result<(), String> {
    let mut failure = None;
    let mut first = true;
    wait_until(|| {
        while let Some(event) = runtime.try_recv() {
            if let Event::Decoded { key, outcome } = event {
                if key.asset != request.key.asset || key.generation != request.key.generation {
                    continue;
                }
                match outcome {
                    DecodeOutcome::Decoded(_) => {
                        if first {
                            metrics.record("first_usable_received", Some(key), 0, 0, 1);
                            first = false;
                        }
                        if key.purpose == request.key.purpose {
                            metrics.record("target_received", Some(key), 0, 0, 1);
                            return true;
                        }
                    }
                    DecodeOutcome::Unsupported(reason) => {
                        failure = Some(reason);
                        return true;
                    }
                    DecodeOutcome::Failed(error) => {
                        failure = Some(error.to_string());
                        return true;
                    }
                }
            }
        }
        false
    })?;
    failure.map_or(Ok(()), Err)
}
fn run(args: Vec<std::ffi::OsString>) -> Result<(), String> {
    if args.len() < 4 {
        return Err("usage: crema-nitro <thumbnail|viewer|next|cancel|aba|pressure> <cache-root|-> <events.tsv> <photo> [photo...]".into());
    }
    let scenario = args[0].to_string_lossy();
    if scenario == "next" && args.len() < 5 {
        return Err("next-photo requires at least two fixtures".into());
    }
    let metric_path = PathBuf::from(&args[2]);
    let metrics = Metrics::new(true);
    metrics.record("runtime_process_start", None, 0, 0, 1);
    let mut requests = Vec::new();
    for path in &args[3..] {
        let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
        let scan = scan_folder(
            path.parent().ok_or("source parent missing")?,
            classify_candidate,
        )
        .map_err(|error| error.to_string())?;
        let candidate = scan
            .filter_map(|event| match event {
                ScanEvent::Candidate(candidate) if candidate.path() == path => Some(candidate),
                _ => None,
            })
            .next()
            .ok_or("fixture is not a supported candidate")?;
        requests.push(PreviewRequest {
            key: JobKey {
                generation: 1,
                asset: candidate.id(),
                purpose: if scenario == "thumbnail" {
                    Purpose::Thumbnail
                } else {
                    Purpose::Viewer
                },
            },
            path,
            format: *candidate.kind(),
            needed: true,
        });
    }
    metrics.record("fixture_scan_finished", None, 0, 0, requests.len() as u64);
    let cache = CacheConfig {
        root: (args[1] != "-").then(|| PathBuf::from(&args[1])),
        budget: DEFAULT_BUDGET,
    };
    let runtime = PreviewRuntime::with_options(
        std::env::current_exe().map_err(|error| error.to_string())?,
        cache,
        metrics.clone(),
        || {},
    );
    let result = (|| {
        match scenario.as_ref() {
            "thumbnail" | "viewer" => {
                select(&runtime, &metrics, requests[0].clone());
                ready(&runtime, &metrics, &requests[0])?;
            }
            "next" => {
                for request in &requests {
                    select(&runtime, &metrics, request.clone());
                    ready(&runtime, &metrics, request)?;
                }
            }
            "cancel" | "aba" => {
                let a = requests[0].clone();
                let b = requests
                    .get(1)
                    .ok_or("cancel/aba need two photos, first must use a heavy worker")?
                    .clone();
                select(&runtime, &metrics, a.clone());
                wait_until(|| {
                    metrics
                        .records()
                        .iter()
                        .any(|record| record.event == "worker_spawned")
                })?;
                select(&runtime, &metrics, b.clone());
                if scenario == "aba" {
                    wait_until(|| {
                        metrics.records().iter().any(|record| {
                            record.event == "attempt_started" && record.key == Some(b.key)
                        })
                    })?;
                    select(&runtime, &metrics, a.clone());
                    ready(&runtime, &metrics, &a)?;
                } else {
                    ready(&runtime, &metrics, &b)?;
                }
                if !metrics
                    .records()
                    .iter()
                    .any(|record| record.event == "worker_reaped")
                {
                    return Err("cancelled worker was not reaped".into());
                }
            }
            "pressure" => {
                let first = requests[0].clone();
                let thumbnails = requests
                    .iter()
                    .map(|request| PreviewRequest {
                        key: JobKey {
                            purpose: Purpose::Thumbnail,
                            ..request.key
                        },
                        ..request.clone()
                    })
                    .collect();
                runtime.replace(PreviewDemand {
                    thumbnails,
                    ..Default::default()
                });
                wait_until(|| {
                    metrics
                        .records()
                        .iter()
                        .any(|record| record.event == "thumbnail_ready")
                })?;
                select(&runtime, &metrics, first.clone());
                ready(&runtime, &metrics, &first)?;
            }
            _ => return Err("unknown scenario".into()),
        }
        Ok(())
    })();
    drop(runtime);
    metrics.record("runtime_joined", None, 0, 0, 1);
    metrics
        .save(&metric_path)
        .map_err(|error| error.to_string())?;
    result
}
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--crema-decode-worker")
    {
        return match crema_image::run_worker(io::stdin().lock(), io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
