# Complete the Phase 0 fixes and acceptance proof

## Repository repair status

The September 8 repository findings F1 through F8 are implemented and covered by
focused regression tests. Fresh native evidence must still be collected from this
source identity. Windows and Linux execution, representative camera files, display
profiles, assistive technologies, declared performance hardware, and independent
color references remain acceptance inputs rather than inferred passes.

Keep Phase 0 open. Start from the [September 8 review](reviews/2026-09-08-phase0-review.md) of commit `5d4f178b14538ec0f57464d969b863a9c03a0511`. Recheck the current diff before editing because later work may supersede a finding.

Implement the units below in order. Each unit ends with a behavioral check. Do not mark the phase complete because the implementation compiles or because a missing capability is safely rejected.

## 1. Protect sidecar association and HEIC color

Fix F1 first in `crema-core/src/sidecar.rs` and the app's document-open path. Model sidecar association as unique, explicitly associated, or ambiguous. Block ambiguous writes and retain the unsaved recipe. Recheck competing originals before publication. Do not guess association from a basename or automatically rename existing sidecars.

Add real-filesystem tests for these cases:

- Two proprietary RAWs share a stem before open.
- A competing RAW appears after open but before save.
- A RAW/JPEG pair retains separate recipes.
- Existing supported naming variants conflict.
- A sidecar changes externally while an edit remains open.

Then fix F3 in `crema-image/src/decode.rs`. Define the complete supported NCLX descriptions at the decoder boundary. Reject unverified primaries and matrices. Add actual HEIC worker tests for unsupported tags and numerical references for each accepted conversion. Recheck baseline JPEG and accepted SDR HEIC appearance.

Run the touched core/image tests and the actual decoder probe. Retain source hashes before and after each filesystem/decoder scenario. Do not add a codec dependency without researching its fit and obtaining the already-required dependency decision.

## 2. Stabilize the displayed edit

Fix F4 in `crema-app/src/browser.rs`. Separate the current render request from the last accepted image eligible for display. Retain the latter while the next result is pending. Continue rejecting results for old revisions, source epochs, and assets.

Add a test against actual egui output that starts with an accepted nonzero edit, requests another edit, and verifies the displayed texture before the new result arrives. Cover Before, reset, source replacement, navigation, and cache eviction. Drive the release slider repeatedly on a large photo and preserve the frame/interaction evidence.

## 3. Implement Windows persistence

Fix F2 across `crema-app/src/platform.rs`, `crema-core/src/sidecar.rs`, and export publication. Define stable opened-file identity using native Windows file information. Preserve the distinction between identity, content change, and the current pathname.

Implement create-only publication and atomic replacement with explicit durability outcomes. Do not claim a universal cross-application compare-and-swap guarantee. Keep collisions, changed sources, foreign sidecars, and denied writes visible to the user.

Run the save/export behavior tests on native Windows. Keep platform guards only for platform-specific setup or assertions. Exercise these cases on a real filesystem:

- Save, normal exit, restart, and recipe restoration.
- Original replacement or rename while work is pending.
- External sidecar modification and export collisions.
- Read-only directories, Unicode paths, and case behavior.
- Interrupted publication, restart recovery, and worker shutdown.
- Persistent-thumbnail reuse across processes.

This unit needs a Windows machine or native runner. A Mac cross-build cannot close it. Confirm whether the chosen implementation fits existing dependencies before adding one.

## 4. Make evidence acceptance trustworthy

Fix F5–F8 before collecting final platform receipts.

Define one versioned GUI receipt contract. Bind the source identity, app binary, host, original hashes, successful observations, metrics, sidecars, exports, and session IDs. Validate both required fields and referenced artifacts in `verify-phase0.py`. Keep display-color and screen-reader capabilities separate from ordinary GUI navigation.

Give each app launch a new create-only metrics destination. Record an explicit second-process document-open/recipe-restored event linked to the first session's sidecar. Require both normal exits. Do not overwrite the first session's evidence or accept `save_finished` alone as proof of restart restoration.

Own benchmark thresholds in the assessor. Reject a caller budget that weakens the declared target. Validate measurement kind, counts, cache state, traversal, source/binary identity, and raw metric evidence. Keep GUI CPU work separate from presentation timing.

Add these boundary tests using real temporary files and the actual validator:

- A source-only GUI receipt is rejected.
- Missing, malformed, mismatched, or failed capability evidence is rejected.
- A missing restart session, wrong restored recipe, or unclean exit is rejected.
- A caller-selected one-second frame budget is rejected.
- The fixed 16,667 µs boundary passes and 16,668 µs fails for GUI work.
- A valid producer output survives the consumer's validation.

Run `python3 -B -m unittest discover -s scripts -p 'test_*.py'` in every native CI job. Add a required Python-test row to the local evidence command and the release bundle contract. Configure Python explicitly. Replace the unconditional dependency-plan pass with a reference to a reviewed source-bound decision record, or report that review as missing.

## 5. Close the camera and color feasibility questions

