# Render clip warp (time-stretch) in playback and bounce

> Status: **partly built** (data model, UI, persistence, control API and DSP
> exist; no render path reads them)
> Area: audio-engine (render core), with timeline consequences
> Source: `docs/history/reviews/code-review-2026-10-02.md` FU-W2a, FU-W2b,
> FU-W2e; doc #166; confirmed on master @ `8eac71b3`.
> Register items covered: FU-W2a, FU-W2b, FU-W2e (review 2026-10-02).

## Why

A user or agent can switch a clip to "follow tempo", set its source BPM,
transpose and warp markers, and it is saved, undoable and scriptable
(`clip.set_warp`, `clip.set_warp_markers`). None of it changes what is heard.
The clip inspector says so ("playback does not stretch yet"). An agent that
sets warp to fit a loop to the song tempo gets no feedback that nothing
happened.

## What it would do

A clip with `warp_enabled` plays and bounces time-stretched to the tempo map
(uniformly from `original_bpm`, or piecewise between warp markers), pitched
by `transpose_semitones`, with the chosen `WarpAlgorithm` (`Transient` or
`Tonal`). Fades, the declick ramp, crossfades, trims, splits and the clip's
drawn timeline length follow the warped length. Live playback and offline
bounce/export/freeze render the same samples. Tempo detection
(`DetectClipTempo`) no longer blocks the engine control thread (FU-W2b). The
"playback does not stretch yet" caption and the MCP caveats are removed.

## What exists to build on

- Model: `resonance-audio/src/types/clip.rs` (`warp_enabled`,
  `original_bpm`, `transpose_semitones`, `warp_algorithm`, `warp_markers`,
  `AudioClip::warp_source_frame`).
- Handlers: `engine/clips.rs` (`set_clip_warp_in_place`,
  `set_clip_warp_markers_in_place`, `detect_clip_tempo_in_place`).
- DSP: `resonance-dsp/src/timestretch/{wsola,phase_vocoder}.rs`, tested in
  `resonance-dsp/tests/timestretch.rs`; called from nowhere else.
- Data, GUI, persistence, undo and control API are complete — see
  [warp-and-tempo-detection.md](../../features/timeline/warp-and-tempo-detection.md).
  `warp_source_frame` is tested in `resonance-audio/tests/types/clip_warp.rs`
  and called by nothing else.
- Render: `mixer/render/clips.rs::mix_track_clips` /
  `mix_track_clips_governed` read source frames 1:1.
- Precedent for an offline-built render cache: vocal tuning's
  `ensure_tuning_caches` (built off the RT thread, read on the hot path).
- App: `resonance-app/src/state/clip_warp.rs`,
  `view/clip_inspector/warp.rs`; tests `resonance-app/tests/control/control_clip_warp.rs`,
  `tests/io/clip_warp_persistence.rs`, `resonance-audio/tests/types/clip_warp.rs`,
  `tests/engine/clip_warp_handlers.rs`.

## Open decisions

1. Stretch live per block, or render a warped cache off the audio thread
   (like the vocal-tuning cache) and play that? **Recommended:** a cache
   built on a worker when warp settings change (keyed by the warp parameters
   and project tempo), published through the render graph; the audio thread
   stays a straight read. A per-block realtime stretcher is harder to keep
   allocation-free and identical in bounce.
2. How the no-marker mapping treats trim (the review flagged this): the app
   mirror starts an unmarked warp at `trim_start_frames`, the engine mapping
   at the clip start. **Recommended:** trim offsets stay in source frames and
   the mapping starts at `trim_start_frames`, as the app mirror does.
3. What the clip's timeline length means when warped. **Recommended:**
   the visible length is derived (source length ÷ ratio) and
   `duration_samples` follows it, so fades and split positions stay in
   timeline frames; edits that need source frames convert through
   `warp_source_frame`.
4. Tempo map or base tempo? Markers are projected at the base tempo today.
   **Recommended:** keep base tempo for the first slice and record it as a
   limit; follow the tempo map in a later slice.

## Slices

1. Uniform warp from `original_bpm` (no markers), live and offline, via the
   chosen cache; clip length follows the ratio; a bounce test.
2. Marker-based piecewise warp.
3. Transpose and algorithm choice.
4. Fades / split / trim / bounce-length consequences.
5. Tempo detection moved to a worker, with a control-thread stall test
   (FU-W2b).
6. Remove the caveats from `view/clip_inspector/warp.rs` and
   `resonance-mcp/src/tools/clip.rs` (FU-W2e's marker-vs-trim shadowing at
   the clip's corners was fixed separately as BUG-15).

## How we'd know it works

- Bounce tests in `resonance-audio/tests/bounce/`: a clip at 100 BPM in a
  120 BPM project plays in 5/6 of its length with onsets on the grid
  (measured, not listened).
- A live-vs-bounce parity test like `mixer/render_block_parity.rs` for a
  warped clip.
- `RESONANCE_RENDER_THREADS=8` run with every golden holding.
- By ear: a drum loop at a different tempo stays locked to the metronome.
