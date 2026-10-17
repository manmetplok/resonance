# Known defects

Behaviour defects that exist in the code today (checked against master
`ef215b08`, 2026-10-15). Each row is a real, user- or agent-visible fault —
wrong sound, lost data, a control that does nothing, a security leak — not a
missing feature. Missing features are intents in the area folders; when a bug
is part of a larger intent, the Source column links it.

Rows marked *(likely)* were traced by reading code after their fix batch
merged, but not reproduced. Fix any row the usual way: write the failing test
first. When a bug is fixed, delete its row.

Paths: `app/` = `resonance-app/src/`, `audio/` = `resonance-audio/src/`.

## Triage 2026-10-15

Every row was re-checked against `ef215b08`; all fixable ones were fixed the same day.
The live-path engine defects that need realtime design work (the former
BUG-21, 30, 32, 33, 34, 35 and 37) are the intent
[audio-engine/live-path-robustness.md](audio-engine/live-path-robustness.md).

| Area | Bug | Where | Source |
|---|---|---|---|
| plugins-instruments | BUG-76: The shared slider widget clips its thumb to its own rect (`painter_at(rect)`), so the thumb is half cut off at either end of its travel in every plugin editor. | `plugin-gui-core/src/widgets/slider.rs` | BUG-41 fix 2026-10-15 |
| audio-engine | BUG-75: A frozen drum kit's tap sends are silent: the cache is the summed taps, so there is no per-tap signal left to send, and a kit with taps on a room bus loses the room while frozen. A send on the kit's parent track also taps the whole cache when frozen, but only port 0 when unfrozen. | freeze cache (`audio/engine/bounce/freeze.rs`); documented in [track-freeze](../features/audio-engine/track-freeze.md) | BUG-67 fix 2026-10-15 |
| project-core | BUG-78: Test hygiene: test runs leave their temp dirs in `/tmp` and never delete them. About 90,000 had piled up (`resonance_crash_detection_*`, `resonance_autosave_recovery_*`, `resonance-hermetic-*`, `resonance-backup-test-*`, `resonance-mcp-test-*`, export-encoder dirs and others) until the tmpfs hit "Disk quota exceeded" and tests failed on writes. Fix: use `tempfile::TempDir`, or a drop guard, so each dir is removed when its test ends; for the per-process hermetic root, remove it at process exit. | `resonance-app/src/user_dirs.rs:25` (hermetic root per pid); `resonance-app/tests/io/crash_detection.rs:22`; `resonance-mcp/tests/common/mod.rs:36`; others, found by the prefixes | dependency upgrade 2026-10-15 |
