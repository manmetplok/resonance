# Reference tracks and A/B monitoring

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: design `docs/intents/prototypes/reference-track-ab-monitoring/DESIGN.md` (epic #43); `reference.load` from warmth-width-depth.md §7.5.

A user loads one or more finished, mastered tracks into a right-hand rail
of the Mix view and flips the monitor between their own mix (A) and a
reference (B), reading a Mix / Ref / Δ loudness table side by side. The
reference plays through the monitor only: it bypasses the master chain and
never reaches a render, bounce, stem or recording. An agent adds a
reference with `reference.load` and measures it with `meter.measure`.

## Behaviour

- **Panel.** The `REF` chrome toggle opens a 360 px rail right of the
  inspector (`MixerUiState::reference_panel_open`, runtime only). States:
  empty (drop zone, "Add reference…"), analysing (checklist: decoding,
  measuring loudness, building waveform, matching loudness; Cancel removes
  the entry), populated, and an inline error card per failed or missing
  entry.
- **Loading.** Drop an audio file anywhere on the window while the Mix
  view is showing (the rail need not be open; other views ignore drops for
  this purpose), or pick one (`wav`, `flac`, `mp3`, `ogg`;
  `REFERENCE_AUDIO_EXTENSIONS`). The engine registers the entry at once,
  then a worker thread decodes it to the engine rate, measures integrated
  LUFS (BS.1770, `LufsMeter`) and builds a 1000-pair waveform overview.
- **List.** Each loaded entry shows name and integrated LUFS and can be
  selected (active) or removed. Several references can be loaded; A/B
  auditions the active one.
- **A/B.** A two-segment control selects Mix or Reference. Holding **B**
  (bare key, focus-gated; ⌘B still bounces) auditions the reference and
  restores the previous source on release. The subscription is only
  attached while the rail is open with a reference selected.
- **Monitor path.** When the source is the reference, the callback
  replaces the whole output with reference PCM after the master stage
  (`callback/reference.rs::monitor_reference`). It is suppressed while
  recording or counting in. Offline render paths never read the reference,
  so exports stay the processed mix.
- **Timing.** In loop-to-mix mode the reference cursor follows the
  transport, delayed by the mix's total compensation latency plus master
  latency, so flipping A/B causes no timing jump. Otherwise the reference
  free-runs from its own cursor; the waveform scrubs it.
- **Level.** Monitor gain = (loudness-match offset if enabled) + manual
  trim. The trim slider works. **The loudness-match offset is never
  computed**: `ReferenceEntry::offset_db` stays 0, so turning match on
  changes nothing audible and the panel shows a 0 dB offset (see Known
  gaps).
- **Meters.** The mix is metered post-master; the reference is metered as
  monitored (after trim). The panel polls `PollABMeters` every tick while
  open with a reference loaded and shows integrated, short-term and
  momentary LUFS, true-peak max and LRA for Mix, Ref and Δ, plus a pair of
  bars against a −14 LUFS target line.
- **Markers.** Comparison markers at a reference position, add and remove,
  saved with the project. Marker ids are app-allocated.
- **Persistence and undo.** `ProjectFile::references` (path, name, cached
  loudness, markers) and `reference_settings` (active index, loudness
  match, trim, A/B source, loop-to-mix). Load, remove, set active,
  loudness-match toggle and trim are undoable (trim drags coalesce); A/B
  source, loop-to-mix, markers and scrub are not. A reference whose file
  is gone loads as Missing under an id from `MISSING_REFERENCE_ID_BASE` and
  is never sent to the engine. Engine ids are session-monotonic so a late
  echo for a dropped reference is recognised as stale.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Engine state, analysis | `resonance-audio/src/engine/reference.rs` | `ReferencePlayer`, `run_reference_analysis`, `handle_set_ref_loudness_match`, `REFERENCE_OVERVIEW_PEAKS` |
| Monitor tap | `resonance-audio/src/mixer/callback/reference.rs` | `monitor_reference` |
| Mix meter | `resonance-audio/src/mixer/callback/master_pass.rs` | `ab_meters.mix` |
| App state | `resonance-app/src/reference/state.rs` | `ReferenceState`, `ReferenceMonitorState`, `ReferenceEntry`, `AbMeters` |
| Messages + update | `resonance-app/src/reference/messages.rs`, `update/reference.rs` | `ReferenceMessage`, `REFERENCE_AUDIO_EXTENSIONS` |
| Key / drop subscriptions | `resonance-app/src/update.rs` | `MOMENTARY_AUDITION_KEY`, `reference_momentary_keys`, `reference_file_drop` |
| Echoes | `resonance-app/src/engine_events/reference.rs` | `loudness_match_changed`, `ab_meter_snapshot` |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectReference`, `ProjectReferenceMarker`, `ProjectReferenceSettings` |
| View | `resonance-app/src/view/mixer/reference_panel/` | `bodies.rs`, `ab_controls.rs`, `loudness_canvas.rs`, `waveform.rs` |
| Control | `resonance-app/src/update/control/reference_track.rs` | `reference.load` |

## Surfaces

- **GUI:** Mix tab, `REF` toggle; hold **B** to audition.
- **Control API / MCP:** `reference.load` → `mcp__resonance__reference_load`
  (takes a `pool_asset_id`, loads the pooled project-rate WAV, goes through
  the same undoable update path as a GUI load). Measuring a reference:
  `meter.measure` with `target: {"reference": N}` (see
  [meter-control-api.md](meter-control-api.md)).
- **Agent skills:** `mastering` (`reference_load`, `meter_compare`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Engine handlers | `resonance-audio/tests/engine/reference_handlers.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Monitor tap, export exclusion | `resonance-audio/tests/mixer/reference_monitor.rs`, `resonance-audio/tests/bounce/reference_export_exclusion.rs` | same |
| Measuring a reference | `resonance-audio/tests/mixer/measure_reference.rs` | same |
| App handlers, events, undo, persistence | `resonance-app/tests/performance/{reference_handlers,reference_events,reference_undo,reference_persistence}.rs`, `resonance-app/tests/io/{reference_disk_round_trip,reference_echo_races}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Panel states | `resonance-app/tests/performance/reference_panel_{scaffold,analyzing,populated,loudness,error}.rs` | same |
| Goldens | `resonance-app/tests/snapshots/reference_panel_*-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test performance` |
| Control | `resonance-app/tests/control/control_reference.rs` | same |

Not covered: no test enables loudness match on a loaded reference and
checks a non-zero offset — which is how the missing computation went
unnoticed.

## Decisions and deviations

- The momentary-audition key is **B**, not X as the design said.
- Accepted formats are WAV, FLAC, MP3 and OGG; the design listed AIFF and
  M4A instead of OGG.
- The panel's tests live in the `performance` test group, not `mixer`.
- The control API has only `reference.load`; listing, removing and A/B
  switching are GUI-only (an agent cannot hear A/B; it compares with
  `meter.compare` / `meter.measure`).

## Known gaps

- Loudness match is wired end to end but the offset is never computed —
  [`../../intents/mixer/reference-loudness-match.md`](../../intents/mixer/reference-loudness-match.md).
- Several engine doc comments in `engine/reference.rs` still describe the
  decode/playback path as "a later todo" although it is built.
