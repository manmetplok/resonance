# Reference tracks and A/B monitoring

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: design `docs/intents/prototypes/reference-track-ab-monitoring/DESIGN.md` (epic #43); `reference.load` from warmth-width-depth.md §7.5; loudness match from the intent `mixer/reference-loudness-match.md` (BUG-03, folded in here).

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
  trim (±12 dB slider).
- **Loudness match.** Offset = mix integrated LUFS − active reference
  integrated LUFS, so the reference auditions at the mix's loudness. The
  mix value is the live integrated reading of the A/B mix meter
  (post-master, reset on transport start and on a relocate), which the
  audio callback publishes lock-free into `SharedState::mix_meter`; the
  engine thread reads it there. Matching is **on demand**: the offset is
  computed when matching is switched on, when the active reference
  changes while it is on, and when the active reference finishes
  analysing while it is on — never continuously, so the reference level
  does not drift while the user listens. To re-match after more playback,
  switch it off and on again. With no mix measurement yet (`-inf`, nothing
  played since the transport started) or an unanalysed reference, the
  offset is 0. `RefLoudnessMatchChanged` carries the offset and the mix
  loudness it was matched to (`mix_lufs`, `None` when nothing was matched);
  the panel shows the offset beside the toggle and beneath it
  "-9.4 → -16.1 LUFS", "Matches once analysis finishes", or "No mix
  loudness yet — play, then re-toggle".
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
| Engine state, analysis | `resonance-audio/src/engine/reference.rs` | `ReferencePlayer` (`rematch`), `run_reference_analysis`, `handle_set_ref_loudness_match`, `handle_rematch_loudness`, `REFERENCE_OVERVIEW_PEAKS` |
| Engine dispatch | `resonance-audio/src/engine/thread/dispatch/reference.rs` | `mix_integrated_lufs` (reads `SharedState::mix_meter`) |
| Monitor tap | `resonance-audio/src/mixer/callback/reference.rs` | `monitor_reference` |
| Mix meter | `resonance-audio/src/mixer/callback/master_pass.rs` | `ab_meters.mix` |
| App state | `resonance-app/src/reference/state.rs` | `ReferenceState`, `ReferenceMonitorState`, `ReferenceEntry`, `AbMeters` |
| Messages + update | `resonance-app/src/reference/messages.rs`, `update/reference.rs` | `ReferenceMessage`, `REFERENCE_AUDIO_EXTENSIONS` |
| Key / drop subscriptions | `resonance-app/src/update.rs` | `MOMENTARY_AUDITION_KEY`, `reference_momentary_keys`, `reference_file_drop` |
| Echoes | `resonance-app/src/engine_events/reference.rs` | `loudness_match_changed`, `ab_meter_snapshot` |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectReference`, `ProjectReferenceMarker`, `ProjectReferenceSettings` |
| View | `resonance-app/src/view/mixer/reference_panel/` | `bodies.rs`, `ab_controls.rs` (`loudness_row`, `match_caption`), `loudness_canvas.rs`, `waveform.rs` |
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

Loudness match: `reference_handlers.rs` checks the offset (−9 LUFS
reference against a −14 LUFS mix → −5 dB; a quieter reference is raised),
the 0 dB fallbacks (no mix measurement, unanalysed reference), re-matching
on an active-reference change only while matching is on, and that a
matched reference rendered through the monitor meters at the mix's
−14 LUFS. `reference_panel_loudness.rs` checks the readout text for a
match and for no mix measurement; the `reference_panel_loudness_readout`
golden shows matching on with a −6.7 dB offset.

## Decisions and deviations

- The momentary-audition key is **B**, not X as the design said.
- Accepted formats are WAV, FLAC, MP3 and OGG; the design listed AIFF and
  M4A instead of OGG.
- The panel's tests live in the `performance` test group, not `mixer`.
- The control API has only `reference.load`; listing, removing and A/B
  switching are GUI-only (an agent cannot hear A/B; it compares with
  `meter.compare` / `meter.measure`).

## Known gaps

- Loudness match uses only the live mix meter. There is no fallback to
  an offline measurement of the whole mix when nothing has played, and no
  automatic re-match on transport start (the meter resets there, so a
  re-match would have to wait for playback); the user re-toggles. Neither
  is exposed over the control API (an agent compares with
  `meter.compare` / `meter.measure`).
- Several engine doc comments in `engine/reference.rs` still describe the
  decode/playback path as "a later todo" although it is built.
