# Wire up MIDI project export

> Status: **partly built** (writer exists and is tested; no caller)
> Area: audio-engine (`resonance_audio::midi_io`) + the export dialog
> Source: prototype `docs/intents/prototypes/multitrack-stem-and-midi-export/`;
> `resonance-app/src/update/export.rs` module doc ("The MIDI tab has no
> exporter on the engine side yet, so its primary action stays disabled");
> confirmed on master @ `8eac71b3`.
> Register items covered: FU-A1a2 (review 2026-10-02).

## Why

The export dialog shows an "Export MIDI" tab whose button is disabled. The
app's comment says there is no exporter, but
`resonance_audio::midi_io::write_midi_project` already writes a format-1 SMF
(conductor track with tempo and time-signature events, one named track per
source track) and is covered by `tests/bounce/midi_export_project.rs`.
Moving parts to another DAW or a notation program needs it.

## What it would do

The MIDI tab exports the selected tracks' MIDI clips as one format-1 `.mid`
file (or one file per track, if the prototype asks for it), with the
project's tempo and meter map.

## What exists to build on

- `resonance-audio/src/midi_io.rs` (`write_midi_project`, `encode_midi`).
- `resonance-audio/tests/bounce/midi_export_project.rs`.
- Export dialog mode tabs: `resonance-app/src/view/export_dialog.rs`,
  `resonance-app/src/update/export.rs`.

## Open decisions

1. One multi-track file or one file per track? **Recommended:** one
   format-1 file by default; the dialog's "Export {count} MIDI files" label
   suggests per-track, so decide and fix the label.
2. App-side or engine-command? **Recommended:** app-side call into
   `midi_io` (the app owns the note mirror; no audio is involved).

## Slices

1. Enable the MIDI tab, write the file from the app mirror, test with
   `midly` read-back in `resonance-app/tests/io/`.
2. Optional control method (`render.midi`) if agents need it.

## How we'd know it works

A round trip (export, then `notes.import_midi`) reproduces every note and
the tempo map.
