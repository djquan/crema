# Crema

Crema is an experimental desktop photo browser and non-destructive editor written
in Rust. It runs on macOS, Windows, and Linux through `eframe`, `egui`, and WGPU.

The current Phase 0 application opens one folder, fills a virtualized thumbnail
grid as the scan progresses, and shares selection between the grid and photo
viewer. It decodes JPEG, RAW, and a narrow SDR HEIC subset. The editor supports
exposure, before and after comparison, Crema-owned XMP sidecars, and profiled JPEG
export. Originals remain unchanged.

Phase 0 is implemented but has not passed its acceptance gate. Representative
camera files, native Windows and Linux UI runs, assistive technology, display
color, and release-hardware performance still need evidence.

## Run Crema

Crema requires Rust 1.95 or newer.

```sh
cargo run --release -p crema-app --bin crema -- /path/to/photos
```

The folder scan is non-recursive and uses filesystem order. The app does not yet
have a folder picker, catalog database, sorting, filtering, ratings, keywords, or
file management.

In the grid, click a thumbnail to select it. Double-click or press Enter to open
the viewer. Use Left and Right to change the selected photo. Press Escape to
return to the grid.

The viewer offers fit and 100% display of the decoded preview, which has a maximum
long edge of 4096 pixels. Exposure ranges from -5.00 to +5.00 EV. Dragging uses a
1024-pixel preview, and releasing the slider renders the detailed preview. **Before**
shows the unedited preview without changing the recipe.

## Data safety

Crema opens originals read-only. It can create a new sidecar or update one that
uses the Crema XMP schema and has an unambiguous association with its original.
Foreign, malformed, newer, or ambiguous sidecars stay unchanged and make saving
read-only. Closing with an unsaved exposure change asks whether to keep editing or
discard it.

Export writes `<complete-original-filename>-crema.jpg` beside the source. It
refuses to replace an existing file. Preview, sidecar save, and export all verify
that the original still identifies the same file before accepting their result.

The persistent cache stores only 320-pixel thumbnails in the platform cache
directory. Viewer pixels, selection, and unsaved edits remain in memory.

## Image limits

JPEG decoding applies EXIF orientation and converts supported embedded ICC
profiles to sRGB. RAW uses Rawler's baseline development and does not establish
camera-specific color or development quality. HEIC accepts only complete 8-bit
SDR NCLX `[1, 13, 6, 0]` input without an embedded ICC profile. Other HEIC color
paths fail as unsupported. PNG and TIFF files appear as candidates but do not
decode yet.

Exposure operates on display-ready 8-bit pixels. The app does not manage display
profiles, render scene-linear edits, or export full-resolution images. JPEG
exports have a maximum long edge of 4096 pixels and include an sRGB profile.

## Development

The main checks match CI.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -B -m unittest discover -s scripts -p 'test_*.py'
cargo build --release -p crema-app --bins
```

The repository also provides `crema-scan`, `crema-probe`,
`crema-runtime-bench`, and `crema-ui-fixture` for focused evidence collection.
The [Phase 0 status](docs/phase0-status.md) describes the current evidence command
and open gates.

Read [architecture.md](architecture.md) for the runtime design,
[the product roadmap](docs/roadmap.md) for planned work, and
[the decode reference](docs/decode-prototype.md) for protocol and cache details.

Crema's original code uses the [MIT License](LICENSE). Dependencies retain their
own licenses.
