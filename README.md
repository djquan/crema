# Crema

A planned desktop RAW photo manager and non-destructive editor for macOS,
Windows, and Linux, built with Rust.

Browse existing folders, organize photographs, and keep metadata and editing
instructions in XMP sidecars beside the originals.

Broad RAW, HEIC, and JPEG support is a core goal. Fujifilm-specific validation
is deferred while the first editing path settles.

The current prototype opens a real folder progressively, displays JPEG, RAW, and
HEIC previews, and keeps selection shared between a thumbnail grid and a viewer.
RAW development and HEIC decoding run in a fresh worker process for each file.
Originals remain untouched. PNG and TIFF candidates appear as unsupported.

```sh
cargo run --release -p crema-app --bin crema -- /path/to/photos
cargo run -p crema-app --bin crema-scan -- /path/to/photos
cargo run --release -p crema-app --bin crema-probe -- /path/to/photos
```

Click a thumbnail to select it. Double-click or press Enter to open the viewer.
Left and right arrows change selection. Escape returns to the grid.
The viewer offers fit and 100% of the decoded preview, capped at 4096 pixels.
It also offers exposure from -5.00 to +5.00 EV, a view-only **Before** toggle,
reset, explicit sidecar save, and profiled JPEG export. Dragging the exposure
slider uses a 1024-pixel preview. Releasing it renders the 4096-pixel preview.

Crema writes only sidecars that use its own XMP schema. An existing foreign or
unrecognized XMP file stays unchanged and makes sidecar save read-only. The
editor still keeps unsaved changes in memory and shows both states. Export writes
`<original complete filename>-crema.jpg` beside the source and refuses to replace
an existing path.

The probe writes a TSV record for every attempted decode and continues after errors.
Add `--fail-on-decode-error` to return status 2 when any decode fails. `--output`
creates a new report file and refuses to overwrite an existing file.

Color rendering is experimental. Embedded ICC profiles are reported but not
applied, HEIC HDR is not tone-mapped, and display profiles are not managed.
RAW previews use Rawler's baseline development. Successful pixels do not establish
camera-mode coverage or rendering quality. Exposure works on the decoder's
display-ready 8-bit output under an sRGB assumption. Preview and export use the
same exposure renderer. JPEG exports include an sRGB profile and have a maximum
long edge of 4096 pixels. This is not a scene-linear or full-resolution workflow.

Crash-safe sidecar and export publication has real-filesystem coverage on macOS.
Windows source identity and durable folder publication still need implementation
and validation. The app waits for editing workers during normal exit. Closing a
window with unsaved changes asks whether to keep editing or discard the changes.

See the [decode prototype reference](docs/decode-prototype.md) for bounds and
verification, and the [initial plan](docs/initial-plan.md) for the wider product scope.

Crema's original code is licensed under the [MIT License](LICENSE).
Third-party dependencies retain their own licenses.
