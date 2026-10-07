# Vocal rendering (singing synthesis)

> Verified against master @ `cb3b1d6a` + voicebank registry (vocal slices 4–5) on 2026-10-07.
> Spec history: crate notes `resonance-svs/README.md`, `resonance-svs/NOTES.md` (partly stale, see below); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane's notes and lyrics are sung by a DiffSinger voicebank. The app
turns the lane's derived MIDI clip into a DiffSinger score, runs the
acoustic and vocoder ONNX models off the UI thread, writes a WAV, and
installs it as an audio clip at every placement of the section. Re-renders
only redo the parts of the clip that changed. A lane whose voicebank is
not installed keeps playing its MIDI, and the render fails naming the bank
(`voicebank_missing`).

## Behaviour

- **Voicebanks are installable content** (`VoicebankRegistry`,
  `compose/vocal_svs/registry/`). A lane names its bank by string id
  (`VocalParams::voicebank: VoicebankId`, Lilia by default) and its voice
  mode by `speaker: Option<String>`. The registry scans, at startup, on
  every `vocal.voicebanks` and after an install (`rescan`):
  - **user banks**: each folder of `$XDG_DATA_HOME/resonance/voicebanks/`
    (`$RESONANCE_VOICEBANK_DIR`) holding an acoustic `dsconfig.yaml` (the
    one with an `acoustic:` key) at most two levels down, which covers
    OpenUtau layout and Meiji's `configs/configs/`; the id is the folder
    name as a slug;
  - **development banks**: the three shipped layouts under
    `$RESONANCE_SVS_MODELS_DIR` and `resonance-svs/models/` (resolved from
    the app crate's compile-time `CARGO_MANIFEST_DIR`) with fixed ids —
    `singer/extracted` = `tiger`, `voicebanks/lilia` = `lilia`,
    `voicebanks/meiji` = `meiji`. A user bank with the same id wins.
  - **vocoders**: the folder the acoustic config's `vocoder:` names next
    to it (TIGER's and Meiji's `dsvocoder`), else a bundled
    `dsvocoder/` (Lilia), else a shared vocoder by id in
    `…/resonance/vocoders/<id>/` (`$RESONANCE_VOCODER_DIR`) or a dev root's
    `vocoder/`, matched by folder or by the config's `name`. A vocoder
    counts as installed when its model file exists.
  A hermetic test app scans a private folder under its hermetic root plus
  `$RESONANCE_SVS_MODELS_DIR` only when set, never the source tree.
  `resonance-svs/scripts/download_models.sh` fetches the three shipped
  banks. Models are git-ignored.
- **Facts per bank** (`BankProfile`): derived from the files when the
  bank is installed, from the built-in table (`registry/builtin.rs`) when
  it is a known bank that is not, and conservative defaults otherwise.
  - *Speakers*: the acoustic config's `speakers`. The table holds TIGER's
    allow-list (its acoustic model lists about 40 speakers, most from
    other creators' banks; only `glam fresh disco royal electric mystic
    vinyl` are offered, matched to `…/tiger_*`) and Meiji's four modes
    (`standard hunter lilith phantom` → `embeds/*`). Other banks offer
    every model speaker, id = the slug of its last path segment. A bank
    switch resets the speaker to the bank's default (TIGER: by voice
    type; Meiji: `standard`; others: the first; none for ≤1 speaker).
  - *Phoneme inventory*: the acoustic phoneme dictionary; an ARPAbet
    phone counts as present bare or `en/`-prefixed. `missing_phonemes`
    is ARPAbet minus that. A bank missing more than 8 phones is
    `english: false` (listed, refused by `vocal.set_params`).
  - *Spelling and language ids*: a phone is sent under the key the bank
    stores it as (`en/ah` on Meiji). When the model takes a language
    input, a namespaced key reports its language's id from the bank's
    `languages.json` (read from the path the config names, else the
    bank root), a bare symbol 0 in a bank that namespaces English and the
    bank's id for `en` otherwise.
  - *Curves*: `use_energy_embed` gates Dynamics, `use_breathiness_embed`
    Breathiness, `use_tension_embed` Tension; pitch bend always applies.
    Read at scan time (the old process-lifetime cache is gone).
  - *Comfortable range*, *substitution hints*, *credit / terms URL /
    commercial use* come from the table only: TIGER C3–E5, `paid`;
    Lilia D3–G5, `v` → `f`/`b`, credit "LIEE", `free_with_credit`;
    Meiji A2–D5, credit "Gahata Meiji from LUNAI Project", `permission`.
    An unknown bank gets 48–76 (`range_source: "default"`) and no terms.
- **Missing bank.** `vocal.render` checks every lane it covers before
  anything starts and fails with `voicebank_missing`, naming the bank (or
  the bank's missing vocoder), where a user bank goes, and
  `vocal_voicebanks`. `vocal.generate` still writes the melody but
  reports `render_error {kind: voicebank_missing}`. A GUI render, or a
  bank deleted mid-render, fails asynchronously (`VocalAudioFailed` with
  kind `VoicebankMissing`, banner + job `error.kind`). A render whose
  units all resolve to no phonemes is its own error ("no phonemes to
  sing"); a clip with no notes resolves the job as "no notes to sing".
- **What gets sung.** For each note, the lane's lyric syllable (per-note
  `clip_lyrics` over the draft) is resolved to ARPAbet by
  [pronunciation](pronunciation.md) (the bank's own English dictionary
  is one of its layers) and mapped onto the bank's inventory: a phone the
  bank lacks takes the bank's hint (Lilia `v` → `f`, then `b`), else its
  voiced/voiceless partner, else the nearest phone of the same
  articulatory class (`nearest_by_class`); anything not ARPAbet or with
  nothing near enough blocks the render with an error naming the note,
  the syllable and the word. Slurred notes hold the previous vowel.
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
  `ExpressionCurves` (dynamics, tension, breathiness, pitch bend) and
  sends only those the bank takes (`BankProfile::curve_supported`). On
  the shipped banks no model takes energy or breathiness, so **dynamics
  and breathiness do nothing on any shipped bank**; Lilia and Meiji take
  tension, TIGER does not. An unsupported curve is left out of the
  segment (the dock shows `n/a`). No GUI or control path edits the
  curves yet — see
  [vocal MCP control and quality §10](../../intents/compose/vocal-mcp-control-and-quality.md).
- **Comfortable ranges** (warnings only, `BankProfile::comfortable_range`):
  per bank from the built-in table (see above); per-speaker ranges are not
  known yet, so each speaker reports its bank's.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render entry | `resonance-app/src/compose/vocal_svs/mod.rs`, `error.rs` | `render_vocal_clip`, `VocalRenderError`, `vocal_audio_start`, `RenderedVocal`, `SEGMENT_PAD_SEC`, `SILENCE_GAP_SEC` |
| Voicebank registry | `compose/vocal_svs/registry/{mod,roots,installed,profile,builtin,dict}.rs` | `VoicebankRegistry` (`scan`, `rescan`, `profile`, `get`, `is_known`, `user_bank_dir`), `VoicebankRoots`, `InstalledBank`, `BankProfile` (`render_target`, `phoneme_name`, `language_id`, `curve_supported`, `speakers`, `default_speaker`, `comfortable_range`, `missing_phonemes`, `english`, `dictionary`), `VoicebankMissing`, `BUILTIN_BANKS` |
| App state | `resonance-app/src/lib.rs`, `update/compose/vocal_control.rs` | `Resonance::voicebanks`, `Resonance::rescan_voicebanks` |
| Curve kinds, backend | `compose/vocal_svs/paths.rs` | `CurveKind`, `svs_backend` |
| Phoneme inventory + timing | `compose/vocal_svs/phonemes.rs`, `validate.rs` | `VoicebankPhonemes`, `nearest_substitutes`, `nearest_by_class`, `validate_for_voicebank`, `articulation_report` |
| Segment build | `compose/vocal_svs/segment/{duration,f0,tension,overlay}.rs` | `build_segment` |
| Cache | `compose/vocal_svs/render_cache.rs` | `SvsRenderCache`, `split_render_units`, `render_units_cached`, `RenderPlan` |
| Post-processing | `compose/vocal_svs/post.rs` | `apply_ap_gate`, `safety_gain_factor`, `resample_to`, `write_stereo_wav` |
| Orchestration | `resonance-app/src/update/compose/vocal_render.rs`, `vocal_render_plan.rs`, `vocal_audio_io.rs`, `vocal_audio_install.rs`, `vocal_midi_install.rs` | `enqueue_vocal_render`, `spawn_render`, `rerender_vocal_audio`, `plan_vocal_render`, `handle_vocal_audio_ready`, `reap_orphaned_takes` |
| Registry | `resonance-app/src/compose/state.rs` | `VocalAudioRegistry` (`clips`, `render_epoch`, `in_flight_render`, `clip_lyrics`, `render_cache`) |
| Control | `resonance-app/src/update/control/vocal.rs`, `view_model/voicebanks.rs`, `gate.rs` | `render`, `voicebanks`, `checked_voicebank`, `bank_missing`, `lane_voice`, `voicebank_list` |
| GUI pickers | `view/compose/lane_inspector/vocal/voice_group.rs`, `state/picks.rs`, `state/ui_caches.rs`, `update/compose/lane_inspector/vocal_params.rs` | `voicebank_pickers`, `VoicebankChoice`, `SpeakerChoice`, `UiViewCaches::voicebank_choices`, `VocalRailContext`, `set_voicebank` |
| SVS crate | `resonance-svs/src/{pipeline,ds,config,audio}.rs`, `stages/{acoustic,vocoder,common}.rs`, `voicebank/{mod,layout,dsdict,phonetics}.rs` | `render_segments`, `PipelineArgs`, `DsSegment`, `active_backend`, `voicebank::scan`, `BankDetails`, `looks_like_bank`, `find_dsdict`, `find_library_vocoder`, `installed_vocoder_name`, `load_dsdict`, `PhonemeInventory` |

The crate also has split-pipeline stage scaffolds (`stages/{linguistic,
duration,pitch,variance}.rs`) and a CLI (`--features cli`), which the app
does not use. The registry is built on its voicebank manifest
(`voicebank::scan`).
It depends only on `resonance-music-theory`.

## Surfaces

- **GUI:** vocal lane right rail → Generate (renders after generating) and
  Re-render; Voice & delivery has a **Voicebank** pick list (the installed
  English banks, options cached in `UiViewCaches::voicebank_choices` and
  rebuilt on every rescan; a lane on a bank that is not installed shows
  `<id> (not installed)`) and a **Speaker** pick list (that bank's
  speakers, hidden for a bank with at most one); tempo mismatch warning
  in the inspector. A missing bank surfaces as the render-failure banner.
- **Control API / MCP:** `vocal.render` → `mcp__resonance__vocal_render`,
  a job (`job_status` / `job_wait`). `section_id` renders one lane;
  `track_id` alone renders **all** that track's lanes; neither renders
  every vocal lane. Lanes with no notes or no lyrics are skipped (the call
  fails only if none can render). `voicebank` omitted renders each lane
  with the bank it holds; a given one (any installed or built-in id,
  case-insensitive) is stored on every lane the call covers. A lane whose
  bank or vocoder is missing fails the call with `voicebank_missing`.
  Payload: `track_ids`, `revision`. `vocal.voicebanks` →
  `vocal_voicebanks` (read-only, above the mutation gate, no revision):
  `{installed: [{id, name, version, english, speakers [{id, name,
  comfortable_range}], default_speaker, comfortable_range {low, high,
  range_source}, missing_phonemes, substitutions [{phoneme, sung_as}],
  curves {dynamics, tension, breathiness, pitch_bend}, credit, terms_url,
  commercial, vocoder, vocoder_installed}], available: []}`; it rescans
  first. `song.vocal` reports render state; banks and speakers are better
  set with `vocal.set_params` ([vocal lanes](vocal-lyrics-and-melody.md)).
  `control.hello` carries the optional `svs_backend` (`cpu` / `cuda` /
  `migraphx`), absent until a render has run.
- **Agent skills:** `mixing`, `spatial` read `song_vocal`; external
  `singer-songwriter` and `resonance-studio` skills drive renders.

## How it's tested

| What | Where | Run |
|---|---|---|
| Plan, cache, IO, placement, timing, articulation | `resonance-app/tests/vocal/{vocal_render_plan,vocal_render_cache,vocal_audio_io,vocal_audio_placement,vocal_segment_timing,vocal_articulation,vocal_phoneme_accessor,vocal_expression_segment,voicebank_curve_support}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Registry scans (temp-dir fixture banks: slugs, two-level layout, languages, shared vocoder, user-over-dev, rescan, built-in fallbacks) | `tests/vocal/voicebank_registry.rs`; fixtures from `resonance_app::{write_voicebank_fixture, VoicebankFixture}` | same |
| Real banks register with the right ids, speakers, curve flags | `tests/vocal/voicebank_curve_support.rs` (`real_banks_register_…`, `installed_flags_match_…`; no-ops unless set) | `RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models cargo test -p resonance-app --test vocal` |
| `vocal.voicebanks`, `voicebank_missing`, set_params bank checks | `tests/control/control_voicebanks.rs`, `control_jobs.rs` | same |
| Inspector pickers (installed bank + speaker; `lilia (not installed)`) | `tests/compose/compose_rail_collapse.rs` → `compose_vocal_rail_installed_bank-wgpu.png`, `compose_vocal_rail_lyrics_draft_folded-wgpu.png` | `RESONANCE_BLESS=1` |
| In-flight / re-render safety | `tests/compose/vocal_render_inflight_placement.rs`, `vocal_rerender_keeps_saved_audio.rs`, `vocal_tempo_mismatch_warning.rs` | same |
| Control render, `svs_backend` in hello | `tests/control/control_endpoint.rs`, `control_vocal.rs`, `control_vocal_render_all_lanes.rs`, `control_jobs.rs`, `tests/e2e_compose_via_control.rs` | same |
| SVS crate | `resonance-svs/tests/{ds_validation,mix_timeline,voicebank_manifest,voicebank_phonetics,smoke}.rs` (`read_curve_support`, two-level layout, library vocoders, `dsdict` parsing, execution-provider names); `smoke.rs::end_to_end_render` is `#[ignore]` and also checks `active_backend()` | `./scripts/run-tests.py -p resonance-svs`; by hand: `SVS_POC_VOICEBANK_DIR=… SVS_POC_DS_FILE=… cargo test -p resonance-svs --test smoke -- --ignored` |

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
- A missing bank **is** an error now (spec D12): it fails the render
  naming the bank. The GUI path still tears the lane's old audio down
  before the render discovers the bank is missing (the check runs in the
  render job), so the lane falls back to MIDI rather than keeping stale
  audio; the control path checks up front and leaves everything alone.
- TIGER is scanned through its OpenUtau root config
  (`singer/extracted/dsconfig.yaml`, speakers `dsacoustic/tiger_*`,
  `vocoder: dsvocoder`) rather than the `dsacoustic/dsconfig.yaml` the
  old fixed paths used; it names the same acoustic model, embeddings and
  bundled vocoder (the inner config lists ~40 speakers whose embeddings
  are not all shipped). Checked 2026-10-07: TIGER, Lilia and Meiji each
  rendered non-silent audio through the registry with the real models; no
  listening comparison was made.
- Bank dictionary entries are used only when every phoneme is ARPAbet
  (after stripping `en/`); multi-language extras (`dx`, `L`, `d0`) make the
  word fall through to CMU, because everything after the resolver works in
  ARPAbet (see [pronunciation](pronunciation.md)).
- `set_params` accepts an installed bank or one of the three built-in ids
  (the plok.org index is not there yet, spec §4.6); the GUI picker offers
  installed English banks only.
- The model root falls back to a compile-time source path, so an installed
  binary away from the source tree needs `RESONANCE_SVS_MODELS_DIR` (or the
  bank in the user voicebank folder).

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
- No install path yet (plok.org index, library overlay,
  `vocal.voicebank_install`, `available` entries) — spec slice 7; a bank
  is installed by dropping its folder into the user voicebank folder.
- Comfortable ranges are per bank; no per-speaker ranges are known.
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
