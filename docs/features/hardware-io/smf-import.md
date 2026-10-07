# Standard MIDI File import

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/prototypes/standard-midi-file-import/index.html` (doc #158). Leads: code review VIEW-25, FU-V2a, FU-V4a, FU-D4, FU-E1; ba doc #270 §10, todo #1195 (control path).

Bring a `.mid` file into the project as editable MIDI clips. In the app a
dialog shows the file's tracks, lets the user pick and rename them, decide
where they land and how a different file tempo is handled, and imports
everything as one undo step. Over the control API, `notes.import_midi`
puts one track of a file into one clip, which is far cheaper than sending
thousands of notes as JSON.

The dialog is summarised from the pool side in
[../timeline/audio-pool-and-import.md](../timeline/audio-pool-and-import.md);
this doc is the full description.

## Behaviour

### Parsing (shared)

- `midi_io::parse_smf_bytes` (midly) accepts Format 0 and Format 1 with
  metrical timing. Format 2 ("unsupported SMF Format 2") and SMPTE-timed
  files are refused.
- Ticks are scaled from the file's PPQ to the engine's 480 PPQ (rounded half
  up). Note-on with velocity 0 counts as note-off; notes are paired per
  channel and pitch.
- Each file track becomes an `ImportedTrack`: name (track or instrument
  name), the channels that carried notes, all notes, pitch range, and a
  conductor flag (meta only, no notes). A Format 0 file is one track with
  every channel merged; it is not split by channel.
- Tempo and time-signature meta events become a bar-indexed tempo map.
  Tempos outside the app's BPM range are clamped and counted
  (`tempo_points_clamped`).
- Only notes are imported. Controllers, pitch bend, aftertouch, program
  changes and lyrics are dropped.

### The Import MIDI dialog

- Open it with the transport's "Import…" button, `Cmd+I` / the palette's
  "Import MIDI…", or by dropping a `.mid`/`.midi` file on the window. The
  file is parsed off the UI thread (Parsing stage).
- **Review:** a summary (format, track count, PPQ, length, tempo range) and
  one row per file track with a checkbox, rename field, kind (Drum if the
  track uses channel 10, else Instrument), first channel, note count and
  pitch range. Tracks with notes start selected; the conductor row cannot be
  selected. All / None toggles. Placement: start at bar 1 or at the
  playhead; add as new tracks, or merge every selected row into one existing
  instrument or vocal track. The per-row preview is an empty placeholder.
- **Tempo conflict:** if the file's tempo differs from the project's by more
  than 0.05 BPM, the dialog asks for one of:
  - keep the project tempo, match bars (notes keep bar/beat positions);
  - keep the project tempo, match time (notes are re-timed from the file's
    tempo map onto the project's, from the import point on);
  - use the file tempo: the project tempo map is replaced from the placement
    bar on (earlier bars keep theirs); notes keep bar/beat positions.
  The file's time signatures are never adopted.
- **Confirm** is refused with a reason when nothing is selected, the merge
  target is missing, not an instrument/vocal track, or frozen. Otherwise it
  creates the tracks (Drums kind for Drum rows), clips and notes, plus any
  adopted tempo, as one undo entry. Each clip is rounded up to whole bars of
  the project's meter (`whole_bars_ticks`). The Imported stage reports what
  landed.

### `notes.import_midi`

- Source: exactly one of `path` (absolute) or `data_base64` (≤ 4 MiB
  decoded).
- Target: `clip_id` (replace that clip's notes) or `track_id` (new clip at
  `start_bar`, default bar 1, optional `name`). A new clip uses the same
  `create_clip_with_notes` as the dialog, so it is rounded up to whole
  bars; a replaced clip keeps its length.
- A file with more than one note-carrying track needs `source_track`
  (0-based); it is refused rather than flattened.
- At most 100 000 notes. The file's tempo is ignored: notes keep their
  bar/beat positions under the project's tempo map.
- One undoable edit. The result gives `clip_id`, `track_id`, `note_count`,
  `source_track`, `source_track_name`, `length_beats` (to the last
  note-off) and `revision`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Parser | `resonance-audio/src/midi_io.rs` | `parse_smf_bytes`, `parse_midi_file`, `ImportedSmf`, `ImportedTrack`, `SmfFormat`, `MidiIoError::UnsupportedFormat2` |
| Dialog state | `resonance-app/src/state/import.rs` | `ImportDialogState`, `ImportStage`, `TempoChoice`, `TempoAlignment`, `PlacementStart`, `PlacementMode` |
| Dialog logic | `resonance-app/src/update/import.rs` | `ImportMessage`, `parse_import_file`, `confirm_blocker`, `confirm`, `create_clip_with_notes`, `whole_bars_ticks`, `is_midi_path` |
| View | `resonance-app/src/view/import_dialog.rs` | Review rows, `preview_slot` placeholder |
| Control | `resonance-app/src/update/control/import_midi.rs` | `read_source`, `resolve_target`, `MAX_NOTES` |

The same `midi_io` module writes clip `.mid` files for project storage and
MIDI export (`write_midi_file`, `write_midi_project`).

## Surfaces

- **GUI:** transport "Import…" button, `Cmd+I`, palette "Import MIDI…",
  window drop.
- **Control API / MCP:** `notes.import_midi` → `mcp__resonance__notes_import_midi`.
- **Agent skills:** `arranging` and `drumming` recommend it for bulk
  material.

## How it's tested

| What | Where | Run |
|---|---|---|
| Parser (formats, PPQ scaling, tempo/meter, clamping) | `resonance-audio/tests/midi_hw/smf_import.rs`, `midi_io.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Dialog stages, parse, review, confirm | `resonance-app/tests/io/import_dialog.rs`, `import_dialog_parse.rs`, `import_dialog_review.rs`, `import_dialog_confirm.rs`, `import_entry_points.rs`; goldens `import_dialog_{drop,review,tempo_conflict}-wgpu.png` | `./scripts/run-tests.py -p resonance-app` |
| Control | `resonance-app/tests/control/control_import_midi.rs` | same |

Not covered: no golden of the Error or Imported stages.

## Decisions and deviations

Against the prototype:

- Built: drop/choose, background parse, review with rename and kind,
  placement (bar 1 / playhead, new tracks / merge), tempo conflict, Format 2
  error, single-undo import, result summary.
- Different: three tempo choices (match bars, match time, adopt) instead of
  keep/adopt with an "align by" sub-choice; adopting keeps bars before the
  placement bar.
- Not built: the per-row mini piano-roll preview (todo #511; the data is
  there, the canvas is a placeholder), "Import first pattern only" for
  Format 2, adopting time signatures, and the summary's "Undo import" /
  "Show in arrangement" buttons.

## Known gaps

- [smf-import-leftovers](../../intents/hardware-io/smf-import-leftovers.md):
  preview canvas, meters, Format 2, controller data.
