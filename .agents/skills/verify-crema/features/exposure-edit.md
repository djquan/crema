# Exposure edit

## Sub-features

- Exposure slider changes the preview
- Drag uses the interactive render, release settles to detail
- Before is view-only
- Reset restores zero without saving

## How to get to it (user POV)

Open a photo in the viewer. Use the Exposure control, Before, and Reset.

## Driving it with Computer Use

Read the current Exposure value, change it to a clearly nonzero value, and wait for fresh state. Capture before and after screenshots. Hold Before and confirm the original view appears, then release it and confirm the edit returns. Use Reset and verify the control returns to 0.00 EV. Repeat the edit before continuing to sidecar verification.

## Gotchas

Do not infer a successful render from the slider value alone. The finished evidence must contain `edit_render_received`. Screenshots prove visible change, not numeric color accuracy or display-profile handling.
