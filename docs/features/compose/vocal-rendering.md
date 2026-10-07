# Vocal rendering (singing synthesis)

> Verified against master @ `86e5c2af` + vocal-params work on 2026-10-07.
> Spec history: crate notes `resonance-svs/README.md`, `resonance-svs/NOTES.md` (partly stale, see below); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane's notes and lyrics are sung by a DiffSinger voicebank. The app
turns the lane's derived MIDI clip into a DiffSinger score, runs the
acoustic and vocoder ONNX models off the UI thread, writes a WAV, and
installs it as an audio clip at every placement of the section. Re-renders
only redo the parts of the clip that changed. If no voicebank is
installed, the lane quietly keeps playing its MIDI.

## Behaviour

- **Voicebanks.** `VocalVoicebank::{Tiger, Lilia, Meiji}`; Lilia is the default. Looked up under
  `$RESONANCE_SVS_MODELS_DIR`, then `resonance-svs/models/` resolved from
  the app crate's compile-time `CARGO_MANIFEST_DIR` (`locate_voicebank`):
  TIGER at `singer/extracted/dsacoustic/` (bundled vocoder preferred over
  `vocoder/dsvocoder/`), Lilia at `voicebanks/lilia/`, Meiji at
  `voicebanks/meiji/configs/configs/`. TIGER (7 `tiger_*` speakers via
  `VocalSinger`) and Meiji (4 modes via `VocalSingerMeiji`) are
  multi-speaker; Lilia is single-speaker. `resonance-svs/scripts/
  download_models.sh` fetches all three. Models are git-ignored.
- **What gets sung.** For each note, the lane's lyric syllable (per-note
  `clip_lyrics` over the draft) is resolved to ARPAbet by
  [pronunciation](pronunciation.md) and validated for the voicebank:
  symbols a bank lacks are swapped for the nearest substitute (Lilia has no
  `v`, so `v` → `f`, then `b`); anything not ARPAbet or not substitutable
  blocks the render with a precise error. Slurred notes hold the previous
  vowel.
- **Score building (app side).** `build_segment` allocates each note's
  time over its phonemes (consonants short, vowel long, minimum
  articulation times per phoneme class), builds a piecewise f0 with
  portamento (`portamento_ms`) and optional vibrato, a tension curve from
  velocity/contour, and adds 0.3 s `AP` padding at both ends. Gaps longer
  than 0.4 s become breaths and split the clip into independent **render
  units** (`split_render_units`).
- **Inference.** Each missing unit goes through
  `resonance_svs::pipeline::render_segments` — combined acoustic ONNX (mel)
  then vocoder ONNX — on CPU, `speedup` 20, `depth` 1000. `AP`/`SP` spans
  are gated to silence, a safety gain is applied, mono becomes stereo and
  is resampled to the engine rate. The duration / pitch / variance models
  are not run; the app supplies durations and f0 itself.
- **Cache.** `SvsRenderCache` keys units by content, per `(section,
  track)`, so an edit re-renders only the units it touched; the plan
  reports "N of M segments changed".
- **Install.** The WAV (`vocal_*.wav`) goes to the project's `audio/`
  folder, or `$TMP/resonance_vocal` for an unsaved project. One audio clip
  per placement starts at the section start plus the lane's first-note
  offset (`vocal_audio_start`), trimmed past the padding. Each render bumps
  a per-lane **render epoch**; a result arriving for an old epoch, a moved
  or deleted placement is discarded or re-targeted. A save copies live
  clips to `audio/clip_<id>.wav` and reaps orphaned takes in the project's
  own `audio/`.
- **Tempo.** A lane renders once, at the tempo of the section's first
  placement. If the tempo changes while a render runs, it is re-queued at
  the new tempo. If placements sit at different tempos, the inspector warns
  (`vocal_tempo_mismatch`) instead of rendering per placement.
- **Expression curves.** The segment builder reads the lane's
  `ExpressionCurves` (dynamics, tension, breathiness, pitch bend; TIGER
  accepts neither tension nor breathiness). No GUI or control path edits
  them yet — see
  [vocal MCP control and quality §10](../../intents/compose/vocal-mcp-control-and-quality.md).
- **Comfortable ranges** (warnings only, `comfortable_pitch_range`):
  TIGER C3–E5, Lilia D3–G5, Meiji A2–D5.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render entry | `resonance-app/src/compose/vocal_svs/mod.rs` | `render_vocal_clip`, `vocal_audio_start`, `RenderedVocal`, `SEGMENT_PAD_SEC`, `SILENCE_GAP_SEC` |
