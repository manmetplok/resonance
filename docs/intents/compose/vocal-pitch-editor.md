# Vocal pitch editor (graphical tuning of recorded vocals)

> Stage: **intent**
> Status: **partly built**
> Area: compose
> Touches: editor UI; engine part is [audio-engine](../../features/audio-engine/)
> Source: doc #160 (todos #352–#359) as cited in `resonance-app/src/update/vocal_tuning.rs` and `resonance-audio/src/engine/{vocal_analysis,vocal_render}.rs`.

## Why

Recorded vocal takes need pitch correction. The engine already detects
pitch, segments notes and renders a formant-preserving retune from a
clip's `VocalTuning` edits, but there is no editor: the only app-side
messages are `VocalTuningMessage::{OpenPitchEditor, ClosePitchEditor}`,
and no view sends them. Users cannot see or edit the detected notes, so
the retune path is unreachable.

## What it would do

Open a pitch editor on an audio clip of a vocal track: show the detected
f0 contour and note blobs, let the user drag notes to pitches, snap to a
scale, set global correction amount/speed, and hear the result in
playback and bounces. Edits are undoable and saved.

## What exists to build on

- App: `update/vocal_tuning.rs` (open → `AudioCommand::AnalyzeClipPitch`),
  `ClipState::vocal_tuning` mirror, `ui.interaction.editing_pitch_clip`;
  test `tests/vocal/vocal_tuning_mirror.rs`.
- Engine: `resonance-audio/src/engine/vocal_analysis.rs` (`detect_f0`,
  `segment_notes` from `resonance-dsp`), `engine/vocal_render.rs`
  (`ensure_tuning_caches`, `retune_clip`, `pitch_ratio_curve`),
  `types::TuningScale`.
- Feature docs: compose [README](../../features/compose/README.md);
  engine side under [audio-engine](../../features/audio-engine/).

## Open decisions

1. Where the editor opens: a bottom panel like the MIDI editor
   (recommended, reuses the editor-panel slot) vs. the clip inspector.
2. Edit granularity first: per-note pitch offset only (recommended), then
   drift/vibrato.

## Slices

1. Entry point (clip context menu / double-click on vocal track clip) +
   read-only contour view; golden.
2. Per-note pitch edits + scale snap messages (undoable) wired to the
   engine's `VocalTuning`.
3. Control methods for agents (`clip.set_tuning`), tests.

## How we'd know it works

Goldens of the editor; a test that a note edit changes the clip's
`VocalTuning` and that a bounce of the clip differs from the untuned
source.
