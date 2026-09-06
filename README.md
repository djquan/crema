# Crema

A planned desktop RAW photo manager and non-destructive editor for macOS,
Windows, and Linux, built with Rust.

Browse existing folders, organize photographs, and keep metadata and editing
instructions in XMP sidecars beside the originals.

Broad RAW, HEIC, and JPEG support is a core goal, with Fujifilm RAF and
OM System/Olympus ORF prioritized for initial validation.

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

The probe writes a TSV record for every attempted decode and continues after errors.
Add `--fail-on-decode-error` to return status 2 when any decode fails. `--output`
creates a new report file and refuses to overwrite an existing file.

Color rendering is experimental. Embedded ICC profiles are reported but not
applied, HEIC HDR is not tone-mapped, and display profiles are not managed.
RAW previews use Rawler's baseline development. Successful pixels do not establish
camera-mode coverage or rendering quality. XMP, edits, and export are not implemented.

See the [decode prototype reference](docs/decode-prototype.md) for bounds and
verification, and the [initial plan](docs/initial-plan.md) for the wider product scope.

Crema's original code is licensed under the [MIT License](LICENSE).
Third-party dependencies retain their own licenses.
