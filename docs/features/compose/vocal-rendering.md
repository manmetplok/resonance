# Vocal rendering (singing synthesis)

> Verified against master @ `2b28debd` + vocal phase 4 on 2026-10-07.
> Spec history: [`docs/history/specs/vocal-mcp-control-and-quality.md`](../../history/specs/vocal-mcp-control-and-quality.md) §7–§10 (slices 2, 6, 9, 12–14); crate notes `resonance-svs/README.md`, `resonance-svs/NOTES.md` (partly stale, see below); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane's notes and lyrics are sung by a DiffSinger voicebank. The app
turns the lane's derived MIDI clip into a DiffSinger score, runs the
acoustic and vocoder ONNX models off the UI thread, writes a WAV, and
installs it as an audio clip at every placement of the section. Re-renders
only redo the parts of the clip that changed. A lane whose voicebank is
not installed keeps playing its MIDI, and the render fails naming the bank
(`voicebank_missing`). Each lane knows whether its audio is current
(`rendered` / `stale` / `rendering` / `error` / `not_rendered`); a GUI edit
re-renders the changed phrases 750 ms after the last edit, a control edit
only marks the lane stale.

## Behaviour

- **Voicebanks** are installable content: a lane names its bank by id
  (`VocalParams::voicebank`, Lilia by default) and its voice mode by
  `speaker`. Which banks exist, what each can do (speakers, phoneme
  inventory, language ids, curve support, comfortable range, terms) and
  how they are installed from plok.org is the
  [voicebank library](voicebank-library.md). The render asks
  `BankProfile` (`VoicebankRegistry::profile`) for the bank's model files
  (`render_target`), phoneme keys, language ids and curve support.
