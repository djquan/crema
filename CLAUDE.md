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

Linux requires: `sudo apt-get install libwayland-dev libxkbcommon-dev libgtk-3-dev libxdo-dev`

## Architecture

Crema is a GPU-accelerated photo editor structured as a Cargo workspace with five library crates and one binary crate.

### Crate Dependency Graph

```
crema (binary, iced app)
  ├── crema-core       (ImageBuf, Pipeline, ProcessingModule trait, rawler/image loading)
  ├── crema-gpu        (wgpu context, textures, WGSL compute shaders) -> depends on crema-core
  ├── crema-catalog    (SQLite via rusqlite, import, models) -> depends on crema-core, crema-metadata
  ├── crema-metadata   (EXIF reading via kamadak-exif)
  └── crema-thumbnails (blake3 disk cache, resize) -> depends on crema-core
```

### End-to-End Data Flow

```
                                        ┌──────────────────┐
                                        │   SQLite catalog │
                                        │  ~/.local/share/ │
                                        │  crema/catalog.db│
                                        └──────┬───────────┘
                                               │ photo paths, edits
    ┌──────────┐     ┌──────────────┐   ┌──────┴───────────┐     ┌──────────────────┐
    │ RAW file │────>│ rawler       │──>│                   │────>│ to_rgba_u8_srgb()│
    │ (CR2,NEF │     │ decode +     │   │ ImageBuf          │     │ (LUT-based gamma)│
    │  ARW...) │     │ sRGB→linear  │   │ linear f32 RGB   │     └────────┬─────────┘
    └──────────┘     └──────────────┘   │ scene-referred   │              │
                                        │                   │     ┌───────┴──────────┐
    ┌──────────┐     ┌──────────────┐   │ CPU Pipeline:    │     │ iced image widget│
    │ JPEG/PNG │────>│ image crate  │──>│  WB -> Exp -> Crop│     │ (display)        │
    │ TIFF     │     │ u8→linear    │   │                   │     └──────────────────┘
    └──────────┘     └──────────────┘   └───────────────────┘
                                               │
                                        ┌──────┴───────────┐
                                        │ GPU Pipeline:    │
                                        │  WB -> Exposure  │
                                        │ (WGSL compute    │
                                        │  shaders)        │
                                        └──────────────────┘
```

All pixel processing happens in **linear light f32**. The `ImageBuf` type (`crema-core/src/image_buf.rs`) is the universal pixel container: flat `Vec<f32>` in `[R,G,B,R,G,B,...]` layout. Values are scene-referred (unbounded above 1.0). sRGB gamma is only applied at the final display step via LUT-based conversion.

**Gamma conversion details:** rawler outputs sRGB gamma, so `raw.rs` immediately converts to linear using a 4096-entry LUT (`SRGB_F32_TO_LINEAR`). Standard images (JPEG/PNG) use a 256-entry `SRGB_U8_TO_LINEAR` LUT. The reverse (`to_rgba_u8_srgb`) uses a 4096-entry `SRGB_LUT` for f32-to-u8.

---

### crema-core

The foundational crate. Three modules:

**`image_buf.rs`** — The `ImageBuf` pixel container and `EditParams`:
- `ImageBuf { width: u32, height: u32, data: Vec<f32> }` (RGB, 3 floats per pixel)
- `to_rgba_f32()` for GPU upload (adds alpha=1.0), `to_rgba_u8_srgb()` for display
- `downsample(max_edge)` box-averages to a smaller size (used for 2048px editing preview)
- `EditParams` holds all edit state: exposure (EV stops), wb_temp (Kelvin), wb_tint, crop (normalized 0..1). Derives `Serialize`/`Deserialize` for SQLite persistence.

**`raw.rs`** — File loading:
- `RAW_EXTENSIONS`: 27 formats (cr2, cr3, nef, arw, dng, orf, raf, etc.)
- `IMAGE_EXTENSIONS`: jpg, jpeg, png, tiff, tif
- `decode_raw(path)`: rawler decode -> `RawDevelop::default().develop_intermediate()` -> sRGB-to-linear conversion -> `ImageBuf`
- `load_image(path)` / `load_any(path)`: dispatch by extension, standard images via `image` crate
- `load_any_scaled(path, max_edge)`: for standard images, resizes in u8 space *before* linear conversion (performance optimization; RAW must decode full then downsample)