| Voicebank paths + curve support | `compose/vocal_svs/paths.rs` | `locate_voicebank`, `voicebank_phoneme_name`, `curve_supported`, `CurveKind` |
| Phoneme inventory + timing | `compose/vocal_svs/phonemes.rs`, `validate.rs` | `VoicebankPhonemes`, `missing_phonemes`, `nearest_substitutes`, `validate_for_voicebank`, `comfortable_pitch_range` |
| Segment build | `compose/vocal_svs/segment/{duration,f0,tension,overlay}.rs` | `build_segment` |
| Cache | `compose/vocal_svs/render_cache.rs` | `SvsRenderCache`, `split_render_units`, `render_units_cached`, `RenderPlan` |
| Post-processing | `compose/vocal_svs/post.rs` | `apply_ap_gate`, `safety_gain_factor`, `resample_to`, `write_stereo_wav` |
| Orchestration | `resonance-app/src/update/compose/vocal_render.rs`, `vocal_render_plan.rs`, `vocal_audio_io.rs`, `vocal_audio_install.rs`, `vocal_midi_install.rs` | `enqueue_vocal_render`, `spawn_render`, `rerender_vocal_audio`, `plan_vocal_render`, `handle_vocal_audio_ready`, `reap_orphaned_takes` |
| Registry | `resonance-app/src/compose/state.rs` | `VocalAudioRegistry` (`clips`, `render_epoch`, `in_flight_render`, `clip_lyrics`, `render_cache`) |
| Control | `resonance-app/src/update/control/vocal.rs` | `render`, `lane_render_block`, `lane_voicebank` |
| SVS crate | `resonance-svs/src/{pipeline,ds,config,audio}.rs`, `stages/{acoustic,vocoder}.rs` | `render_segments`, `PipelineArgs`, `DsSegment` |

The crate also has split-pipeline stage scaffolds (`stages/{linguistic,
duration,pitch,variance}.rs`), a voicebank manifest module
(`src/voicebank/`) and a CLI (`--features cli`); the app uses none of them.
It depends only on `resonance-music-theory`.

## Surfaces

- **GUI:** vocal lane right rail → Generate (renders after generating) and
  Re-render; voicebank / singer pick lists in Voice & delivery; tempo
  mismatch warning in the inspector.
- **Control API / MCP:** `vocal.render` → `mcp__resonance__vocal_render`,
  a job (`job_status` / `job_wait`). `section_id` renders one lane;
  `track_id` alone renders **all** that track's lanes; neither renders
  every vocal lane. Lanes with no notes or no lyrics are skipped (the call
  fails only if none can render). `voicebank` omitted renders each lane
  with the bank it holds (a new lane holds Lilia, the code default); a
  given one (`tiger`, `lilia`, `meiji`, case-insensitive) is stored on
  every lane the call covers. Payload: `track_ids`, `revision`.
  `song.vocal` reports render state; banks and speakers are better set
  with `vocal.set_params` ([vocal lanes](vocal-lyrics-and-melody.md)).
- **Agent skills:** `mixing`, `spatial` read `song_vocal`; external
  `singer-songwriter` and `resonance-studio` skills drive renders.

## How it's tested

| What | Where | Run |
|---|---|---|
| Plan, cache, IO, placement, timing, articulation | `resonance-app/tests/vocal/{vocal_render_plan,vocal_render_cache,vocal_audio_io,vocal_audio_placement,vocal_segment_timing,vocal_articulation,vocal_phoneme_accessor,vocal_expression_segment,voicebank_curve_support}.rs` | `./scripts/run-tests.py -p resonance-app` |
| In-flight / re-render safety | `tests/compose/vocal_render_inflight_placement.rs`, `vocal_rerender_keeps_saved_audio.rs`, `vocal_tempo_mismatch_warning.rs` | same |
| Control render | `tests/control/control_vocal.rs`, `control_vocal_render_all_lanes.rs`, `control_jobs.rs`, `tests/e2e_compose_via_control.rs` | same |
| SVS crate | `resonance-svs/tests/{ds_validation,mix_timeline,voicebank_manifest,voicebank_phonetics}.rs`; `smoke.rs::end_to_end_render` is `#[ignore]` | `./scripts/run-tests.py -p resonance-svs`; by hand: `SVS_POC_VOICEBANK_DIR=… SVS_POC_DS_FILE=… cargo test -p resonance-svs --test smoke -- --ignored` |

Not covered: no hermetic test runs ONNX inference; real audio quality is
checked by ear only. The app tests drive the pipeline with injected
results.

## Decisions and deviations

- Durations and f0 are computed in the app rather than by the voicebank's
  duration/pitch models: the combined-acoustic path was the one that
  worked end to end in the PoC.
- One render per lane, not per placement, even across tempo regions
  (FU-M11b): per-placement renders would multiply the vocal-audio maps,
  and a tempo change inside a placement cannot be fixed by any
  constant-tempo render.
- Missing models are not an error: the GUI shows no banner and the control
  job still resolves (`VocalAudioUnavailable`).
- The model root falls back to a compile-time source path, so an installed
  binary away from the source tree needs `RESONANCE_SVS_MODELS_DIR`.

## Known gaps

- Expression curves have no editor, no persistence and no control method —
  [vocal MCP control and quality §10](../../intents/compose/vocal-mcp-control-and-quality.md).
- No harmony / doubled voices —
  [vocal-harmony-doubling](../../intents/compose/vocal-harmony-doubling.md).
- Inference is CPU-only in the app (`ExecutionProvider::Cpu` is hard-coded
  even though the crate supports CUDA/ROCm features).