- **Missing bank.** `vocal.render` checks every lane it covers before
  anything starts and fails with `voicebank_missing`, naming the bank (or
  the bank's missing vocoder), where a user bank goes, and
  `vocal_voicebanks`. Every render path checks the bank right after
  planning and **before** queuing anything
  (`enqueue_vocal_render` → `render_target_or_missing`): a GUI render
  keeps the old audio, raises the banner and fails any job waiting on the
  lane with `voicebank_missing`; `vocal.generate` writes the melody,
  skips its auto-render and reports `render_error {kind:
  voicebank_missing}`; a chord or length edit re-derives the lane's
  melody without rendering at all (`regenerate_lane_as_side_effect` →
  `rederive_vocal_melody`), so it raises no banner and keeps whatever
  one was showing. A bank deleted mid-render fails
  asynchronously (`VocalAudioFailed` with kind `VoicebankMissing`, banner
  + job `error.kind`), naming the user voicebank folder like the
  up-front check. Every failure is also kept as the lane's render
  `error` (see Render state). A render whose
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
  velocity/contour, and adds 0.3 s `SP` padding at both ends. Rests split
  the clip into independent **render units** (`split_render_units`).
- **Rests and breaths** (`compose/vocal_svs/rests.rs`, spec §8). A gap
  between a note's end and the next onset of **at least**
  `rest_threshold_beats` (lane param, 0.125–4 beats, default ½, converted
  at the lane's tempo — 0.25 s at 120 bpm) is a rest: the note sings its
  written length and the gap is an `SP`. A shorter gap is sung legato
  (the note holds to the next onset). One `RestRule` answers this for the
  duration builder, the render-unit splitter and the articulation report
  (`articulation_report_for` takes the lane's threshold; `song.vocal`
  passes it). A standalone `AP` in the
  lyrics (`AssignedSyllable::breath_before`) is an `AP` **breath** in
  front of its note: it takes the end of the gap, up to 0.35 s; when the
  gap is shorter (or the join legato) it also takes up to 30 % of the
  previous note's phonemes, at most 0.25 s (and never below the 50 ms
  note floor); on the segment's first note it takes the leading pad,
  keeping 0.05 s of silence (0.25 s of breath). Time only moves between
  neighbours, so the segment still spans its notes. A rest followed by a
  breath does not split render units, so the breath can use the gap.
  Only `SP` is gated to silence after the vocoder; `AP` (written
  breaths, and a bracketed `[AP]` note) is heard.
- **Velocity dynamics** (spec §9; `compose/vocal_svs/dynamics.rs`,
  `segment/energy.rs`). The lane's `velocity_dynamics` (0–1, default 0.5)
  maps note velocity to loudness. On a bank whose model takes energy
  (`curve_supported(Dynamics)`) velocity drives the energy curve
  (`amount · (vel − 0.78)/0.22`, clamped to ±1, the overlay's signed
  convention; an edited Dynamics overlay wins). On a bank without energy
  velocity reaches the model through the tension curve as before
  (`tension_velocity_amount`, unchanged). On **every** bank a per-note
  gain is applied to the stitched take after the vocoder and before the
  safety gain: `amount · 6 dB · (vel − 0.78)/0.22` (raw note velocity,
  clamped to ±6 dB), each note from its onset to the next (a written
  breath belongs to the note it leads into: that note's span starts at
  the breath, `breath_lead_secs` → `velocity_gain_spans_with_breaths`),
  steps smoothed
  by a centred 30 ms moving average in dB. Amount 0 leaves the take
  bit-identical. The gain is a reusable **dynamics gain envelope**:
  `GainSpan {start_sec, end_sec, gain_db}`, `dynamics_gain_envelope(spans,
  len, sample_rate, smooth_sec) -> Option<Vec<f32>>` (per-sample linear
  gain; `None` when every span is 0 dB), `apply_dynamics_gain(mono,
  spans, sample_rate) -> bool`, with `velocity_gain_spans` /
  `velocity_gain_db` as the velocity feed. The Dynamics expression curve
  on a bank without energy is a second layer (one span per curve frame,
  `RenderUnit::curve_gain`); `dynamics_gain_envelope_layers` /
  `apply_dynamics_gain_layers` lay each layer out on its own and add them
  in dB (the gains multiply) before the one smoothing pass.
- **Inference.** Each missing unit goes through
  `resonance_svs::pipeline::render_segments` — combined acoustic ONNX (mel)
  then vocoder ONNX — on CPU, `speedup` 20, `depth` 1000. All three shipped
  acoustic models are continuous-acceleration exports that take a `steps`
  input rather than `speedup`, and the pipeline passes the `speedup` value
  as the step count: the app renders with **20 sampling steps**. TIGER and
  Meiji take a fractional `depth`, so 1000 is clamped to their `max_depth`
  (0.6); Lilia takes no `depth`. `SP` spans
  are gated to silence (`AP` breaths are not), the velocity gain and then
  a safety gain are applied, mono becomes stereo and
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
  reports "N of M segments changed". The key covers what renders the
  unit plus two lane params: the acoustic and vocoder config paths, the
  speaker key the lane's speaker resolves to, and the bank version — a
  rescan that resolves the same params to another speaker or release
  re-renders — and `rest_threshold_beats` and `velocity_dynamics`
  (`lane_identity`), so changing either re-renders and moves the plan.
  The velocity gain itself is applied after the cache, to the stitched
  take.
- **Install.** The WAV (`vocal_*.wav`) goes to the project's `audio/`
  folder, or `$TMP/resonance_vocal` for an unsaved project. One audio clip
  per placement starts at the section start plus the lane's first-note
  offset (`vocal_audio_start`), trimmed past the padding. Each render bumps
  a per-lane **render epoch**; a result arriving for an old epoch, a moved
  or deleted placement is discarded or re-targeted. The previous take is
  **not** torn down when a render is queued: it plays until the new take
  replaces it, placement by placement, in `handle_vocal_audio_ready` (a
  failed or superseded render leaves it in place). A save copies live
  clips to `audio/clip_<id>.wav`, writes each take's unit keys as
  `ProjectClip::vocal_render_units`, and reaps orphaned takes in the
  project's own `audio/`.
- <a id="render-state"></a>**Render state** (spec §7, D19–D22;
  `update/compose/vocal_render_state.rs`, types in
  `compose/vocal_render_status.rs`).
  - *Staleness by content and position.* Every take records the unit
    **state keys** it stitched (`RenderUnit::state_key`:
    the unit's cache key hashed with its first note's clip tick and,
    when `velocity_dynamics` > 0, its notes' raw velocities —
    what the take depends on that the position-free cache key leaves
    out: moving the whole line moves the installed audio out of place,
    and the post gain reads velocity after the cache, so on TIGER a
    velocity edit changes no cache key) (`SvsRenderCache::last_keys`,
    carried in
    `VocalAudioReadyData::unit_keys`) per installed audio clip
    (`VocalRenderStatus::clip_unit_keys`). The lane's current plan is
    hashed the same way (`expected_unit_keys` → `plan_unit_keys`, through
    the planner `enqueue_vocal_render` uses); equal key lists =
    `rendered`, otherwise `stale` with `stale_units` = planned units not
    in the take. A take with no recorded keys (saved before this existed)
    reads `stale`; so does a lane whose plan cannot be built any more.
  - *Memo.* Planning builds every segment, so the expected keys are
    memoised per lane by a **fingerprint** of what the plan reads (clip
    notes and lyrics, params (expression curves included), overrides,
    project dictionary, section tempo, the bank's model identity and stamp). A read
    with an unchanged fingerprint costs ~3 µs per lane (release build).
  - *Precedence.* A queued render reads `rendering`; else a failure
    recorded on the lane's current fingerprint reads `error` (kinds
    `voicebank_missing`, `nothing_to_sing`, `no_notes`,
    `unsupported_phonemes`, `invalid_params`, `render_failed`); else no
    audio reads `not_rendered`; else the key comparison. A successful
    render or any edit that changes the fingerprint retires an error; an
    async failure is recorded on the fingerprint its render was queued
    from.
  - *Persistence.* The keys travel with their clip: saved on its
    `ProjectClip`, restored by the `vocal_audio_clips` reconcile on load
    and on undo/redo, so a reopened project reads `rendered`.
  - *Origin and the debounced re-render.* `Resonance::update` runs
    `vocal_render_state::settle` after every outermost update, passing
    whether it was a GUI message or a control request (a `Tick` that
    drained no engine event but playhead moves skips it — the tick runs
    at frame rate during playback, and settling fingerprints every
    lane)
    (`Message::Control`, which wraps every MCP edit, nested
    `run_via_update` dispatches included). Settle refreshes the memo and
    the per-lane status the views read (`VocalRenderStatus::lanes`). For
    a lane whose fingerprint a **GUI** update changed (lyric, note, chord,
    param, pronunciation and expression-curve edits, a GUI undo), that has
    audio, a render in flight for other content, or a failed render, and
    whose plan no longer matches its take, it arms a timer: a
    `VocalAutoRender {generation}` message 750 ms later
    (`AUTO_RENDER_DEBOUNCE`). A newer edit re-arms with a new generation,
    so only the last one renders, and the render cache re-synthesises
    only the changed units. A lane never rendered waits for Generate /
    Re-render. **Control** edits never arm it: an agent batches its edits
    and calls `vocal.render` once; `vocal.generate` keeps its auto-render.
  - *Chord and length edits* re-derive the lane's melody without
    rendering (`rederive_vocal_melody`), so they follow the same rule
    (D21). They used to queue a render whose task was dropped, leaving the
    lane without audio and stuck `rendering`. Expression-curve edits no
    longer render on every breakpoint move either: the debounce covers
    them.
  - *Disk load* resets the status (`VocalRenderStatus::reset`), so opening
    a project arms nothing; the debounce generation keeps counting, so a
    timer armed before the load can never fire as one armed after it;
    an undo keeps the memo, so a GUI undo counts as an edit.
- **Tempo.** A lane renders once, at the tempo of the section's first
  placement. If the tempo changes while a render runs, it is re-queued at
  the new tempo. If placements sit at different tempos, the inspector warns
  (`vocal_tempo_mismatch`) instead of rendering per placement.
- **Expression curves** (§10, slice 14). Dynamics, Tension, Breathiness
  and Pitch bend live in the lane config, `VocalParams::curves`
  (`resonance-music-theory` `VocalCurves` / `VocalCurvePoint {beat,
  value, stepped}`, `#[serde(default)]`, left out of the file when
  empty): **section-relative** points, `beat` in quarter notes from the
  section start, `value` normalized `0..=1`. They are saved and undone
  with the project (BUG-50's curve part is fixed) and edited only as
  `Vocal:` automation rows on the Arrange view and through `automation.*`
  (see [automation lanes](../timeline/automation-lanes.md)) —
  `vocal.set_params` has no `curves` field. Real units: the three
  envelopes `0..=1` with `0.5` neutral, pitch bend `-50..=50` cents
  (`compose/expression.rs` `real_value`). An empty curve leaves the
  render exactly as before.
  - **Time.** Each render unit samples the curves on the f0 grid with a
    `CurveClock` built from its own first note (section-relative ticks):
    frame `i` sits at beat `first_note_beat − pad + i·timestep` at the
    section tempo. Points are linear (or stepped) between, held flat past
    the ends (`expression::sample_at_beat`).
  - **Per curve.** Pitch bend: an additive cents offset after portamento
    and vibrato, every bank. Tension / Breathiness: the model input
    (`0.5` → 0, ends → ±1) where `BankProfile::curve_supported`, else
    nothing — Lilia and Meiji take tension, no shipped bank takes
    breathiness. Dynamics: the energy input where the bank takes it
    (none does), **otherwise the post-render gain**, `±DYNAMICS_CURVE_MAX_DB`
    (12 dB) at the ends, `0.5` = 0 dB, multiplied with the velocity gain.
    So Dynamics and Pitch bend are audible on every bank.
  - **Render state.** The curves ride in the params, so the plan
    fingerprint moves on every curve edit: a GUI edit arms the debounce, a
    control edit leaves the lane `stale`. Model-input curves change the
    unit's cache key; the Dynamics gain is applied after the cache, so the
    unit **state key** hashes it (`state_key(.., curve_gain_db)`) and a
    Dynamics edit on TIGER reads stale without re-running the model
    needlessly on the units it does not touch.
- **Comfortable ranges** (warnings only, `BankProfile::comfortable_range`):
  per bank from the built-in table ([voicebank library](voicebank-library.md));
  per-speaker ranges are not known yet, so each speaker reports its bank's.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render entry | `resonance-app/src/compose/vocal_svs/{mod,render,error}.rs` | `render_vocal_clip`, `RenderedVocal`, `VocalRenderError`, `vocal_audio_start`, `SEGMENT_PAD_SEC` |
| Rests and breaths | `compose/vocal_svs/rests.rs`, `segment/duration.rs` | `RestRule`, `REST_PHONEME` (`SP`), `BREATH_PHONEME` (`AP`), `BREATH_*`, `place_breaths` |
| Velocity dynamics | `compose/vocal_svs/dynamics.rs`, `segment/{energy,tension}.rs` | `GainSpan`, `dynamics_gain_envelope`, `apply_dynamics_gain`, `velocity_gain_spans`, `velocity_gain_spans_with_breaths`, `breath_lead_secs`, `velocity_gain_db`, `velocity_modulation`, `build_energy_curve` |
| Bank facts the render reads | `compose/vocal_svs/registry/profile.rs` (registry, installs and overlay: [voicebank library](voicebank-library.md)) | `BankProfile` (`render_target`, `phoneme_name`, `language_id`, `curve_supported`, `default_speaker`, `comfortable_range`), `VoicebankMissing`, `Resonance::voicebanks` |
| Curve kinds, backend | `compose/vocal_svs/paths.rs` | `CurveKind`, `svs_backend` |
| Phoneme inventory + timing | `compose/vocal_svs/phonemes.rs`, `validate.rs` | `VoicebankPhonemes`, `nearest_substitutes`, `nearest_by_class`, `validate_for_voicebank`, `articulation_report_for` |
| Segment build | `compose/vocal_svs/segment/{duration,f0,tension,energy,overlay}.rs` | `build_segment` |
| Cache | `compose/vocal_svs/render_cache.rs` | `SvsRenderCache`, `split_render_units`, `render_units_cached`, `RenderPlan`, `RenderUnit::state_key`, `plan_unit_keys`, `model_identity` |
| Post-processing | `compose/vocal_svs/post.rs` | `collect_rest_intervals`, `apply_ap_gate`, `safety_gain_factor`, `resample_to`, `write_stereo_wav` |
| Orchestration | `resonance-app/src/update/compose/vocal_render.rs`, `vocal_render_plan.rs`, `vocal_audio_io.rs`, `vocal_audio_install.rs`, `vocal_midi_install.rs`, `vocal_control.rs`, `regenerate.rs` | `enqueue_vocal_render`, `render_target_or_missing`, `spawn_render`, `rerender_vocal_audio`, `plan_vocal_render`, `handle_vocal_audio_ready`, `reap_orphaned_takes`, `control_generate_vocal`, `regenerate_lane_as_side_effect` |
| Registry | `resonance-app/src/compose/state.rs` | `VocalAudioRegistry` (`clips`, `render_epoch`, `in_flight_render`, `clip_lyrics`, `render_cache`, `status`) |
| Render state | `compose/vocal_render_status.rs`, `update/compose/vocal_render_state.rs`, `update/compose/vocal_render_events.rs`, `update.rs` (the `settle` call) | `VocalRenderStatus`, `VocalLaneRenderState` (`severity`, `worst`), `VocalLaneErrorKind`, `lane_fingerprint`, `expected_unit_keys`, `lane_render_status`, `settle`, `auto_render_due`, `AUTO_RENDER_DEBOUNCE`, `ComposeMessage::VocalAutoRender`; render results `ready` / `failed` / `unavailable` |
| Badge | `view/compose/vocal_render_badge.rs`, `view/compose/lane_inspector/vocal/mod.rs`, `view/editor_panel.rs` | `render_card` (inspector), `render_chip` (vocal-roll toolbar), `VocalRailContext::render_status` |
| Control render state | `update/control/view_model/lane.rs`, `update/control/song.rs` | `lane_render_view`, `track_render_state` |
| Control | `resonance-app/src/update/control/vocal.rs`, `control_jobs.rs` | `render`, `checked_voicebank`, `bank_missing`, `lane_voice` |
| Vocal roll | `view/compose/vocal_roll/{build,draw}.rs` | `build_canvas`, `VocalRollCanvas::fingerprint` (hashes the bank's identity and stamp, so a rescan that changes the bank repaints the phoneme strip) |
| SVS crate | `resonance-svs/src/{pipeline,ds,config,audio}.rs`, `stages/{acoustic,vocoder,common}.rs`, `voicebank/{mod,quirks,layout,dsdict,phonetics}.rs` | `render_segments`, `PipelineArgs`, `DsSegment`, `active_backend`, `voicebank::scan`, `BankDetails`, `looks_like_bank`, `find_dsdict`, `find_library_vocoder`, `installed_vocoder_name`, `load_dsdict`, `PhonemeInventory` |

The crate also has split-pipeline stage scaffolds (`stages/{linguistic,
duration,pitch,variance}.rs`) and a CLI (`--features cli`), which the app
does not use. The registry is built on its voicebank manifest
(`voicebank::scan`).
It depends only on `resonance-music-theory`.

## Surfaces

- **GUI:** vocal lane right rail → Generate (renders after generating) and
  Re-render; Voice & delivery has the **Voicebank** and **Speaker** pick
  lists and **Library…** (see [voicebank library](voicebank-library.md)),
  and **Rest after gap** (0.125–4 beats, step ⅛) and **Velocity →
  loudness** (0–1) sliders (`SetVocalRestThresholdBeats`,
  `SetVocalVelocityDynamics`, coalesced undo); tempo mismatch warning in
  the inspector. A missing bank surfaces as the render-failure banner, and
  the lane keeps its audio. A lane whose audio is not current shows a
  **badge** at the top of its inspector body and in the vocal-roll
  toolbar: `STALE · N of M phrases changed` (warm; the card adds that the
  old take plays until a render replaces it), `RENDERING` (neutral) or
  `RENDER FAILED · <kind>` (`BAD`; the card adds the message). GUI edits
  re-render the lane by themselves after 750 ms. Expression curves are
  edited as `Vocal:` automation rows on the Arrange view (see Expression
  curves above).
- **Control API / MCP:** `vocal.render` → `mcp__resonance__vocal_render`,
  a job (`job_status` / `job_wait`). `section_id` renders one lane;
  `track_id` alone renders **all** that track's lanes; neither renders
  every vocal lane. Lanes with no notes or no lyrics are skipped (the call
  fails only if none can render). `voicebank` omitted renders each lane
  with the bank it holds; a given one (any installed or built-in id,
  case-insensitive) is stored on every lane the call covers. A lane whose
  bank or vocoder is missing fails the call with `voicebank_missing`.
  Payload: `track_ids`, `revision`. `vocal.voicebanks` and
  `vocal.voicebank_install` list and install banks
  ([voicebank library](voicebank-library.md)). `song.vocal` reports render state; banks and speakers are better
  set with `vocal.set_params` ([vocal lanes](vocal-lyrics-and-melody.md)).
  `vocal.set_params` / `vocal.get_params` carry `rest_threshold_beats`
  and `velocity_dynamics` (range-checked on the merged lane).
  `song.vocal` reports per lane `render_state`, `stale_units` and `error
  {kind, message}`, and a track-level `render_state` that is the worst
  lane's (`error > rendering > stale > not_rendered > rendered`;
  `not_rendered` for no lanes). Before vocal phase 3 the track field read
  `rendered` as soon as any lane had audio; the meaning changed within
  protocol 2 (no external clients). Control edits never auto-render.
  `control.hello` carries the optional `svs_backend` (`cpu` / `cuda` /
  `migraphx`), absent until a render has run.
- **Agent skills:** `mixing`, `spatial` read `song_vocal`; external
  `singer-songwriter` and `resonance-studio` skills drive renders.

## Bank audition

A blind listening tool for choosing which banks join the registry (spec
§12 slice 6, D30) — not a render feature itself, just a harness over the
real one. `resonance-app/tests/vocal/bank_audition.rs` (`#[ignore]`d,
test `audition`) renders one committed test phrase
(`tests/vocal/fixtures/bank_audition_song.json`: ~4 bars, `v`/`th`/`r`/`l`
clusters, a held `_` melisma, a written `AP` breath, a rest, a high and a
low note) through the real registry → resolver → segment → render path
on **every installed bank × speaker** — the same `VoicebankRoots::for_machine`
roots the app itself scans, so it picks up `$RESONANCE_SVS_MODELS_DIR` and
any bank dropped into the user voicebank folder. It checks each render's
RMS before writing it (a render under −45 dBFS fails the test loudly
rather than leaving a silent WAV in the pile) and writes plainly-named
WAVs (`<bank>__<speaker>.wav`) plus a `manifest.json` into
`$RESONANCE_AUDITION_OUT`.

`scripts/vocal-audition.py` is the entry point: it sets
`$RESONANCE_AUDITION_OUT` (default `target/audition`), runs the test,
then shuffles `manifest.json`'s entries behind random 8-character tokens,
renames the WAVs to `<token>.wav` and writes `key.json`
(token → bank/speaker) — so a listen is blind until `--reveal` prints the
key. Neither the WAVs nor the keys are committed.

```sh
RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models scripts/vocal-audition.py
# listen to target/audition/*.wav blind, then:
scripts/vocal-audition.py --reveal
```

Run once against TIGER, Lilia and Meiji (2026-10-07): 12 files (Lilia's
one voice, Meiji's four modes, TIGER's seven), 7.85 s each, all
non-silent (−20 to −27 dBFS). The listen itself and picking which
candidates (Peiton, the LUNAI English singers, Meiji v170) join the index
is Jorrit's job, not this tool's — see
[vocal follow-ups](../../intents/compose/vocal-followups.md).

The script clears only what it writes in `--out` (`*.wav`,
`manifest.json`, `key.json`) and refuses a non-empty directory holding
neither a `key.json` nor a `manifest.json`. It shuffles whenever a
`manifest.json` exists — also after a failed render, before reporting the
failure — drops plainly-named WAVs a render that died before its manifest
left, and gives every token file the same modification time, so a listing
by time does not reveal the render order.

## How it's tested

| What | Where | Run |
|---|---|---|
| Registry, install, library overlay, pickers, `vocal.voicebanks` | see [voicebank library § How it's tested](voicebank-library.md#how-its-tested) | |
| `voicebank_missing` before anything queues: GUI re-render keeps the old audio, `vocal.generate` reports `render_error`, a missing vocoder named | `tests/control/control_voicebanks.rs` (`render_with_a_missing_*`, `a_render_with_a_missing_bank_keeps_the_old_audio`, `generate_reports_the_missing_bank_…`), `control_jobs.rs` | `./scripts/run-tests.py -p resonance-app` |
| Plan, cache, IO, placement, timing, articulation | `resonance-app/tests/vocal/{vocal_render_plan,vocal_render_cache,vocal_audio_io,vocal_audio_placement,vocal_segment_timing,vocal_articulation,vocal_phoneme_accessor,vocal_expression_segment,voicebank_curve_support}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Expression curves: per-curve support, value mapping, section-relative sampling, Dynamics as gain on a bank without energy (same cache key, new state key), gain layers add in dB; the model and `plan_set_lane` (windows of different lengths) | `tests/vocal/{vocal_expression_segment,expression_curves_model}.rs`; curve edits → stale / debounce, persistence (a rendered lane with a curve reads `rendered` after save and reopen, the beat read back exactly) in `tests/control/control_vocal_curves.rs` | same |
| A Dynamics curve audibly changes a TIGER take (−6 / +6 dB per note, silence guarded) | `tests/vocal/vocal_dynamics_render.rs` `on_tiger_a_dynamics_curve_changes_the_take_through_gain` (no-op unless the models are set) | `RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models cargo test -p resonance-app --test vocal` |
| Rests (eighth rest at the default threshold, threshold flips legato/rest, articulation report follows it), breaths (gap, short gap, no gap, first note, render units keep rest + breath together) | `tests/vocal/vocal_rests_breaths.rs` | `./scripts/run-tests.py -p resonance-app` |
| Velocity gain shape and 30 ms ramp, a breath takes its note's gain, TIGER per-note RMS via the post gain, tension on Lilia, energy on a fixture bank with `use_energy_embed`, content key follows both params | `tests/vocal/vocal_velocity_dynamics.rs` | same |
| Real models: AP vs SP rest level before the gate, rest gated silent and breath heard (Lilia), TIGER velocity sweep ±6 dB | `tests/vocal/vocal_dynamics_render.rs` (no-ops unless set) | `RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models cargo test -p resonance-app --test vocal` |
| Param ranges, serde defaults; wire round-trip and range errors | `resonance-music-theory/tests/vocal_params_validate.rs`, `tests/control/control_vocal_params.rs` | same |
| Cache key follows the resolved speaker; vocal-roll fingerprint follows a rescan | `tests/vocal/vocal_render_cache.rs` (`the_key_follows_the_resolved_speaker_not_the_params`), `tests/compose/vocal_roll_fingerprint.rs` (`a_rescan_that_changes_the_bank_invalidates`) | same |
| Render state: per-lane stale / error / rendered, control edits don't render, GUI debounce (timer generations), chord edits stale not stuck, reopen reads `rendered`, keyless take reads `stale`, worst-lane summary; a whole-line shift (control: stale, GUI: armed), a TIGER velocity edit stale only with `velocity_dynamics`, `song.vocal` articulation follows the lane's rest threshold, the debounce generation survives a load, a playhead-only tick skips the settle | `tests/control/control_vocal_render_state.rs`, `control_vocal_render_state_edits.rs` (`test_finish_vocal_render` / `test_fail_vocal_render` land or fail a render through the real messages) | same |
| Badge goldens | `tests/compose/compose_vocal_render_badge.rs` → `compose_vocal_rail_stale_badge-wgpu.png`, `compose_vocal_rail_error_badge-wgpu.png`, `compose_vocal_roll_stale_badge-wgpu.png` | `RESONANCE_BLESS=1` |
| In-flight / re-render safety | `tests/compose/vocal_render_inflight_placement.rs`, `vocal_rerender_keeps_saved_audio.rs`, `vocal_tempo_mismatch_warning.rs` | same |
| Control render, `svs_backend` in hello | `tests/control/control_endpoint.rs`, `control_vocal.rs`, `control_vocal_render_all_lanes.rs`, `control_jobs.rs`, `tests/e2e_compose_via_control.rs` | same |
| SVS crate | `resonance-svs/tests/{ds_validation,mix_timeline,voicebank_manifest,voicebank_phonetics,smoke}.rs` (`read_curve_support`, two-level layout, library vocoders, `dsdict` parsing, execution-provider names); `smoke.rs::end_to_end_render` is `#[ignore]` and also checks `active_backend()` | `./scripts/run-tests.py -p resonance-svs`; by hand: `SVS_POC_VOICEBANK_DIR=… SVS_POC_DS_FILE=… cargo test -p resonance-svs --test smoke -- --ignored` |
| Bank audition: every installed bank × speaker renders the committed phrase non-silently | `tests/vocal/bank_audition.rs` (`#[ignore]`, no-ops with no `$RESONANCE_AUDITION_OUT`) | `scripts/vocal-audition.py` (see [Bank audition](#bank-audition)) |

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
- Render state, against spec §7: the debounced re-render only runs for
  lanes that already have audio (or a render in flight, or a failed one);
  a never-rendered lane waits for Generate / Re-render. A GUI edit that
  changes no unit key (a lyric-generator setting) arms nothing. The badge
  also shows `RENDERING` and an error card, not only `STALE`. A take saved
  before keys existed reads `stale`.
- A missing bank **is** an error now (spec D12): it fails the render
  naming the bank, and every path checks before queueing the render, so
  the lane keeps whatever audio it had.
- Every refusal of a GUI re-render (`rerender_vocal_audio`) is also the
  lane's render error, including an unplaced section (`no_notes`: the
  lane has no clip to render).
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
- `set_params` accepts an installed bank, one of the three built-in ids,
  or an id the plok.org index lists; the GUI picker offers installed
  English banks only. Install deviations are in the
  [voicebank library](voicebank-library.md#decisions-and-deviations).
- Render state, more against spec §7 (slice 9): the plan memo is keyed by
  a fingerprint of the plan's inputs, not a lane revision counter; the
  track-level order is `error > rendering > stale > not_rendered >
  rendered`; chord and length edits no longer render at all (they
  re-derive the melody and leave the lane stale), which replaces plumbing
  the dropped Task up; the compared keys are unit *state* keys (cache key
  + first clip tick + raw velocities when `velocity_dynamics` > 0, + the
  Dynamics curve gain), so a whole-line shift or a TIGER velocity edit
  reads stale though no cache key moves.
- Rests and breaths, against spec §8 (slice 12): a gap of **at least** the
  threshold is a rest (so a written eighth rest at ½ beat rests); a breath
  on a phrase's first note takes 0.25 s of the 0.3 s leading pad; a short
  gap lends the breath its whole length and the rest comes from the
  previous note's tail; a rest ending in a breath no longer splits render
  units. The "check first" was done: an `AP` rest renders as breath noise
  (see Measurements), so rests and pads are `SP` and only written breaths
  are `AP`. Vocal goldens were re-blessed for the shorter default rest
  (0.25 s at 120 bpm against the old fixed 0.4 s) — an approved sound
  change.
- Velocity dynamics, against spec §9 (slice 13): on a bank without energy,
  tension keeps its own `tension_velocity_amount` rather than being
  scaled by `velocity_dynamics`; the post gain is applied to the stitched
  take after the render cache (raw note velocity, not the stress-weighted
  one), while the unit content key still hashes both new params, so
  changing `velocity_dynamics` re-runs the models; energy uses the
  overlay's signed ±1 units, not dB.
- Expression curves, against spec §10 (slice 14): the old dock model's
  baseline / depth / smoothing are dropped (a curve is its points); the
  envelopes stay `0..=1` (`0.5` neutral) and pitch bend `±50` cents as in
  `expression.rs`; Dynamics on a bank without energy is a ±12 dB post
  gain. The Arrange rows and `automation.*` deviations are in
  [automation lanes](../timeline/automation-lanes.md) and the
  [automation control API](../timeline/automation-control-api.md).
- `resonance-svs` honesty (slice 2): `speedup` stays 20 — the shipped
  acoustic models take `steps`, so 50 is 1.7× *slower* (Measurements);
  the `rocm` feature became `migraphx`.
- The bank audition (slice 6, D30) is an `#[ignore]`d app test through the
  real registry → resolver → segment → render path rather than a
  `resonance-svs` example, so it reuses `VoicebankRoots::for_machine`
  and can drive Meiji (the example cannot).
- Saved floats read back exactly: the workspace builds `serde_json` with
  `float_roundtrip`. The default parser lands one ULP off for some values
  (a curve point 49 ticks into a section is one), which made a reopened,
  rendered lane read `stale`.

## Known gaps

- The energy path (velocity → energy) runs on no shipped bank and has not
  been heard; it uses the overlay's signed ±1 convention, not dB.
- The expression curves' baseline / depth / smoothing from the old dock
  model (doc #154) are gone: a curve is just its points. Nothing derives a
  baseline from the melody.
- No harmony / doubled voices —
  [vocal-harmony-doubling](../../intents/compose/vocal-harmony-doubling.md).
- Inference is CPU-only in the app (`ExecutionProvider::Cpu` is hard-coded
  even though the crate supports the `cuda` and `migraphx` features).
  Neither GPU path has been exercised on a GPU; the prebuilt ONNX Runtime
  that `download-binaries` fetches has no MIGraphX provider, so a
  `migraphx` build needs `ORT_LIB_LOCATION` pointing at an ORT built with
  it (without one, registration fails and the session falls back to CPU,
  which is checked).
- Which banks a user should get (the blind listen of the audition
  renders) and the publishing of the plok.org index are open —
  [vocal follow-ups](../../intents/compose/vocal-followups.md); install
  gaps are in the [voicebank library](voicebank-library.md#known-gaps).
- The `render_resonance_vocal` example cannot drive Meiji: its G2P copy
  emits bare ARPAbet without Meiji's `en/` prefix or language ids, so the
  `speedup` measurement has no Meiji row.
- Breaths are rendered but not drawn in the vocal roll.
- `speedup` is a misnomer for the shipped banks (it is the `steps` count);
  the knob is not exposed to the user.

## Measurements

**AP vs SP in a rest**, 2026-10-07 (`vocal_dynamics_render.rs`,
`an_ap_rest_breathes_and_an_sp_rest_does_not`): two `la` half notes
around a 2 s rest at 120 bpm, the rest sung as `AP` or as `SP`, RMS of
the rest's middle 1.6 s measured **before** the rest gate:

| Bank | `AP` rest | `SP` rest | note |
|---|---|---|---|
| Lilia | −50.7 dBFS (−37.6 in the 0.3 s pads next to the notes) | −85.1 dBFS | ≈ −20 dBFS |
| TIGER | −64.0 to −65.4 dBFS | −78 to −89 dBFS | ≈ −27 dBFS |

`AP` comes out as audible breath noise, `SP` as near-silence, so plain
rests and pads became `SP` and only written breaths are `AP`. (Before this
change every `AP`/`SP` span was gated to silence, so rests were silent
either way; the switch decides what a breath may sound like.) A rendered
0.35 s breath on Lilia measured −37 dBFS against −21 dBFS for the note
after it; TIGER's velocity sweep at full amount measured −6.0 / 0 /
+6.0 dB per note against amount 0.

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
