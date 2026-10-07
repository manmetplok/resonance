# Vocal rendering (singing synthesis)

> Verified against master @ `86e5c2af` on 2026-10-07.
> Spec history: crate notes `resonance-svs/README.md`, `resonance-svs/NOTES.md` (partly stale, see below); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane's notes and lyrics are sung by a DiffSinger voicebank. The app
turns the lane's derived MIDI clip into a DiffSinger score, runs the
acoustic and vocoder ONNX models off the UI thread, writes a WAV, and
installs it as an audio clip at every placement of the section. Re-renders
only redo the parts of the clip that changed. If no voicebank is
installed, the lane quietly keeps playing its MIDI.

## Behaviour

- **Voicebanks.** `VocalVoicebank::{Tiger, Lilia, Meiji}`. Looked up under
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
  then vocoder ONNX — on CPU, `speedup` 20, `depth` 1000. All three shipped
  acoustic models are continuous-acceleration exports that take a `steps`
  input rather than `speedup`, and the pipeline passes the `speedup` value
  as the step count: the app renders with **20 sampling steps**. TIGER and
  Meiji take a fractional `depth`, so 1000 is clamped to their `max_depth`
  (0.6); Lilia takes no `depth`. `AP`/`SP` spans
  are gated to silence, a safety gain is applied, mono becomes stereo and
  is resampled to the engine rate. The duration / pitch / variance models
  are not run; the app supplies durations and f0 itself.
- **Backend.** The app requests CPU. The crate can request `cuda` or
  `migraphx` (cargo features of the same name; the old `rocm` feature is
  gone because ONNX Runtime 1.23 removed that provider). A GPU provider is
  registered with ort's `error_on_failure`; if it fails, the session is
  rebuilt on CPU with a warning. The backend the acoustic session really
  runs on is logged once per change and recorded
  (`resonance_svs::active_backend()`); `control.hello` reports it as
  `svs_backend` once a render has created a session.
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
  `ExpressionCurves` (dynamics, tension, breathiness, pitch bend). Which
  of them a bank takes comes from its acoustic `dsconfig.yaml`
  (`curve_supported`): `use_energy_embed` gates dynamics,
  `use_breathiness_embed` breathiness, `use_tension_embed` tension; pitch
  bend is an f0 edit and always applies. The flags are parsed once per
  bank per process; a bank that is not installed answers from a table
  equal to the shipped configs (`shipped_curve_support`). On the shipped
  banks no model takes energy or breathiness, so **dynamics and
  breathiness do nothing on any shipped bank**; Lilia and Meiji take
  tension, TIGER does not. An unsupported curve is left out of the
  segment (the dock shows `n/a`). No GUI or control path edits the
  curves yet — see
  [vocal MCP control and quality §10](../../intents/compose/vocal-mcp-control-and-quality.md).
- **Comfortable ranges** (warnings only, `comfortable_pitch_range`):
  TIGER C3–E5, Lilia D3–G5, Meiji A2–D5.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render entry | `resonance-app/src/compose/vocal_svs/mod.rs` | `render_vocal_clip`, `vocal_audio_start`, `RenderedVocal`, `SEGMENT_PAD_SEC`, `SILENCE_GAP_SEC` |
| Voicebank paths, curve support, backend | `compose/vocal_svs/paths.rs` | `locate_voicebank`, `voicebank_phoneme_name`, `curve_supported`, `bank_curve_support`, `shipped_curve_support`, `svs_backend`, `CurveKind` |
| Phoneme inventory + timing | `compose/vocal_svs/phonemes.rs`, `validate.rs` | `VoicebankPhonemes`, `missing_phonemes`, `nearest_substitutes`, `validate_for_voicebank`, `comfortable_pitch_range` |
| Segment build | `compose/vocal_svs/segment/{duration,f0,tension,overlay}.rs` | `build_segment` |
| Cache | `compose/vocal_svs/render_cache.rs` | `SvsRenderCache`, `split_render_units`, `render_units_cached`, `RenderPlan` |
| Post-processing | `compose/vocal_svs/post.rs` | `apply_ap_gate`, `safety_gain_factor`, `resample_to`, `write_stereo_wav` |
| Orchestration | `resonance-app/src/update/compose/vocal_render.rs`, `vocal_render_plan.rs`, `vocal_audio_io.rs`, `vocal_audio_install.rs`, `vocal_midi_install.rs` | `enqueue_vocal_render`, `spawn_render`, `rerender_vocal_audio`, `plan_vocal_render`, `handle_vocal_audio_ready`, `reap_orphaned_takes` |
| Registry | `resonance-app/src/compose/state.rs` | `VocalAudioRegistry` (`clips`, `render_epoch`, `in_flight_render`, `clip_lyrics`, `render_cache`) |
| Control | `resonance-app/src/update/control/vocal.rs` | `render`, `lane_render_block`, `lane_voicebank` |
| SVS crate | `resonance-svs/src/{pipeline,ds,config,audio}.rs`, `stages/{acoustic,vocoder,common}.rs`, `voicebank/layout.rs` | `render_segments`, `PipelineArgs`, `DsSegment`, `build_session_reporting`, `active_backend`, `read_curve_support` |

