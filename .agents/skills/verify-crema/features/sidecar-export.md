# Sidecar and export

## Sub-features

- Save writes Crema-owned XMP without modifying the original
- Reopen restores the exposure recipe
- Export creates a new profiled JPEG
- Existing export paths are never replaced

## How to get to it (user POV)

Make a nonzero exposure edit in the viewer. Choose Save XMP, then Export JPEG.

## Driving it with Computer Use

Activate Save XMP and wait for the saved state. Activate Export JPEG and wait for its completed path. Confirm the sidecar and export exist outside the app with the finish helper. Close and relaunch the same prepared app, reopen the photo, and confirm the saved exposure value returns. Capture the restored control value.

## Gotchas

Only Crema-owned XMP is writable. A foreign, malformed, or newer packet must remain unchanged and read-only. The exported JPEG must contain its ICC APP2 payload. A successful button press without the matching metrics and files is not a pass.
