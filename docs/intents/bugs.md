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

## Mixer and routing

None known.

## Timeline and clips

None known.

## Recording and hardware

None known.

## Plugins and the CLAP host

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-76 | plugins-instruments | The shared slider widget clips its thumb to its own rect (`painter_at(rect)`), so the thumb is half cut off at either end of its travel in every plugin editor. | `plugin-gui-core/src/widgets/slider.rs` | BUG-41 fix 2026-10-15 |
| BUG-77 | plugins-instruments | Test infra: `wayland-plugin-gui --test editor_size` `a_compositor_resize_is_reported_by_the_handle` fails with "could not drive the compositor": Hyprland 0.56 replaced `hyprctl dispatch resizewindowpixel` with Lua dispatch (`hl.dsp.window.resize({...})`). | `wayland-plugin-gui/tests/editor_size.rs` | BUG-41 fix 2026-10-15 |

## Compose and vocals

None known.

## Project, save and export

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-75 | audio-engine | A frozen drum kit's tap sends are silent: the cache is the summed taps, so there is no per-tap signal left to send, and a kit with taps on a room bus loses the room while frozen. A send on the kit's parent track also taps the whole cache when frozen, but only port 0 when unfrozen. | freeze cache (`audio/engine/bounce/freeze.rs`); documented in [track-freeze](../features/audio-engine/track-freeze.md) | BUG-67 fix 2026-10-15 |