The crate also has split-pipeline stage scaffolds (`stages/{linguistic,
duration,pitch,variance}.rs`), a voicebank manifest module
(`src/voicebank/`) and a CLI (`--features cli`); of these the app uses
only `voicebank::layout::read_curve_support`.
It depends only on `resonance-music-theory`.

## Surfaces

- **GUI:** vocal lane right rail → Generate (renders after generating) and
  Re-render; voicebank / singer pick lists in Voice & delivery; tempo
  mismatch warning in the inspector.
- **Control API / MCP:** `vocal.render` → `mcp__resonance__vocal_render`,
  a job (`job_status` / `job_wait`). `section_id` renders one lane;
  `track_id` alone renders **all** that track's lanes; neither renders
  every vocal lane. Lanes with no notes or no lyrics are skipped (the call
  fails only if none can render). `voicebank` omitted keeps the lane's
  choice, except that a lane still on the code default (TIGER) renders
  with Lilia. Payload: `track_ids`, `revision`. `song.vocal` reports
  render state. `control.hello` carries the optional `svs_backend`
  (`cpu` / `cuda` / `migraphx`), absent until a render has run.
- **Agent skills:** `mixing`, `spatial` read `song_vocal`; external
  `singer-songwriter` and `resonance-studio` skills drive renders.

## How it's tested

| What | Where | Run |
|---|---|---|
| Plan, cache, IO, placement, timing, articulation | `resonance-app/tests/vocal/{vocal_render_plan,vocal_render_cache,vocal_audio_io,vocal_audio_placement,vocal_segment_timing,vocal_articulation,vocal_phoneme_accessor,vocal_expression_segment,voicebank_curve_support}.rs` | `./scripts/run-tests.py -p resonance-app` |
| In-flight / re-render safety | `tests/compose/vocal_render_inflight_placement.rs`, `vocal_rerender_keeps_saved_audio.rs`, `vocal_tempo_mismatch_warning.rs` | same |
| Control render, `svs_backend` in hello | `tests/control/control_endpoint.rs`, `control_vocal.rs`, `control_vocal_render_all_lanes.rs`, `control_jobs.rs`, `tests/e2e_compose_via_control.rs` | same |
| SVS crate | `resonance-svs/tests/{ds_validation,mix_timeline,voicebank_manifest,voicebank_phonetics,smoke}.rs` (`read_curve_support`, execution-provider names); `smoke.rs::end_to_end_render` is `#[ignore]` and also checks `active_backend()` | `./scripts/run-tests.py -p resonance-svs`; by hand: `SVS_POC_VOICEBANK_DIR=… SVS_POC_DS_FILE=… cargo test -p resonance-svs --test smoke -- --ignored` |

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
  even though the crate supports the `cuda` and `migraphx` features).
  Neither GPU path has been exercised on a GPU; the prebuilt ONNX Runtime
  that `download-binaries` fetches has no MIGraphX provider, so a
  `migraphx` build needs `ORT_LIB_LOCATION` pointing at an ORT built with
  it (without one, registration fails and the session falls back to CPU,
  which is checked).
- The `render_resonance_vocal` example cannot drive Meiji: its G2P copy
  emits bare ARPAbet without Meiji's `en/` prefix or language ids.
- `speedup` is a misnomer for the shipped banks (it is the `steps` count);
  the knob is not exposed to the user.

## Measurements

`speedup` 20 against 50, 2026-10-07, with
`cargo run --release -p resonance-svs --features cli --example
render_resonance_vocal -- --speedup N --repeat 3` on its built-in 12.72 s
B-minor demo phrase (103 phonemes, one segment), CPU backend, AMD Ryzen 7
9700X (8 cores / 16 threads), ONNX Runtime default threading. The machine
was shared with parallel builds (load average 12–20), so treat absolute
times as upper bounds; the ratio held across two batches. `wall` covers
session loads plus inference, `infer` the acoustic + vocoder runs; RTF =
time / audio seconds. Warm runs 2–3 of the second batch:

| Bank | `speedup` (= steps) | wall | infer | RTF (wall) |
|---|---|---|---|---|
| TIGER | 20 | 3.78–3.89 s | 3.60–3.72 s | 0.30 |
| TIGER | 50 | 6.24–6.39 s | 6.08–6.23 s | 0.49–0.50 |
| Lilia | 20 | 3.06–3.16 s | 2.95–3.05 s | 0.24–0.25 |
| Lilia | 50 | 5.14–5.27 s | 5.03–5.16 s | 0.40–0.41 |
| Meiji | — | not measured (example can't phonemise for Meiji) | | |

Because every shipped acoustic model takes `steps`, raising `speedup` from
20 to 50 means **2.5x the sampling steps** and costs about 1.7x the time.
It does not speed rendering up, as it would on an old `speedup`-input
export. The research note's premise (`speedup` 20 = 2.5x OpenUtau's
default denoising work) does not hold for these banks: moving to 50 buys
more steps, not fewer. The app default stays 20; no listening comparison was made
here (the WAVs are not kept in the repo).
