# Close and reopen

## Sub-features

- Dirty close offers Keep editing and Discard and close
- Keep editing preserves the session
- Normal close waits for workers and publishes metrics
- Saved edit survives a fresh process

## How to get to it (user POV)

Create an unsaved exposure change, then close the window.

## Driving it with Computer Use

Trigger the native close action. Fetch fresh state and verify the confirmation text and both choices. Choose Keep editing once and confirm the viewer remains. Close again, choose Discard and close, and wait for the process to exit. For persistence, relaunch after a saved edit and confirm the saved exposure is restored.

## Gotchas

Do not force-kill the app during the normal-close proof. The metrics file is create-only and is written on exit. Native VoiceOver, NVDA, and Orca remain separate acceptance evidence even when the AccessKit tree is labeled.
