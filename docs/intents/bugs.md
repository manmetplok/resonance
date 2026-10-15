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

## Decided fixes (triage 2026-10-15)

The triage re-checked every row against `ef215b08` and settled how to
fix the ones below. The engine-path defects it judged to be design work
(the former BUG-21, 30, 32, 33, 34, 35 and 37) moved to the intent
[audio-engine/live-path-robustness.md](audio-engine/live-path-robustness.md).

| ID | Decided fix |
|---|---|
| BUG-39, BUG-61 | Request ids on `SavePluginState` / `SaveAllPluginStates` / `SaveClipsToProjectDir` and their replies; collectors ignore replies whose id doesn't match. |
| BUG-51 | Wire swing and humanize at render time, with jitter seeded from the group seed; the default humanize becomes 0. Fills stay in the intent. |
| BUG-60 | Save As copies the WAVs that only the undo history names from the old `audio/` into the new one. |
| BUG-62 | When Save As targets a different existing project, confirm first, then clear its `backups/`, autosave and old media. |
| BUG-72 | Key-aware spelling: `Scale::prefers_flats`, `Chord::display_in(scale)` at the four display sites. |
| BUG-73 | Placing a section installs its derived parts and the existing vocal take (the status half is fixed separately). |

## Mixer and routing

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|

## Timeline and clips

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|

## Recording and hardware

None known.

## Plugins and the CLAP host

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-39 | plugins-instruments | *(likely)* Edge cases: a drums `kit_select` undo races a cache refresh; an in-flight `SavePluginState` is counted as an owed echo. | plugin-state echo bookkeeping | review 2026-10-02 FU-P1c |
| BUG-76 | plugins-instruments | The shared slider widget clips its thumb to its own rect (`painter_at(rect)`), so the thumb is half cut off at either end of its travel in every plugin editor. | `plugin-gui-core/src/widgets/slider.rs` | BUG-41 fix 2026-10-15 |
| BUG-77 | plugins-instruments | Test infra: `wayland-plugin-gui --test editor_size` `a_compositor_resize_is_reported_by_the_handle` fails with "could not drive the compositor": Hyprland 0.56 replaced `hyprctl dispatch resizewindowpixel` with Lua dispatch (`hl.dsp.window.resize({...})`). | `wayland-plugin-gui/tests/editor_size.rs` | BUG-41 fix 2026-10-15 |

## Compose and vocals

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-72 | compose | Chord symbols are spelled with sharps regardless of key: `harmony_apply_progression` with "Bbmaj7" in D minor reads back as "A#maj7" in `song_sections` (and the chord track). | chord symbol formatting in `resonance-music-theory` | live MCP session 2026-10-13 |
| BUG-73 | compose | Placing a section that already has a rendered vocal lane gives the new placement no vocal take (the lane now reads `stale` until a `vocal.render`). Same root as the empty generated parts on a new placement ([section-placement-editing](compose/section-placement-editing.md)). | `app/update/compose/section.rs` `handle_place` (installs nothing) | live MCP session 2026-10-13 (triphop.rproj) |
| BUG-51 | compose | Drum group Swing, Humanize and "Fills (last bar)" sliders change nothing in the generated drums. | sliders `app/view/compose/lane_inspector/drums/rhythm.rs:44-97`; stored at `app/update/compose/drum_groups.rs:315-324`; never read | intent [compose/drum-groove-feel.md](compose/drum-groove-feel.md) |

## Project, save and export

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-60 | project-core | Save As does not copy clip WAVs that only the undo history references; undoing back to them in the new project plays silence. | `app/update/project_io/mod.rs:660-672` (engine saves live clips only) | review 2026-10-02 FU-U1a |
| BUG-61 | project-core | Save replies are untagged (`SaveCollector` has no round-trip id), so a late reply from an earlier save can complete a newer one. | `app/update/project_io/mod.rs:661-672` | review 2026-10-02 FU-S2c |
| BUG-62 | project-core | *(likely)* Save As over another existing `.rproj` leaves that project's `backups/` behind, and clip GC keeps their clips alive. | `app/project/clip_gc.rs:102` seeds the keep-set from `backups/`; nothing clears it on Save As | review 2026-10-02 STATE2-10 |
| BUG-75 | audio-engine | A frozen drum kit's tap sends are silent: the cache is the summed taps, so there is no per-tap signal left to send, and a kit with taps on a room bus loses the room while frozen. A send on the kit's parent track also taps the whole cache when frozen, but only port 0 when unfrozen. | freeze cache (`audio/engine/bounce/freeze.rs`); documented in [track-freeze](../features/audio-engine/track-freeze.md) | BUG-67 fix 2026-10-15 |
