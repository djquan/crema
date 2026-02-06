# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build --workspace          # build everything
cargo test --workspace           # run all tests
cargo test -p crema-core         # test a single crate
cargo test -p crema-catalog -- db::tests::save_and_load_edits  # single test
cargo clippy --workspace         # lint (CI runs with -D warnings)
cargo run                        # launch the GUI app
RUST_LOG=debug cargo run         # launch with verbose logging
```

Linux requires: `sudo apt-get install libwayland-dev libxkbcommon-dev`

## Architecture

Crema is a GPU-accelerated photo editor structured as a Cargo workspace with five library crates and one binary crate.

### Data Flow

```
RAW/JPEG file
  -> rawler decode (or image crate)
  -> ImageBuf (linear f32 RGB, scene-referred)
  -> CPU Pipeline: WhiteBalance -> Exposure -> Crop
  -> to_rgba_u8_srgb() for display
  -> iced image widget
```

All pixel processing happens in **linear light f32**. The `ImageBuf` type (`crema-core/src/image_buf.rs`) is the universal pixel container. sRGB gamma is only applied at the final display step. rawler's output comes back in sRGB gamma, so `raw.rs` immediately converts to linear.

### Crate Dependency Graph

```
crema (binary, iced app)
  ├── crema-core       (ImageBuf, Pipeline, ProcessingModule trait, rawler/image loading)
  ├── crema-gpu        (wgpu context, textures, WGSL compute shaders) -> depends on crema-core
  ├── crema-catalog    (SQLite via rusqlite, import, models) -> depends on crema-core, crema-metadata
  ├── crema-metadata   (EXIF reading via kamadak-exif)
  └── crema-thumbnails (blake3 disk cache, resize) -> depends on crema-core
```

### Processing Pipeline

The `ProcessingModule` trait (`crema-core/src/pipeline/module.rs`) defines one method: `process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf>`. Modules are chained in `Pipeline::new()`. The GPU pipeline (`crema-gpu/src/pipeline.rs`) mirrors this with WGSL compute shaders in `crates/crema-gpu/shaders/`.

### iced Application Pattern

Uses iced 0.14's **function-based API** (not the old `Application` trait):
- `iced::application(App::new, App::update, App::view).run()`
- No `horizontal_space()` exists; use `Space::new().width(Length::Fill)`
- The `Shader` widget uses `Program` / `Primitive` / `Pipeline` traits for custom wgpu rendering

The app has two views: `Lighttable` (thumbnail grid) and `Darkroom` (full image + edit panel). View switching is handled by the `View` enum in `app.rs`. Edit params persist to SQLite when the processed image completes (natural debounce).

### Key Version Constraints

- **wgpu must be 27.x** to match iced 0.14's pinned version. wgpu 27 uses `PollType::Wait` (not `Maintain::Wait`) and `request_device()` takes one argument.
- **rawler 0.7.1**: Use `rawler::decode_file()` then `RawDevelop::default().develop_intermediate()`. The result is `Intermediate::ThreeColor(Color2D<f32, 3>)` where `data: Vec<[f32; 3]>`.
- **Rust edition 2024**.

### Catalog

SQLite database at `~/.local/share/crema/catalog.db` with two tables: `photos` (file metadata, EXIF) and `edits` (non-destructive edit params per photo, UNIQUE on photo_id with proper upsert). Files are identified by blake3 content hash.

### Thumbnail Cache

Disk cache at configurable path, keyed by blake3 hash of path + modification time. Files stored as `{hash[0..2]}/{hash}.jpg` for filesystem bucketing.