**`pipeline/`** — Processing chain:
- `ProcessingModule` trait: `fn process_cpu(&self, input: ImageBuf, params: &EditParams) -> Result<ImageBuf>`
- `Pipeline::new()` chains: **WhiteBalance -> Exposure -> Crop**
- Each module has early-return identity checks (e.g. exposure=0 skips processing)
- WhiteBalance: converts (temp, tint) to per-channel multipliers relative to D55 (5500K). `r_mult = 1.0 + shift*0.3`, `b_mult = 1.0 - shift*0.3`, `g_mult = 1.0 + tint*0.01`, all clamped >= 0.1
- Exposure: `multiplier = 2^exposure`, applied to all pixels
- Crop: normalized (0..1) params mapped to pixel coords, copies scanlines to new buffer

---

### crema-gpu

Mirrors the CPU pipeline on the GPU. **Not yet integrated into the iced app** (exists for future use).

**`context.rs`** — `GpuContext`: wgpu instance + adapter (HighPerformance) + device + queue. Headless compute only (no surface).

**`texture.rs`** — `GpuTexture`: GPU-resident `Rgba32Float` textures.
- `from_image_buf()`: upload via `queue.write_texture()` with proper row alignment
- `create_storage()`: empty output textures for compute shader writes
- `download()`: GPU->CPU readback with staging buffer, `PollType::Wait`, extracts RGB from RGBA back to `ImageBuf`

**`pipeline.rs`** — `GpuPipeline`: chains WhiteBalance -> Exposure as compute dispatches.
- Each stage: create output texture, write uniform buffer (multipliers), dispatch `ceil(w/16) x ceil(h/16)` workgroups
- Pipelines cached in `ShaderManager` to avoid recompilation

**`shaders/`** — WGSL compute shaders:
- `white_balance.wgsl`: per-pixel RGB multiply by (r_mult, g_mult, b_mult) uniform
- `exposure.wgsl`: per-pixel RGB multiply by single multiplier uniform
- Both use `@workgroup_size(16, 16)` with bounds checking

---

### crema-catalog

SQLite persistence layer. Database at `~/.local/share/crema/catalog.db`.

**Schema** (two tables):
```sql
photos (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL UNIQUE,  -- canonical absolute path
    file_hash TEXT NOT NULL,         -- blake3 content hash
    file_size INTEGER NOT NULL,
    width, height INTEGER,           -- from EXIF
    camera_make, camera_model, lens TEXT,
    focal_length, aperture REAL,
    shutter_speed TEXT, iso INTEGER,
    date_taken TEXT,                  -- EXIF DateTimeOriginal
    imported_at TEXT DEFAULT now,
    thumbnail_path TEXT
)
-- INDEX on file_hash

edits (
    id INTEGER PRIMARY KEY,
    photo_id INTEGER NOT NULL UNIQUE REFERENCES photos(id),
    exposure, wb_temp, wb_tint, crop_x, crop_y, crop_w, crop_h REAL,
    updated_at TEXT DEFAULT now
)
```

**Key patterns:**
- `insert_photo()`: `INSERT OR IGNORE` on `file_path` UNIQUE constraint; returns `Some(id)` on insert, `None` on duplicate
- `save_edits()`: `INSERT ... ON CONFLICT(photo_id) DO UPDATE SET ...` (upsert)
- `list_photos()`: ordered by `date_taken DESC, id DESC`

**Import module** (`import.rs`):
- `import_file(catalog, path)`: canonicalize -> blake3 hash -> extract EXIF -> insert
- `import_folder(catalog, folder)`: scan dir for supported extensions, call `import_file` each
- `import_paths(catalog, paths)`: mixed files/directories; directories delegate to `import_folder`

---

### crema-metadata

Thin EXIF extraction wrapper around `kamadak-exif`.

