# SMF import leftovers

> Status: **partly built**
> Area: hardware-io
> Source: `docs/history/prototypes/standard-midi-file-import/index.html` (doc #158) compared with [smf-import](../../features/hardware-io/smf-import.md); todo #511.

## Why

The import works, but a few parts of the design are missing:

- The Review rows reserve a slot for a mini piano-roll preview (todo #511);
  the preview notes are parsed (`PREVIEW_NOTES`) but nothing draws them, so
  tracks are told apart only by name, channel and pitch range.
- "Adopt file tempo" takes the tempo map but never the time signatures, so a
  file that changes to 6/8 imports onto 4/4 bars.
- A Format 2 file is refused outright; the design offered "Import first
  pattern only".
- A Format 0 file (the usual export from phones and notation apps) arrives
  as one row with every channel merged, so its parts cannot be picked or
  placed on separate tracks.
- Controller data is dropped (see
  [midi-controller-data](midi-controller-data.md)).

## What it would do

- Draw each row's preview in a small canvas.
- Adopting the file's tempo also adopts its signature events from the
  placement bar on.
- Split a Format 0 file into one row per channel.
- Import the first pattern of a Format 2 file on request.

## What exists to build on

- `ImportedSmf::signature_points` and `TrackImportRow::preview`
  (`resonance-app/src/update/import.rs`).
- `preview_slot` in `resonance-app/src/view/import_dialog.rs` (108 × 30 px).
- The adopt path in `update/import.rs` already splices tempo points.

## Open decisions

1. Split Format 0 by channel always, or as a toggle? Recommended: always,
   drum channel 10 becomes a Drum row.
2. Signature adoption with "match time": skip it? Recommended: adopt only
   with "use the file tempo".

## Slices

1. Preview canvas + golden.
2. Signature adoption with tests in `resonance-app/tests/io/import_dialog_confirm.rs`.
3. Format 0 channel split in `midi_io` with tests in `smf_import.rs`.
4. Format 2 first pattern.

## How we'd know it works

Goldens for the Review stage with previews; parser tests for the split and
Format 2; an import of a 4/4 → 6/8 file lands its bars correctly.