Collect representative files outside the repository. Record permission/provenance, hash, exact model, sensor layout, capture mode, compression, dimensions, bit depth, orientation, and expected color description. Start a matrix with separate decode, metadata, development, preview, edit/reopen, and export outcomes.

Prioritize representative Fuji files, including X-Trans where applicable, and ordinary OM captures. Track OM high-resolution modes, broader RAW brands, high-bit/HDR HEIC, embedded ICC, grids, rotation, mirroring, and JPEG profile/orientation variants in the matrix. Use samples to measure feasibility. Do not require every matrix row to pass Phase 0. High-resolution ORF becomes part of the representative Phase 0 proof only when selected to represent the intended camera workflow. Full mode/variant coverage closes in Phase 4.

Compare camera development to independently established references. Evaluate X-Trans detail, false color, white balance, missing calibration, and highlights. Evaluate whether Rawler's baseline and the current RGBA8 exposure path satisfy the feasibility goal. Record any pipeline change needed to preserve scene-linear precision. An expected render generated by the same implementation is not an independent reference.

Measure unsupported HEIC paths and decide how the required precision/color conversion will be achieved. Establish representative SDR conversion and a viable path for the measured limitations. Record an unsupported sample as unsupported. Its measured limitation can inform the feasibility decision, but cannot count as successful conversion or full format support.

Review the actual pinned dependency features, transitive native processing code, and license fit. Preserve Crema's MIT license. Leave final distribution packaging to Phase 4.

## 6. Measure native interaction, display, accessibility, and memory

Use `.agents/skills/verify-crema/SKILL.md` for the Mac UI workflow after repairing its session evidence. Produce equivalent source-bound Windows and Linux evidence. Exercise both Wayland and X11. Record hardware, GPU, display refresh, scaling, OS, build mode, fixture sizes, cache state, and sample count.

Implement the missing next-photo, exposure, and memory evidence producers. Measure the initial plan's targets:

| Measurement | Required evidence |
| --- | --- |
| Cached grid, 10,000 assets | Declared 60 fps target with a clearly defined frame/presentation measure. Keep the 16,667 µs GUI-work statistic as a separate supporting metric. |
| Cached next-photo | p50/p95 and sample count, with p95 below 100 ms. Exclude first open from transition statistics. |
| Exposure on prepared 2 MP preview | Input-to-visible-result p50/p95 with p95 below 50 ms. The existing 1024-edge drag path does not establish this workload. |
| Memory, 100,000 assets | CPU, GPU, cache, and simultaneous parent/worker measurements, explicit budgets, and bounded growth over repeated navigation. |
| Idle | No continuous repaint or full-folder rescans after work settles. |

Measure decode allocation separately from output size. Account for HEIF threads that do not use Rayon. Include large/high-resolution captures and cancellation. A process deadline and a pixel check after allocation do not impose a memory ceiling.

Run VoiceOver, NVDA, and Orca against real app actions. Verify focus, useful names, slider operation, dialogs, and keyboard-only navigation. A populated AccessKit tree is supporting evidence only.

Validate the active display/compositor behavior with independent color evidence. Exercise profile changes and moving between displays where available. Document and test the explicit sRGB fallback where integration is unavailable. Do not certify wide-gamut correctness from an embedded JPEG profile.

## 7. Publish the completion verdict

After the fixes, generate fresh evidence from one source identity. Do not reuse the pre-fix receipts as proof of the changed implementation. Run the touched tests during each fix, then the complete Phase 0 workflow when assembling final evidence.

Use new output directories for every run:

```sh
python3 scripts/verify-phase0.py mac-local /tmp/crema-phase0-final-mac /path/to/photo.jpg /path/to/photo.heic /path/to/photo.raf /path/to/photo.orf
python3 scripts/macos-phase0-app.py prepare /tmp/crema-phase0-final-mac
python3 scripts/macos-phase0-app.py doctor /tmp/crema-phase0-final-mac
```

Drive the app, then finish with the repaired session workflow. Assemble native bundles only after their behavior has been observed:

```sh
python3 scripts/verify-phase0.py release /tmp/crema-phase0-final-release /path/to/macos-bundle /path/to/windows-bundle /path/to/linux-wayland-bundle /path/to/linux-x11-bundle
```

Update `docs/phase0-status.md` against every row of the review's acceptance matrix. Mark complete only when F1–F8 are resolved and every Phase 0 requirement has current, inspected evidence. Expand the assessor where it currently lacks camera-quality or other required acceptance checks. A green incomplete assessor is insufficient.

If files, hardware, or independent references remain unavailable, publish the exact blocked rows and retain the open verdict. Required external inputs are representative camera files/modes, native Windows/Linux environments, declared performance hardware, assistive-technology sessions, and color references. None was inferred from this Mac review.

The September 8 review passed 75 Rust tests, 11 Python tests, formatting, linting, release builds, and a real Mac JPEG edit/save/export/reopen session. It reproduced four behavioral defects and two false individual gate passes. Use those results as the starting point, not as acceptance for future changes.