`ExifData` struct: 11 optional fields (width, height, camera_make, camera_model, lens, focal_length, aperture, shutter_speed, iso, date_taken, orientation).

`from_file(path)` reads the primary IFD. Helper functions handle type coercion (Rational->f64, Short/Long->u32). `summary_lines()` returns display-friendly `Vec<(String, String)>` for the metadata panel.

---

### crema-thumbnails

Disk-cached thumbnail generation.

**`cache.rs`** — `ThumbnailCache`:
- Path bucketing: `{cache_dir}/{hash[0..2]}/{hash}.jpg`
- `store(hash, bytes)` / `load(hash)` / `has_thumbnail(hash)`
- Cache dir: `~/.cache/crema/thumbnails/`

**`generator.rs`**:
- `generate_thumbnail(buf)`: `ImageBuf` -> sRGB u8 -> resize to 512px longest edge (Lanczos3) -> JPEG encode
- `fast_thumbnail(path)`: intended for embedded RAW thumbnail extraction (currently falls back to full decode)
- Cache key in `app.rs`: blake3 hash of `path + modification_time`

---

### Binary Crate (iced App)

Uses iced 0.14's **function-based API** (not the old `Application` trait):
- `iced::application(App::new, App::update, App::view).run()`
- No `horizontal_space()` exists; use `Space::new().width(Length::Fill)`
- Window: 1400x900, dark theme, antialiasing enabled

**Two views** controlled by `View` enum in `app.rs`:

```
Lighttable                              Darkroom
┌─────────────────────────────┐         ┌──────────────────────────────┐
│ "Crema"        [Import]     │         │ [< Back]        "Darkroom"  │
├────────┬────────────────────┤         ├───────────────────┬──────────┤
│ Date   │                    │         │                   │Histogram │
│ sidebar│  Thumbnail grid    │         │   Processed       ├──────────┤
│ (tree) │  5 cols x 200px    │         │   image           │Edit panel│
│        │  click->darkroom   │         │   (fill)          │ 3 sliders│
│        │                    │         │                   ├──────────┤
│        │                    │         │                   │EXIF panel│
├────────┴────────────────────┤         ├───────────────────┴──────────┤
│ Status: "42 photos"         │         │                              │
└─────────────────────────────┘         └──────────────────────────────┘
```

**Message-driven architecture** — key flows:

1. **Startup**: open catalog -> `list_photos()` -> spawn thumbnail load tasks (cached + async)
2. **Import**: `rfd::AsyncFileDialog::pick_files()` with extension filter -> `import_paths()` -> refresh
3. **Open photo**: `load_any()` full-res + 2048px preview async -> store `Arc<ImageBuf>` -> `reprocess_image()`
4. **Edit slider**: update `EditParams` -> `reprocess_image()` -> CPU pipeline on preview -> histogram -> display
5. **Debouncing**: `processing_generation: u64` counter; stale `ImageProcessed` results are discarded
6. **Edit persistence**: `save_edits()` called when `ImageProcessed` completes (natural debounce) and on `BackToGrid`

**Date sidebar** (`widgets/date_sidebar.rs`): hierarchical year > month > day tree built from photo `date_taken` fields, with expand/collapse and filter-by-click. `DateFilter` enum filters `filtered_photos()`.

**Histogram** (`widgets/histogram.rs`): iced canvas widget, three semi-transparent RGB channels, log scale (`ln_1p`).

### Key Version Constraints

- **wgpu must be 27.x** to match iced 0.14's pinned version. wgpu 27 uses `PollType::Wait` (not `Maintain::Wait`) and `request_device()` takes one argument.
- **rawler 0.7.1**: Use `rawler::decode_file()` then `RawDevelop::default().develop_intermediate()`. The result is `Intermediate::ThreeColor(Color2D<f32, 3>)` where `data: Vec<[f32; 3]>`.
- **Rust edition 2024**.

### Dev Profile

Dependencies are compiled at `opt-level = 2` even in dev builds (configured in workspace `Cargo.toml`). Without this, per-pixel loops in rawler/image/jpeg-decoder run ~10x slower. `crema-core` and `crema-thumbnails` are also at opt-level 2 for the same reason.
