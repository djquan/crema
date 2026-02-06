# Photors

A GPU-accelerated photo editor and browser, built in Rust. Think Lightroom, but open source.

## Features (v0.1)

- Import photos from a folder (JPEG, PNG, TIFF, and 30+ RAW formats via rawler)
- Browse photos in a thumbnail grid (lighttable view)
- Open a photo in the darkroom view with real-time editing
- Non-destructive edits: exposure compensation, white balance (temperature + tint)
- Live RGB histogram
- EXIF metadata display (camera, lens, focal length, aperture, shutter speed, ISO)
- SQLite catalog with edit persistence
- Thumbnail caching (blake3 content-addressed)
- Cross-platform: macOS + Linux

## Building

Requires Rust (edition 2024). On Linux, you also need:

```bash
sudo apt-get install libwayland-dev libxkbcommon-dev
```

Then:

```bash
cargo build --workspace
cargo run
```

## Architecture

```
photors/
  src/                          # Binary crate (iced GUI)
  crates/
    photors-core/               # ImageBuf, processing pipeline, RAW decode
    photors-gpu/                # wgpu compute shaders for image processing
    photors-catalog/            # SQLite catalog and folder import
    photors-metadata/           # EXIF reading
    photors-thumbnails/         # Thumbnail generation and disk cache
```

The processing pipeline operates entirely in linear f32 RGB (scene-referred). Each edit module implements the `ProcessingModule` trait and transforms an `ImageBuf`. sRGB gamma is only applied at display time.

## License

GPL-3.0-only
