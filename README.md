# Crema

A planned desktop RAW photo manager and non-destructive editor for macOS,
Windows, and Linux, built with Rust.

Browse existing folders, organize photographs, and keep metadata and editing
instructions in XMP sidecars beside the originals.

Broad RAW, HEIC, and JPEG support is a core goal, with Fujifilm RAF and
OM System/Olympus ORF prioritized for initial validation.

The first implementation slice scans a real folder and reports likely photo files
without changing them. See the [initial plan](docs/initial-plan.md) for the proposed
technology choices, design direction, architecture, and first milestones. External
dependencies have not yet been selected for installation.

Crema's original code is licensed under the [MIT License](LICENSE).
Third-party dependencies retain their own licenses.
