use std::{io, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let root = args.next();
    if root.as_deref() == Some(std::ffi::OsStr::new("--crema-decode-worker")) {
        return match crema_image::run_worker(io::stdin().lock(), io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    let Some(root) = root else {
        eprintln!("usage: crema <folder>");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("usage: crema <folder>");
        return ExitCode::FAILURE;
    }
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([640.0, 420.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let metric_path = std::env::var_os("CREMA_METRICS").map(PathBuf::from);
    let metrics = crema_app::metrics::Metrics::new(metric_path.is_some());
    metrics.record("gui_launch", None, 0, 0, 1);
    let mut cache = crema_app::thumbnail_cache::CacheConfig::default();
    if let Some(root) = std::env::var_os("CREMA_CACHE_ROOT") {
        cache.root = Some(PathBuf::from(root));
    }
    if std::env::var_os("CREMA_CACHE_DISABLED").is_some() {
        cache.root = None;
    }
    let result = eframe::run_native(
        "Crema",
        options,
        Box::new(move |context| {
            Ok(Box::new(crema_app::Browser::with_options(
                PathBuf::from(root),
                executable,
                context.egui_ctx.clone(),
                cache,
                metrics,
                metric_path,
            )))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
