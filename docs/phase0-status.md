# Phase 0 status

Phase 0 implements the full feasibility path, but the release gate remains open.
Crema reports missing evidence as blocked instead of treating an unavailable
camera file, operating system, display, or screen reader as a skipped success.

The application defects found in the September 8 review have regression coverage
in the current source. Fresh macOS functional evidence is still required because
the earlier GUI receipt predates those repairs. The other hardware- and
fixture-dependent rows below also remain open.

Run the local evidence workflow into a new directory outside the repository:

```sh
python3 scripts/verify-phase0.py mac-local /tmp/crema-phase0 \
  /path/to/photo.jpg /path/to/photo.heic /path/to/photo.raf /path/to/photo.orf
python3 scripts/macos-phase0-app.py prepare /tmp/crema-phase0
```

The first command runs formatting, linting, tests, release builds, decoder probes,
original-file hash checks, and dependency-license inventory. It exits 0 for a
complete pass, 1 for a failed check, and 2 when required evidence is blocked. The
macOS helper builds and signs a real app bundle for the repository-local
`verify-crema` skill. It requires corroborating GUI metrics and observations before
publishing `gui-receipt.json`.

Release assessment consumes native evidence bundles from the same source identity:

```sh
python3 scripts/verify-phase0.py release /tmp/crema-phase0-release \
  /path/to/macos-evidence /path/to/windows-evidence /path/to/linux-evidence
```

## Implemented and directly tested

- Progressive folder scanning, virtualized thumbnail rows, shared grid/viewer
  selection, bounded preview scheduling, cache budgets, and idle-no-repaint logic.
- Keyboard navigation that reveals the selected virtual row, visible focus, stable
  accessibility author IDs, named viewer images, and dirty-close behavior.
- JPEG, RAW, and HEIC routes with worker bounds and typed failures. Embedded JPEG
  ICC profiles convert to sRGB. Unsupported HEIC precision and color paths fail
  closed.
- Non-destructive exposure preview, Crema-owned XMP save/reopen, profiled JPEG
  export, original-file integrity checks, and real-filesystem publication tests.
- Schema-2 sidecars bind edits to one original and block ambiguous same-stem RAW
  ownership, including conflicts introduced immediately before publication.
- Windows opened-file identity and cross-platform sidecar publication. The same
  save, export, replacement, collision, and original-integrity behavior tests are
  enabled on every supported operating system.
- A pinned Rust 1.95 contract and native macOS, Windows, and Linux CI jobs.
- One create-only evidence command, deterministic profiled JPEG generator, real
  macOS app-bundle helper, source-bound performance receipts, and project-local UI
  verification skill.

## Not verified

- Representative Fuji RAF and OM/Olympus ORF files, camera modes, X-Trans detail,
  and photographic development quality. Fuji-specific tuning remains deferred.
- Real SDR, high-bit, HDR, and embedded-profile HEIC fixtures. Unsupported variants
  are safely rejected, but conversion feasibility is not established.
- Active-display color presentation or perceptual/numeric golden-image accuracy.
- Native VoiceOver, NVDA, and Orca behavior. AccessKit roles, names, author IDs,
  keyboard focus, and visible focus are implemented, not equivalent to a screen-
  reader session.
- Fresh macOS functional GUI evidence, native Windows and Linux GUI behavior,
  Linux Wayland and X11, and real-machine GPU/display behavior. The current native
  Windows CI run covers filesystem behavior, but not the GUI.
- The pre-repair 10,000-item Mac run is historical only. Fresh warm-cache metrics
  must use a unique launch log and pass the fixed 16.667 ms assessor budget after
  raw-log recomputation. Monitor presentation timing, cached next-photo latency,
  exposure-response latency, and bounded memory growth on declared release
  hardware remain explicit release blockers. No producer exists for those three
  receipts yet.

## Dependency and scope decisions

The dependency set remains pinned in `Cargo.lock`. A source-bound reviewed
dependency approval record is still required; the verifier reports that row as
blocked instead of inferring approval from lockfile metadata. Crema's owned code
remains MIT, while dependencies retain their licenses. Process isolation limits
decoder failures but does not change Rawler's LGPL obligations.
Final distributable notices, source offer, and relinking artifacts belong to the
release-hardening phase rather than this feasibility gate.

Crema writes and reopens only its own XMP schema. Foreign XMP import is outside this
phase by decision; a later importer can translate other dialects without weakening
the owned sidecar boundary.
