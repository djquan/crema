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
    match eframe::run_native(
        "Crema",
        options,
        Box::new(move |context| {
            Ok(Box::new(crema_app::Browser::new(
                PathBuf::from(root),
                executable,
                context.egui_ctx.clone(),
            )))
        }),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
