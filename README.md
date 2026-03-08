# Crema

A GPU-accelerated photo editor and browser, built in Rust. Think Lightroom, but open source.

## Features (v0.1)

- Import photos from files or folders (JPEG, PNG, TIFF, and 30 RAW formats via rawler)
- Browse photos in a responsive thumbnail grid (Library view) with date sidebar filtering
- Develop view with real-time editing, filmstrip navigation, and collapsible panels
- Non-destructive edits: exposure, contrast, highlights, shadows, blacks, white balance (temperature + tint), vibrance, saturation, crop
- One-click auto enhance (histogram analysis + gray-world white balance)
- Export processed photos to JPEG/PNG/TIFF
- Live RGB histogram
- EXIF metadata display (camera, lens, focal length, aperture, shutter speed, ISO)
- SQLite catalog with edit persistence
- Thumbnail caching (blake3 content-addressed)
- Native macOS menu bar with keyboard shortcuts (Cmd+I import, Cmd+E export)
- Cross-platform: macOS + Linux

## Building

Requires Rust (edition 2024). On Linux, you also need:

```bash
sudo apt-get install libwayland-dev libxkbcommon-dev libgtk-3-dev libxdo-dev
```

Then:

```bash
cargo build --workspace
cargo run
```

## Architecture

```
crema/
  src/                          # Binary crate (iced GUI)
  crates/
    crema-core/                 # ImageBuf, processing pipeline, RAW decode
    crema-gpu/                  # wgpu compute shaders for image processing
    crema-catalog/              # SQLite catalog and folder import
    crema-metadata/             # EXIF reading
    crema-thumbnails/           # Thumbnail generation and disk cache
```

The processing pipeline operates entirely in linear f32 RGB (scene-referred). Each edit module implements the `ProcessingModule` trait and transforms an `ImageBuf`. sRGB gamma is only applied at display time.

## License

GPL-3.0-only
