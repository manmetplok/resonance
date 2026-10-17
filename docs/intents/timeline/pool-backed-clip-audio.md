# Pool-backed clip audio

> Stage: **intent**
> Status: **idea**
> Area: timeline
> Source: verifying `docs/features/timeline/audio-pool-and-import.md` on 2026-10-07 (doc #175 intent: clips reference pool assets).
> Register items covered: FU-W4a (review 2026-10-02).

## Why

A clip placed from the pool plays `audio/asset_<id>.wav` until the first
save. Then `save_clips_to_project_dir` copies its audio to
`audio/clip_<id>.wav`, and after a reload the clip plays that copy. A sample
placed 40 times is stored 41 times. "Relink" also has to handle two kinds of
missing file (assets and clips), and the folder search can't find missing
clips because their filenames carry no original name. Undoing an import also
leaves `asset_<id>.wav` files on disk.

## What it would do

- A clip with an `asset_ref` keeps playing its asset's WAV across save and
  load, and no `clip_<id>.wav` is written for it.
- Only clips that have no asset (recordings, bounces, takes) get their own
  file.
- Optional: a "clean up unused files" pass that removes asset WAVs that
  neither the pool nor the undo history references.

## What exists to build on

- `ClipState::asset_ref`, `ProjectClip::asset_ref`, `MediaPool`.
- `resonance-audio/src/engine/clips.rs::save_clips_to_project_dir`, which
  sorts clips into Ready, Copy and Encode.
- `update/project_io/reconcile/clips.rs::load_audio_clip`, which today always
  loads `pc.audio_file`.

## Open decisions

1. Where the audio path of an asset-backed clip is decided. Recommended:
   `ProjectClip::audio_file` names the asset's WAV when `asset_ref` is set,
   and the engine save treats an asset-mapped clip as Ready.
2. What happens to projects already saved with duplicated clip WAVs.
   Recommended: leave them alone (there are no real users yet). New saves
   simply stop duplicating.
3. Whether to delete orphaned asset files. Recommended: not automatically;
   offer it next to "Remove unused".

## Slices

1. Save and load: asset-backed clips keep their asset path. Covered by a
   test that places the same asset twice, saves, and finds one WAV.
2. Relink: a missing asset then resolves every clip that uses it, so
   missing-clip relink is needed only for clips with no asset.

## How we'd know it works

- After placing one asset N times and saving, `audio/` holds one file for
  it.
- Reload plays the same audio, checked bit-identical with a bounce.
- Existing relink tests still pass.
