use crema_core::edit::EditRecipe;
use crema_image::{PreviewPixels, PreviewSize, edit_render::render_exposure_srgb8};
use std::{fs, fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 640;

fn pixels() -> Vec<u8> {
    let colors: [[u8; 3]; 6] = [
        [190, 55, 55],
        [210, 145, 45],
        [180, 185, 55],
        [55, 155, 85],
        [50, 120, 190],
        [145, 75, 180],
    ];
    let mut output = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let color = colors[(x * colors.len() as u32 / WIDTH) as usize];
            let level = 64 + y * 191 / (HEIGHT - 1);
            for channel in color {
                output.push((u32::from(channel) * level / 255) as u8);
            }
            output.push(255);
        }
    }
    output
}

fn jpeg() -> Result<Vec<u8>, String> {
    let preview = PreviewPixels::new(
        WIDTH,
        HEIGHT,
        pixels(),
        PreviewSize::new(WIDTH).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let rendered = render_exposure_srgb8(&preview, &EditRecipe::default());
    let mut jpeg = crema_image::jpeg_export::encode_srgb_jpeg(&rendered, 92)
        .map_err(|error| error.to_string())?;
    canonicalize_icc_datetime(&mut jpeg)?;
    Ok(jpeg)
}

fn canonicalize_icc_datetime(jpeg: &mut [u8]) -> Result<(), String> {
    const ICC_CHUNK: &[u8] = b"ICC_PROFILE\0\x01\x01";
    const DATETIME_OFFSET: usize = 24;
    let profile = jpeg
        .windows(ICC_CHUNK.len())
        .position(|window| window == ICC_CHUNK)
        .map(|offset| offset + ICC_CHUNK.len())
        .ok_or_else(|| "generated JPEG has no ICC profile".to_owned())?;
    let datetime = jpeg
        .get_mut(profile + DATETIME_OFFSET..profile + DATETIME_OFFSET + 12)
        .ok_or_else(|| "generated JPEG has a truncated ICC header".to_owned())?;
    for (target, value) in datetime
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([2024u16, 1, 1, 0, 0, 0])
    {
        target.copy_from_slice(&value.to_be_bytes());
    }
    Ok(())
}

fn write(output: &PathBuf, jpeg: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| format!("{}: {error}", output.display()))?;
    file.write_all(jpeg).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(())
}

fn run(output: PathBuf, count: Option<usize>) -> Result<(), String> {
    let jpeg = jpeg()?;
    let Some(count) = count else {
        return write(&output, &jpeg);
    };
    if !(1..=10_000).contains(&count) {
        return Err("fixture count must be 1..=10000".to_owned());
    }
    fs::create_dir(&output).map_err(|error| format!("{}: {error}", output.display()))?;
    let first = output.join("photo-00000.jpg");
    write(&first, &jpeg)?;
    for index in 1..count {
        let path = output.join(format!("photo-{index:05}.jpg"));
        fs::hard_link(&first, &path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let count_mode = first.as_deref() == Some(std::ffi::OsStr::new("--count"));
    let (count, output) = if count_mode {
        let count = args
            .next()
            .and_then(|value| value.to_str().and_then(|value| value.parse().ok()));
        (count, args.next().map(PathBuf::from))
    } else {
        (None, first.map(PathBuf::from))
    };
    if count_mode && count.is_none() {
        eprintln!("usage: crema-ui-fixture [--count 1..=10000] <new-output>");
        return ExitCode::FAILURE;
    }
    let Some(output) = output else {
        eprintln!("usage: crema-ui-fixture [--count 1..=10000] <new-output>");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("usage: crema-ui-fixture [--count 1..=10000] <new-output>");
        return ExitCode::FAILURE;
    }
    match run(output, count) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
