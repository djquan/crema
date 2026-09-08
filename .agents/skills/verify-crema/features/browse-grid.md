# Browse grid

## Sub-features

- Progressive population without a blank-window stall
- Virtualized scrolling through a large library
- Visible selection and keyboard focus
- Thumbnail names and unavailable states in the accessibility tree

## How to get to it (user POV)

Launch Crema with a prepared photo folder. The grid is the initial view.

## Driving it with Computer Use

Fetch fresh app state. Locate thumbnails by accessible name or `thumbnail-<asset-id>` author ID. Click a thumbnail, press Left and Right, and scroll far enough to recycle rows. Confirm keyboard selection is revealed and has a visible focus ring. Capture the state and a screenshot.

## Gotchas

Scanning and decoding are progressive, so wait for the specific thumbnail state instead of sleeping blindly. A populated accessibility tree does not prove VoiceOver speech. Large-grid frame targets require the 10,000-item release-build run.
