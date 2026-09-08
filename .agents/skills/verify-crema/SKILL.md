---
name: verify-crema
description: Drive Crema's real macOS UI and create Phase 0 evidence for browsing, editing, sidecar persistence, export, close behavior, accessibility, and performance.
---

# Verify Crema

Use the real release application. Do not replace user actions with calls into Rust internals.

## Prepare

1. Create a new evidence bundle outside the repository:
   `python3 scripts/verify-phase0.py mac-local /tmp/crema-phase0-<run>`
2. Prepare the signed app and deterministic photos:
   `python3 scripts/macos-phase0-app.py prepare /tmp/crema-phase0-<run>`
3. Run the doctor and use the reported `Crema.app` path:
   `python3 scripts/macos-phase0-app.py doctor /tmp/crema-phase0-<run>`

The Phase 0 command normally exits 2 because missing private camera fixtures and native-platform evidence are blockers. The bundle must still exist and must contain no failed checks before UI verification continues.

## Launch

Use Computer Use through the persistent Node REPL and `@oai/sky`. Launch `Crema.app` directly so Launch Services applies the bundle's fixture, cache, and metrics environment. After every action, fetch fresh application state. Prefer accessible role, name, and author ID. Use coordinates only when the accessibility tree is incomplete, and record that limitation.

## Drive

Read the matching file under `features/` before driving a feature. Cover every feature in one session:

- `browse-grid.md`
- `viewer-navigation.md`
- `exposure-edit.md`
- `sidecar-export.md`
- `close-reopen.md`

Save at least one screenshot for each feature under the evidence bundle's `macos-app/screenshots/` directory. Never place private photo pixels in the repository.

After proving a feature, record it with the helper. For example:
`python3 scripts/macos-phase0-app.py observe /tmp/crema-phase0-<run> navigation --detail "keyboard selection remained visible"`

## Finish

1. Close Crema through its UI so `gui_exit` and the metrics file are published.
2. Run `python3 scripts/macos-phase0-app.py finish /tmp/crema-phase0-<run>`.
3. Inspect `gui-receipt.json`, `macos-app/gui-metrics.tsv`, screenshots, XMP, and exported JPEG.
4. Run `python3 scripts/macos-phase0-app.py cleanup /tmp/crema-phase0-<run>` only after inspection. Cleanup removes the disposable app and cache while preserving raw evidence.

If any action, artifact, metric, or native capability is absent, report it as NOT VERIFIED. Do not convert visual plausibility into a pass for color, performance, or assistive technology.

## Large-grid performance

Generate a create-only 10,000-item folder with:
`cargo run --release -p crema-app --bin crema-ui-fixture -- --count 10000 /tmp/crema-grid-10000`

Prepare and launch against that folder. Scroll enough rows to populate the cache,
close normally, and preserve the first metrics file as `gui-metrics-cold.tsv`.
Launch again, wait for `10000 photos`, scroll at least 20 three-page steps, and
close normally. Capture hardware, display refresh rate, frame metrics, cache state,
and sample count. A UI draw call is not proof that the monitor presented the frame.

After a warm-cache run closes normally, run
`python3 scripts/macos-phase0-app.py performance /tmp/crema-phase0-<run>`.
Inspect `performance-receipt.json`. It must cover at least 10,000 indexed photos,
contain only cache hits for attempted previews, traverse at least 500 items across
20 visible grid positions, include at least 100 measured frames, and keep p95 GUI
work within the 16.667 ms budget. The receipt does not claim monitor presentation
timing, next-photo latency, exposure latency, or bounded memory.
