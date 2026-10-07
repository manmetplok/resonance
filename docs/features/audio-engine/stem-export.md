# Stem export

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: prototype `docs/intents/prototypes/multitrack-stem-and-midi-export/`;
> "ba todo #322 / #325 / #1239 / #1242 / #1248" are cited in code.

Render several slices of the mix (single tracks, busses, or the full master)
to separate WAV files over one shared range, so they re-import
sample-aligned in another DAW. The export dialog is described in
[overlays-and-dialogs.md](../ui-shell/overlays-and-dialogs.md); this page covers the engine part.

## Behaviour

- `ExportStems { targets: Vec<StemTarget>, range, sample_rate, bit_depth,
  include_fx_tail }`. Each `StemTarget` is a `StemSource` plus an absolute
  `.wav` path.
- **What each source contains** (`stem_filter`):
  - `Track(id)`: the track and its sub-tracks; no master FX or volume (it
    re-imports at unity).
  - `Bus(id)`: every track routed to the bus, including sub-tracks routed
    there on their own; the bus's own chain runs; no master FX or volume.
  - `Master`: everything, with master FX, master volume and hard clip, as in
    the mixdown.
  - Sidechain key sources are rendered too (key-only) so keyed plugins in the
    slice behave as in the mix.
  - A sub-track pulls in its parent as fan-out-only: the parent's instrument
    runs, but its own main output stays out of the stem.
  - A sub-track whose parent is frozen is refused (`FrozenFanOut`): the
    frozen cache has no separable taps.
- **Range:** the given `range`, or the whole project
  (`stem_project_range`: earliest to latest clip). All targets share it.
  `include_fx_tail` adds the 2 s offline tail.
- **Output:** `StemBitDepth::{Int16, Int24, Float32}`; resampled to
  `sample_rate` with the shared band-limited resampler only when it differs
  from the engine rate. Written via a temp file and rename.
- **Order and failure:** targets render one after another on one worker
  (they share the live plugin instances). A failed target sends
  `StemExportTargetError` and the queue continues. Events:
  `StemExportProgress`, `StemExportTargetDone`, `StemExportTargetError`,
  `StemExportComplete { files }`, or `StemExportCancelled` (with the files
  already written) after `CancelStemExport`, which is checked between
  targets. A failure before any target starts is `StemExportError(EngineError)`.
- All the offline-render rules apply (gate, transport refusal, offline mode,
  PDC trim): see [offline-render-and-export.md](offline-render-and-export.md).
- The same stem renderer feeds offline measurement
  ([offline-measurement.md](offline-measurement.md)).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Stem core | `resonance-audio/src/engine/bounce/stem.rs` | `StemFilter`, `stem_filter`, `stem_filter_with_keys`, `frozen_fan_out_refusal`, `render_stem`, `render_filtered`, `stem_project_range`, `write_stem_wav`, `StemError` |
| Multi-target driver | `resonance-audio/src/engine/bounce/stem_export.rs` | `export_stems`, `export_stems_spawn` |
| Types | `resonance-audio/src/types/stem.rs` | `StemSource`, `StemTarget`, `StemBitDepth` |
| App | `resonance-app/src/update/export.rs`, `engine_events/export.rs`, `view/export_dialog.rs` | `AudioCommand::ExportStems` sender, `StemExport*` mirror |

## Surfaces

- **GUI:** Export dialog → "Audio stems" tab (source checklist, range,
  destination, progress, partial-failure report).
- **Control API / MCP:** none. `render.stems` is defined in
  `resonance-control/src/methods/render.rs` but answers `unsupported` and is
  kept out of `METHODS` and `control.hello` capabilities.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Stem core and export driver | `resonance-audio/tests/bounce/stem_export.rs`, `tests/mixer/stem_render.rs`, `stem_sub_track_render.rs`, `stem_bus_sub_track_render.rs` | `./scripts/run-tests.py -p resonance-audio` |
| App dialog and mirror | `resonance-app/tests/io/export_stems.rs`, `export_dialog_shell.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: stems re-imported and summed against the mixdown in one test
(sub-track and bus cases are checked separately).

## Decisions and deviations

- Stems render sequentially, never in parallel, because they share live
  plugin instances.
- A partial failure keeps finished files and continues, so the dialog can
  offer to retry the rest.

## Known gaps

- No control-API stem export:
  [render-range-and-stems-control](../../intents/control-api/render-range-and-stems.md).
- The export dialog's MIDI tab is disabled although
  `resonance_audio::midi_io::write_midi_project` exists and is tested:
  [midi-project-export](../../intents/audio-engine/midi-project-export.md).
