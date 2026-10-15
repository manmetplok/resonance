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
| BUG-02 | Build it the way [mixer/sub-track-solo.md](mixer/sub-track-solo.md) describes: sub-track solo counts in the total, and the parent becomes a fan-out driver. |
| BUG-03 | Offset = live mix integrated LUFS − reference integrated LUFS, handed to the engine thread without a lock; −inf falls back to 0. |
| BUG-04 | Bus levels are longest-path levels over the send graph, and sends are pulled by level instead of index. Keep the source-index summing order. |
| BUG-20 | On a seek or loop-range change during cycle-record, roll the current audio and MIDI pass, re-arm the start latch with the new loop, and update the session slot. |
| BUG-22 | The seam bumps a wrap-count atomic, and live MIDI events are filed into passes by the wrap count they arrived under (this also stops a backward seek from rolling a MIDI pass). |
| BUG-23 | A monotonic drop total that is never reset; the drain writes the delta as silence and adds it to the cut clock. |
| BUG-39, BUG-61 | Request ids on `SavePluginState` / `SaveAllPluginStates` / `SaveClipsToProjectDir` and their replies; collectors ignore replies whose id doesn't match. |
| BUG-41 | Taller strip (about 440 px) and window, a horizontal scroll over the bands, and a header that wraps. |
| BUG-42 | A phase-matched dry path (complementary split) for Transformer saturation; re-bless the mastering goldens. |
| BUG-51 | Wire swing and humanize at render time, with jitter seeded from the group seed; the default humanize becomes 0. Fills stay in the intent. |
| BUG-60 | Save As copies the WAVs that only the undo history names from the old `audio/` into the new one. |
| BUG-62 | When Save As targets a different existing project, confirm first, then clear its `backups/`, autosave and old media. |
| BUG-72 | Key-aware spelling: `Scale::prefers_flats`, `Chord::display_in(scale)` at the four display sites. |
| BUG-73 | Placing a section installs its derived parts and the existing vocal take (the status half is fixed separately). |

## Mixer and routing

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-02 | mixer | The S button on a sub-track strip lights up, saves and undoes, but changes nothing audible. | `app/view/mixer/track_strip.rs:662-666`; `audio/mixer/render/strategy.rs:56` (`sub_track_silenced` ignores solo); `audio/types/track.rs:611` (`any_top_level_solo` skips sub-tracks) | intent [mixer/sub-track-solo.md](mixer/sub-track-solo.md) |
| BUG-03 | mixer | The reference loudness-match toggle has no audible effect; its readout is always +0.0 dB. | `audio/engine/reference.rs:73` (`offset_db`), only ever set to 0.0 at `:88` | intent [mixer/reference-loudness-match.md](mixer/reference-loudness-match.md) |
| BUG-04 | mixer | A bus→bus send into a lower-indexed (older) bus passes validation and is saved, but is silent. | `audio/mixer/render/bus_pass.rs:152` (`dst_idx > src_idx`); `audio/mixer/render/routing.rs:123-133` | intent [mixer/bus-to-bus-routing.md](mixer/bus-to-bus-routing.md) |

## Timeline and clips

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|

## Recording and hardware

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-20 | hardware-io | Seeking or moving the loop range during cycle-record is ignored by the take session; later passes are cut at stale seams. | `audio/engine/transport.rs:672` (`handle_seek_to` ignores `loop_record_session`) | review 2026-10-02 FU-R1b |
| BUG-22 | hardware-io | MIDI cycle-record passes roll on the engine-thread playhead wrap, so pass boundaries jitter by ~16 ms (audio passes are sample-exact). | `audio/engine/transport.rs:804-818` | review 2026-10-02 FU-R1f |
| BUG-23 | hardware-io | Input-ring overflow drops are reported but not compensated, so every later cycle cut shifts by the dropped frames. | `audio/recording.rs:397-418` | review 2026-10-02 FU-R1a |

## Plugins and the CLAP host

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-39 | plugins-instruments | *(likely)* Edge cases: a drums `kit_select` undo races a cache refresh; an in-flight `SavePluginState` is counted as an owed echo. | plugin-state echo bookkeeping | review 2026-10-02 FU-P1c |
| BUG-41 | plugins-effects | *(likely)* EQ dynamics rows overflow the 350 px strip and the header runs off-window at ≤960 px. | `plugins/resonance-eq/src/editor/app.rs:174`; `factory.rs:70` | review 2026-10-02 FU-P2c |
| BUG-42 | plugins-effects | Mastering's Transformer saturation at partial `sat_mix` dips the sub band (−2.3 dB at 20 Hz); documented, not fixed. | `plugins/resonance-mastering/src/stages/sat_modes.rs:80-92` | review 2026-10-02 FU-D1a2 (DSP2-16 remainder) |

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
