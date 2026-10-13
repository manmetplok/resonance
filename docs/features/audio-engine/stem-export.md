# Stem export

> Verified against master @ `f9438e38` on 2026-10-13 (plus the render.stems change).
> Spec history: prototype `docs/intents/prototypes/multitrack-stem-and-midi-export/`;
> "ba todo #322 / #325 / #1239 / #1242 / #1248" are cited in code.

Render several slices of the mix (single tracks, busses, or the full master)
to separate WAV files over one shared range, so they re-import
sample-aligned in another DAW. The export dialog is described in
[overlays-and-dialogs.md](../ui-shell/overlays-and-dialogs.md); this page covers the engine part
and the control method `render.stems`, which drives the same command.

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
| App | `resonance-app/src/update/export.rs`, `engine_events/export.rs`, `view/export_dialog.rs` | `AudioCommand::ExportStems` sender, `stem_targets_for` (the one file-naming rule), `stem_bit_depth`, `StemExport*` mirror |
| Control | `resonance-app/src/update/control/render.rs`, `update/export.rs`, `state/control.rs`, `view/bounce_progress.rs` | `stems`, `stem_sources`, `stems_*` event hooks, `ExportMessage::RenderStems` (the handler reaches the engine through `update()`), `PendingStems`, `JobToken::StemExport`, `view_stems_progress_overlay` |

## Surfaces

- **GUI:** Export dialog → "Audio stems" tab (source checklist, range,
  destination, progress, partial-failure report).
- **Control API / MCP:** `render.stems {dir, range?, overwrite, track_ids?}`
  → `mcp__resonance__render_stems`, a job whose result is
  `StemsResult {paths, duration_s}`.
  - Sources: every top-level track (the dialog's track list; a track's stem
    carries its sub-tracks), or exactly `track_ids`. An unknown id, a
    sub-track or an empty list is `invalid_params`; so is a project with no
    tracks. Busses and master are not offered (a later field).
  - The dialog's defaults (`ExportFormat::default()`: 48 kHz, 24-bit, FX
    tail) and its names (`NN-<name>.wav` in track-id order, via
    `stem_targets_for`), never suffixed: a name already in `dir` answers
    `needs_confirmation` until `overwrite: true`. `dir` must be absolute
    and is created like the dialog creates its destination.
  - `range` as in `render.mixdown` (`meter::resolve_range`).
  - Progress from `StemExportProgress` / `StemExportTargetDone`. Any
    target error fails the job once the queue finishes, its message
    listing the errors and the files written; `StemExportError` fails it
    with the mapped kind (`engine_kind_to_rpc`); a cancel fails it.
  - One export at a time: `stem_export_in_progress` covers the dialog's
    render and a control job (`ControlEndpointState::pending_stems`, held
    until the terminal event), so the dialog cannot open (the render gate
    drops `ExportMessage::Open`) and a second `render.stems` is `busy`
    (mutation gate, or the handler's own check). While a control job runs
    the window shows the `StemsProgress` overlay, whose Cancel sends
    `CancelStemExport` through `ExportMessage::CancelRender`.
  - `engine_events::export` routes each `StemExport*` event to the control
    job when `pending_stems` is set and to the dialog otherwise.
- **Agent skills:** `singing` (a vocal-only stem for its intelligibility
  check), `mastering` (delivery reference).

## How it's tested

| What | Where | Run |
|---|---|---|
| Stem core and export driver | `resonance-audio/tests/bounce/stem_export.rs`, `tests/mixer/stem_render.rs`, `stem_sub_track_render.rs`, `stem_bus_sub_track_render.rs` | `./scripts/run-tests.py -p resonance-audio` |
| App dialog and mirror | `resonance-app/tests/io/export_stems.rs`, `export_dialog_shell.rs` | `./scripts/run-tests.py -p resonance-app` |
| Control job: names match the dialog, defaults, `track_ids`, overwrite, range, failure, cancel, mutual refusal with the dialog, capabilities | `resonance-app/tests/control/control_render.rs` | same |

Not covered: stems re-imported and summed against the mixdown in one test
(sub-track and bus cases are checked separately).

## Decisions and deviations

- Stems render sequentially, never in parallel, because they share live
  plugin instances.
- A partial failure keeps finished files and continues, so the dialog can
  offer to retry the rest. The control job fails instead of reporting a
  partial `paths` list, which a client could mistake for success.
- The stem events carry no correlation id. One export at a time makes
  "whoever started the running export" unambiguous, so the control job is
  found through `pending_stems` rather than a token lookup.

## Known gaps

- `render.stems` exports tracks only; busses and master, and the bit
  depth and sample rate, have no control field yet.
- The export dialog's MIDI tab is disabled although
  `resonance_audio::midi_io::write_midi_project` exists and is tested:
  [midi-project-export](../../intents/audio-engine/midi-project-export.md).
