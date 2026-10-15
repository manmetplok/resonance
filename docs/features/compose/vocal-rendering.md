# Vocal rendering (singing synthesis)

> Verified against master @ `acac5ea3` + the per-placement render state and the
> take laid at a new placement (BUG-73) on 2026-10-15; the rest @ `f9438e38` + the `singing` skill and the
> chord-change vocal melody flow (BUG-70) on 2026-10-13.
> Spec history: [`docs/history/specs/vocal-mcp-control-and-quality.md`](../../history/specs/vocal-mcp-control-and-quality.md) §7–§10 (slices 2, 6, 9, 12–14); crate notes `resonance-svs/README.md`, `resonance-svs/NOTES.md` (partly stale, see below); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane's notes and lyrics are sung by a DiffSinger voicebank. The app
turns the lane's derived MIDI clip into a DiffSinger score, runs the
bank's own duration and pitch models (where it ships them) plus the
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
  (`VocalParams::voicebank`; a new lane sings TIGER `mystic`) and its
  voice mode by `speaker` (growl, vocal fry and whisper modes are marked
  as effect modes, not lead voices). Which banks exist, what each can do
  (speakers, phoneme inventory, language ids, curve support, comfortable
  range, terms) and
  how they are installed from plok.org is the
  [voicebank library](voicebank-library.md). The render asks
  `BankProfile` (`VoicebankRegistry::profile`) for the bank's model files
  (`render_target`), phoneme keys, language ids and curve support.
- **Missing bank.** `vocal.render` checks every lane it covers before
  anything starts and fails with `voicebank_missing`, naming the bank (or
  the bank's missing vocoder), how to install it (`vocal_voicebank_install`,
  or **Library…** beside the lane's Voicebank picker), where a user bank
  goes, and `vocal_voicebanks`. A test app has no banks, so a new lane's
  TIGER reads `tiger (not installed)` there. Every render path checks the bank right after
  planning and **before** queuing anything
  (`enqueue_vocal_render` → `render_target_or_missing`): a GUI render
  keeps the old audio, raises the banner and fails any job waiting on the
  lane with `voicebank_missing`; `vocal.generate` writes the melody,
  skips its auto-render and reports `render_error {kind:
  voicebank_missing}`; a chord or length edit keeps, re-fits or re-derives
  the lane's melody without rendering at all (`regenerate_lane_as_side_effect` →
  `vocal_melody::on_chord_change`, see
  [vocal lyrics and melody](vocal-lyrics-and-melody.md#chord-and-length-changes-under-a-melody-bug-70)),
  so it raises no banner and keeps whatever
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
  is one of its layers) and mapped onto the bank's inventory, spelt the
  way the bank's dictionary `replacements:` say (Lilia sings English `w`
  as her `L`, `t` as `t0`, `v` as her own `w`; see the
  [voicebank library](voicebank-library.md)): a phone the
  bank lacks takes the bank's hint (no shipped bank has one), else its
  voiced/voiceless partner, else the nearest phone of the same
  articulatory class (`nearest_by_class`); anything not ARPAbet or with
  nothing near enough blocks the render with an error naming the note,
  the syllable and the word. Slurred notes hold the previous vowel.
- **Score building (app side).** `build_segment` allocates each note's
  time over its phonemes (per-class consonant targets, the vowel taking
  the rest, minimum articulation times per phoneme class), builds a
  piecewise f0 with portamento (`portamento_ms`) and optional vibrato, a
  tension curve from velocity/contour, and adds 0.3 s `SP` padding at both
  ends. The consonant targets (`phonemes.rs` `target_band`: stops 56–96
  ms, fricatives 88–152, nasals 72–120, approximants 64–112 across
  `consonant_emphasis`) are 1.6× the first, hand-set table — about what
  the banks' own duration models predict (§ Measurements). A syllable's
  onset consonants are sung **before** its beat so the vowel lands on it:
  taken from the previous note's tail (`apply_onset_lead_in`), and on a
  segment's first note from the leading pad, up to 0.2 s (0.1 s of
  silence stays; `PAD_ONSET_MAX_SEC`). The `gender` input is the timbre
  chip's value (Warm 0, Edged +0.05, Airy +0.20, Bright +0.30) times the
  bank's `gender_scale` (TIGER 0.25): the default is the bank's own voice.
  Rests split the clip into independent **render units**
  (`split_render_units`).
- <a id="bank-models"></a>**The bank's own models** (`bank_timing`,
  `bank_pitch`, lane params, default on; `segment/score.rs`,
  `bank_render.rs`, `resonance_svs::predict`). Where the bank ships them
  ([voicebank library](voicebank-library.md): all three shipped banks
  have `dsdur` and `dspitch`, Lilia and Meiji `dsvariance`), the render
  re-times and re-pitches each unit the way OpenUtau's DiffSinger renderer
  does, just before the acoustic model runs:
  - *Score.* Beside every rule segment the planner builds a `BankScore`
    from the same phoneme track: **groups** start at anchors — a note's
    first vowel, every rest and written breath, the trailing pad — at the
    time the rule layout starts that phoneme (the vowel's start is the
    beat); a syllable's onset consonants sit in the group before (sung
    before the beat), group 0 is the leading pad plus the first onset. A
    consonant–glide–vowel onset (`b r ay`) is no exception: the glide is
    sung before the beat with the consonant, and the group and the pitch
    model's note start on the vowel (see Decisions). A held note (`_`)
    folds its repeated vowel into the vowel before and its coda closes the
    group, but stays a note of its own for the pitch model (a rest exactly
    when it sings no vowel — a hold after a rest or breath is sung).
  - *Which models a unit uses* is decided at planning, per render unit
    (`build_unit_segment` → `BankModelPaths::covering`): the models the
    lane's flags ask for, less any whose phoneme list lacks one of the
    unit's phonemes (TIGER's pitch model lacks `io`, `iz`, `rs`, which its
    duration model has); the variance model goes with the pitch model.
    Such a unit sings the rules' part — synthesized f0 with vibrato — and
    its key says so (`RenderUnit::bank_models`).
  - *Duration* (`bank_timing`): the linguistic encoder in word mode
    (`tokens`, `languages` where the model takes them, `word_div`,
    `word_dur`) and the duration model (`ph_midi`, the speaker's
    embedding where it takes one; a duration model with one speaker
    serves every mode) predict frames per phoneme; each group is stretched
    to fill its anchor-to-anchor span, so every vowel still lands on its
    beat and the unit keeps its length. No phoneme comes out under one
    frame: one squeezed below (a group shorter than its phonemes) takes
    the time from its group's longest phoneme, else the unit's. Language
    ids go to each model through the language names (its own
    `languages.json` against the acoustic model's; `ModelFacts::language`).
    Group 0's onset keeps its
    predicted length up to 0.25 s (`MODEL_LEAD_MAX_SEC`, 50 ms of the pad
    stays silent; the rules cap it at 0.2 s) and becomes the take's
    pre-roll.
  - *Pitch* (`bank_pitch`): the encoder in phoneme mode (`ph_dur`
    frames) and the pitch model (`note_midi` / `note_dur` / `note_rest`
    with rest notes over the pads and gaps, pitch from the nearest sung
    neighbour, `expr` 1, `retake` all, 10 steps) give fractional MIDI per
    frame; the Pitch bend curve is added in cents. Vibrato, vibrato rate
    and portamento are not heard: the model sings its own vibrato and
    glides.
  - *Variance* (with `bank_pitch`, only when the acoustic model takes a
    curve it predicts — tension on Lilia and Meiji): its predicted
    tension (20 steps, on the final pitch) with the lane's own tension
    curve (slider, velocity / contour modulation, Tension curve) added on
    top. Under the bank's pitch the contour modulation reads the written
    pitch (`build_step_curve`), not the synthesized line, so vibrato and
    portamento reach neither the take nor the key. Energy / breathiness /
    voicing predictions are not used (no shipped acoustic model takes
    them).
  - *Fallback.* A bank without a model, a flag off, or a unit whose
    phonemes a model lacks (above) leaves that part to the rules: rule
    timing, synthesized f0; with neither model run the unit is the rule
    segment exactly. A model that fails to **load or run** at render time
    falls back the same way (a warning says which, once per process), but
    the unit then does not sound like its key says: it is stitched into
    the take and **not cached**, and the take records it under a key no
    plan has (`UnitAudio::fallback`, `FALLBACK_KEY_MARK`), so the lane
    reads `stale` and the next render tries it again.
  - *Steps.* Pitch 10, variance 20 (OpenUtau's defaults; Meiji's notes ask
    10 for pitch), acoustic 20 as before.
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
  `resonance_svs::pipeline::Renderer` — combined acoustic ONNX (mel)
  then vocoder ONNX — on CPU, `speedup` 20, `depth` 1000
  (`ACOUSTIC_STEPS`, `ACOUSTIC_DEPTH`). Sessions come from a process-wide
  **session cache** (`resonance_svs::stages::session_cache`): keyed by the
  model file's path, length and modification time, the provider and the
  stage type, grouped by bank, the two most recently used banks kept, all
  dropped when a voicebank rescan finds a bank added, removed or changed.
  So a re-render of one edited unit loads nothing (§ Measurements); a
  first render loads on the first unit that misses the cache. A cached
  session is shared as `Arc<Mutex<…>>`: two lanes rendering with the same
  bank at once take turns per model run. Loading reads the model, rewrites
  it and drops the original before building the session (a model with
  nothing to rewrite is built from its file), so our side never holds
  more than two copies. All three shipped
  acoustic models are continuous-acceleration exports that take a `steps`
  input rather than `speedup`, and the pipeline passes the `speedup` value
  as the step count: the app renders with **20 sampling steps**. TIGER and
  Meiji take a fractional `depth`, so 1000 is clamped to their `max_depth`
  (0.6); Lilia takes no `depth`. `SP` spans
  are gated to silence (`AP` breaths are not), the velocity gain and then
  a safety gain are applied, mono becomes stereo and
  is resampled to the engine rate.
- <a id="determinism"></a>**Repeatable renders.** Every acoustic, pitch
  and variance model draws diffusion noise inside its graph
  (`RandomNormalLike`, on Meiji also `RandomUniformLike`), and the
  NSF-HiFiGAN vocoders draw their harmonic phases and excitation noise
  (`RandomUniform`, `RandomNormalLike`); ONNX Runtime seeded those from the
  clock, so the same score rendered differently every time.
  `resonance_svs::stages::noise::seed_random_ops` rewrites the model bytes
  before the session is built (protobuf wire format, no ONNX crate): each
  such op becomes `Reshape(Slice(noise_k, 0, prod(shape)), shape)` over a
  new float input, fed from a SplitMix64 / Box–Muller stream keyed by a
  seed. The seed is `RenderUnit::seed`: the unit's content key without the
  lane params that only act after the models (`velocity_dynamics` on a
  bank without energy) and without the key-only versioning (render
  revision, file digests, step counts, measurement knobs: `seed_identity`
  keeps the identity seeding started with), so the same model input gives
  the same samples in any process, in any order. Each noise input is fed
  exactly what its op draws: a fixed-shape op (the vocoders'
  `RandomUniform([1, harmonics])`) its size, a `*Like` op the count the
  graph reports (an extra output), learned per session from the first
  runs (`SeededSession`, `stages/seeded.rs`); the stream is prefix-stable,
  so the length never changes the values. Checked: two renders of the
  audition on all 12 bank × speaker voices in separate processes are
  byte-identical, and the TIGER audition renders byte-identically before
  and after the exact sizing and the session cache.
  **Caveats.** Only a model's top-level random ops are rewritten. One in a
  `Loop` / `If` / `Scan` body or a model-local function (it would need
  other noise per iteration), one drawing a non-float type, a model
  older than opset 12 or with external weight files is loaded as it is
  and renders **not repeatably**; so is a model whose rewrite fails to
  parse or build (it is loaded from its file, unseeded). Each says so in
  a warning once per process (`SeededSession::repeatable`,
  `Renderer::repeatable`). None of the shipped banks has any of these. A
  unit whose bank model fell back at render time is never cached (above).
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
  reports "N of M segments changed". Keys never need a model run: when a
  bank model is in use the key hashes the `BankScore` it is fed, which of
  the lane's models the unit uses and the bank models' config paths and
  file digests (`model_identity`), and with the bank's pitch it hashes the
  pitch bend instead of the synthesized f0 (so vibrato and portamento,
  which are not heard, move no key — the tension contour reads the
  written pitch then). `model_identity` also carries the render revision
  (`resonance_svs::pipeline::SVS_RENDER_REV`, bumped when the same input
  would render differently), a digest of the acoustic and vocoder model
  files' lengths and times (`InstalledBank::render_digest`, read at scan;
  the bank's stamp covers the model files, so a replaced model is
  re-read), the step counts and the measurement knobs
  (`RESONANCE_BANK_EXPR`, `RESONANCE_BANK_VARIANCE`,
  `RESONANCE_SVS_SEED_SALT`). Each cached unit keeps
  the silence its leading pad kept (`UnitAudio::lead_sec`), so the
  pre-roll follows the duration model's onset on a cache hit too. The key covers what renders the
  unit plus two lane params: the acoustic and vocoder config paths, the
  speaker key the lane's speaker resolves to, and the bank version — a
  rescan that resolves the same params to another speaker or release
  re-renders — and `rest_threshold_beats` and `velocity_dynamics`
  (`lane_identity`), so changing either re-renders and moves the plan.
  The velocity gain itself is applied after the cache, to the stitched
  take. `bank_timing` / `bank_pitch` change the key only on a bank that
  ships the model.
- **Install.** The WAV (`vocal_*.wav`) goes to the project's `audio/`
  folder, or `$TMP/resonance_vocal` for an unsaved project. One audio clip
  per placement starts at the section start plus the lane's first-note
  offset (`vocal_audio_start`), trimmed past the padding — less the
  take's **pre-roll** (`RenderedVocal::pre_roll_frames`: what the first
  note sings before its beat, its onset consonants or a written breath),
  which the clip starts early to keep; a first note at sample 0 trims it
  instead. **Behaviour change:** a breath written before a phrase's first
  note used to sit in the trimmed pad and was never heard; it is pre-roll
  now, so re-rendering an existing project makes such a breath audible
  (and moves the take's clip start earlier by the pre-roll). The
  install mirrors the clip app-side (`ClipState`, trims included) as it
  sends the load — the engine's `ClipImported` echo carries no trims, so
  a take mirrored from the echo alone was saved untrimmed and reopened
  playing its leading pad, late. Each render bumps
  a per-lane **render epoch**; a result arriving for an old epoch, a moved
  or deleted placement is discarded or re-targeted. The previous take is
  **not** torn down when a render is queued: it plays until the new take
  replaces it, placement by placement, in `handle_vocal_audio_ready` (a
  failed or superseded render leaves it in place). A save copies live
  clips to `audio/clip_<id>.wav`, writes each take's unit keys as
  `ProjectClip::vocal_render_units` and the placement it plays as
  `ProjectClip::vocal_take` (`ProjectVocalTake { definition_id,
  placement_id }`), and reaps orphaned takes in the project's own
  `audio/`.
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
    reads `stale`; so does a lane whose plan cannot be built any more,
    and a lane with a placement that has no installed take: `stale` with
    `stale_units` 0 until a render fans out to every placement (BUG-73).
    Placing the section again lays a `rendered` lane's take at the new
    placement, so this happens only when it cannot — the take was not
    current, or the placement plays at another tempo
    ([sections and placements](sections-and-placements.md)).
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
    and on undo/redo, so a reopened project reads `rendered`. That
    reconcile (`ComposeState::rebuild_vocal_audio_clips`) rebuilds the
    lane → take map, which the next render's replace (D22) needs to find
    the old take: a clip saved with its `vocal_take` goes back to that
    placement's lane while the placement exists; one saved without it
    (before owners were saved) is matched by its first note —
    `start_sample + pad − trim_start` (pad = `vocal_svs::pad_frames`) must
    land, within 2 samples, on a placement's `vocal_audio_start` for the
    lane's lead-in (the first note's tick in its MIDI clip). That holds
    with pre-roll and when clamped at sample 0, since trim + pre-roll =
    pad. Any other clip on a vocal track (the user's own recording) is
    never claimed. It used to claim only clips starting on a placement's
    bar line, which no take with pre-roll or a lead-in does: such a lane
    read `not_rendered` after a reload or any undo, lost its keys, and
    its next render summed a second take with the first.
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
  - *Chord and length edits* keep, re-fit or re-derive the lane's melody
    without rendering (`vocal_melody::on_chord_change`), so they follow the
    same rule
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
- **Comfortable ranges** (warnings only, `BankProfile::comfortable_range`,
  read for the speaker the lane sings with by `song.vocal` and its
  per-note `out_of_range` — `lane_comfortable_range`: the lane's speaker,
  else the bank's default for its voice, as `render_target` resolves
  it): per bank from the built-in table — TIGER C3–A4, Lilia
  D3–A♯4, Meiji B2–F♯4 ([voicebank library](voicebank-library.md)); a
  speaker may have its own (`speaker_ranges`), none does yet.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render entry | `resonance-app/src/compose/vocal_svs/{mod,render,error}.rs` | `render_vocal_clip`, `RenderedVocal`, `VocalRenderError`, `vocal_audio_start`, `SEGMENT_PAD_SEC` |
| Rests and breaths | `compose/vocal_svs/rests.rs`, `segment/duration.rs` | `RestRule`, `REST_PHONEME` (`SP`), `BREATH_PHONEME` (`AP`), `BREATH_*`, `place_breaths` |
| Velocity dynamics | `compose/vocal_svs/dynamics.rs`, `segment/{energy,tension}.rs` | `GainSpan`, `dynamics_gain_envelope`, `apply_dynamics_gain`, `velocity_gain_spans`, `velocity_gain_spans_with_breaths`, `breath_lead_secs`, `velocity_gain_db`, `velocity_modulation`, `build_energy_curve` |
| Bank facts the render reads | `compose/vocal_svs/registry/profile.rs` (registry, installs and overlay: [voicebank library](voicebank-library.md)) | `BankProfile` (`render_target`, `phoneme_name`, `language_id`, `curve_supported`, `default_speaker`, `comfortable_range`), `VoicebankMissing`, `Resonance::voicebanks` |
| Curve kinds, backend | `compose/vocal_svs/paths.rs` | `CurveKind`, `svs_backend` |
| Phoneme inventory + timing | `compose/vocal_svs/phonemes.rs`, `validate.rs` | `VoicebankPhonemes`, `nearest_substitutes`, `nearest_by_class`, `validate_for_voicebank`, `articulation_report_for` |
| Segment build | `compose/vocal_svs/segment/{duration,f0,tension,energy,overlay,gender}.rs` | `build_segment`, `build_unit_segment`, `UnitSegment` |
| Bank models (app) | `compose/vocal_svs/segment/score.rs`, `compose/vocal_svs/bank_render.rs`, `registry/profile.rs` | `build_bank_score`, `EntryRole`, `MODEL_LEAD_MAX_SEC`, `refine_segment`, `apply_prediction`, `unit_seed`, `BankProfile::{bank_models, active_bank_models}` |
| Bank models (crate) | `resonance-svs/src/predict/{mod,config,score,run}.rs` | `BankModels` (`load`, `predict`), `PredictOptions`, `Prediction` (`fallback`), `BankScore`, `align_durations`, `pitch_notes`, `fit_frames`, `discover`, `BankModelPaths::{covers, covering, facts}`, `ModelFacts`, `language_map`, `DurationModel`, `PitchModel`, `VarianceModel` |
| Seeded noise, sessions | `resonance-svs/src/stages/{noise,seeded,session_cache,common}.rs`, `pipeline.rs` | `seed_random_ops`, `SeededModel`, `noise`, `NoiseSpec`, `NoiseSize`, `SeededSession` (`load`, `run`, `noise_lengths`, `repeatable`), `session_cache::{get_or_load, invalidate}`, `ModelSource`, `build_session_from`, `warn_once`, `files_digest`, `render_files`, `SVS_RENDER_REV` |
| Cache | `compose/vocal_svs/render_cache.rs` | `SvsRenderCache`, `split_render_units`, `render_units_cached`, `render_units_cached_with`, `UnitAudio` (`fallback`), `FALLBACK_KEY_MARK`, `RenderPlan`, `RenderUnit::{state_key, seed, score, bank_models}`, `plan_unit_keys`, `model_identity`, `seed_identity` |
| Post-processing | `compose/vocal_svs/post.rs` | `collect_rest_intervals`, `apply_ap_gate`, `safety_gain_factor`, `resample_to`, `write_stereo_wav` |
| Orchestration | `resonance-app/src/update/compose/vocal_render.rs`, `vocal_render_plan.rs`, `vocal_audio_io.rs`, `vocal_audio_install.rs`, `vocal_midi_install.rs`, `vocal_control.rs`, `regenerate.rs` | `enqueue_vocal_render`, `render_target_or_missing`, `spawn_render`, `rerender_vocal_audio`, `plan_vocal_render`, `handle_vocal_audio_ready`, `reap_orphaned_takes`, `control_generate_vocal`, `regenerate_lane_as_side_effect` |
| Registry | `resonance-app/src/compose/state.rs` | `VocalAudioRegistry` (`clips`, `render_epoch`, `in_flight_render`, `clip_lyrics`, `render_cache`, `status`) |
| Render state | `compose/vocal_render_status.rs`, `update/compose/vocal_render_state.rs`, `update/compose/vocal_render_events.rs`, `update.rs` (the `settle` call) | `VocalRenderStatus`, `VocalLaneRenderState` (`severity`, `worst`), `VocalLaneErrorKind`, `lane_fingerprint`, `expected_unit_keys`, `lane_render_status`, `settle`, `auto_render_due`, `AUTO_RENDER_DEBOUNCE`, `ComposeMessage::VocalAutoRender`; render results `ready` / `failed` / `unavailable` |
| Badge | `view/compose/vocal_render_badge.rs`, `view/compose/lane_inspector/vocal/mod.rs`, `view/editor_panel.rs` | `render_card` (inspector), `render_chip` (vocal-roll toolbar), `VocalRailContext::render_status` |
| Control render state | `update/control/view_model/lane.rs`, `update/control/song.rs` | `lane_render_view`, `track_render_state` |
| Control | `resonance-app/src/update/control/vocal.rs`, `control_jobs.rs` | `render`, `checked_voicebank`, `bank_missing`, `lane_voice` |
| Vocal roll | `view/compose/vocal_roll/{build,draw}.rs` | `build_canvas`, `VocalRollCanvas::fingerprint` (hashes the bank's identity and stamp, so a rescan that changes the bank repaints the phoneme strip) |
| SVS crate | `resonance-svs/src/{pipeline,ds,config,audio}.rs`, `stages/{acoustic,vocoder,common}.rs`, `voicebank/{mod,quirks,layout,dsdict,phonetics}.rs` | `render_segments`, `Renderer`, `PipelineArgs`, `DsSegment`, `active_backend`, `voicebank::scan`, `BankDetails`, `looks_like_bank`, `find_dsdict`, `find_library_vocoder`, `installed_vocoder_name`, `load_dsdict`, `PhonemeInventory` |

The crate also has a CLI (`--features cli`), which the app does not use;
its old split-pipeline stage scaffolds became `predict/`. The registry is built on its voicebank manifest
(`voicebank::scan`).
It depends only on `resonance-music-theory`.

## Surfaces

- **GUI:** vocal lane right rail → Generate (renders after generating) and
  Re-render; Voice & delivery has the **Voicebank** and **Speaker** pick
  lists and **Library…** (see [voicebank library](voicebank-library.md)),
  and **Rest after gap** (0.125–4 beats, step ⅛) and **Velocity →
  loudness** (0–1) sliders (`SetVocalRestThresholdBeats`,
  `SetVocalVelocityDynamics`, coalesced undo) and the **Timing from the
  voicebank** / **Pitch from the voicebank** toggles under the Speaker
  picker (`ToggleVocalBankTiming` / `ToggleVocalBankPitch`), each shown
  only when the lane's bank ships that model
  (`VoicebankChoice::{has_timing_model, has_pitch_model}`). While the
  bank's pitch model sings, the Vibrato, Vibrato rate and Portamento
  sliders are muted (`theme::muted_slider_style`, still adjustable)
  under the hint "Vibrato and portamento: the voicebank sings its own";
  tempo mismatch warning in
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
  and `velocity_dynamics` (range-checked on the merged lane) and
  `bank_timing` / `bank_pitch` (default true, read as true from older
  projects); `vocal.voicebanks` reports each bank's `models {duration,
  pitch, variance}`. `song.vocal`'s per-note `duration_ms` / `too_short`
  and the articulation report stay the rule layout's estimates (see
  Decisions).
  `song.vocal` reports per lane `render_state`, `stale_units` and `error
  {kind, message}`, and a track-level `render_state` that is the worst
  lane's (`error > rendering > stale > not_rendered > rendered`;
  `not_rendered` for no lanes). Before vocal phase 3 the track field read
  `rendered` as soon as any lane had audio; the meaning changed within
  protocol 2 (no external clients). Control edits never auto-render.
  `control.hello` carries the optional `svs_backend` (`cpu` / `cuda` /
  `migraphx`), absent until a render has run.
- **Agent skills:** the agent plugin's `singing` skill drives renders
  (batch, pre-flight, one `vocal_render`, read `render_state`) and the
  Whisper check of a soloed bounce; `mixing`, `spatial` read `song_vocal`;
  external `singer-songwriter` and `resonance-studio` defer to `singing`.

## Bank audition

A blind listening tool for choosing which banks join the registry (spec
§12 slice 6, D30), and the intelligibility measure — not a render
feature itself, just a harness over the real one.
`resonance-app/tests/vocal/bank_audition.rs` (`#[ignore]`d, test
`audition`) renders one committed phrase
(`tests/vocal/fixtures/bank_audition_song.json`) through the real
registry → resolver (with the bank's dictionary, as
`plan_vocal_render` resolves) → segment → render path on **every
installed bank × speaker** — the same `VoicebankRoots::for_machine`
roots the app itself scans, so it picks up `$RESONANCE_SVS_MODELS_DIR` and
any bank dropped into the user voicebank folder.

The phrase is two plain lines at 120 bpm, "Walk with me a·long the riv·er
/ AP we will sing _ un·til the morn·ing": 16 syllables split with `·` and
one `_` melisma on 17 notes, an `AP` breath in the half-beat rest between
the lines. Before anything renders the test checks the lyric fills the
notes the way `song.vocal`'s `counts_mismatch` does (a hermetic test,
`the_audition_phrase_fills_its_notes_and_fits_every_builtin_voice`, checks
it without models too). The melody is written in C (C4–C5) and
**transposed per bank × speaker** so its duration-weighted centre sits in
the middle of that voice's comfortable range (TIGER −7, Lilia −5, Meiji
−9). Each render's RMS is checked before it is written (under −45 dBFS
fails the test), and the WAVs (`<bank>__<speaker>.wav`) go to
`$RESONANCE_AUDITION_OUT` with a `manifest.json` carrying the lyric text
and each transposition. `RESONANCE_AUDITION_BANKS=tiger,lilia` limits the
banks, `RESONANCE_AUDITION_DUMP=1` writes each take's render units
(`*.units.json`: phonemes, durations, notes, language ids, curves — what
the model was fed), `RESONANCE_AUDITION_FIXTURE=<file>` sings another
phrase of the same shape — the held-out phrase is committed as
`tests/vocal/fixtures/bank_audition_heldout.json` — and
`RESONANCE_AUDITION_CENTRE=<midi>` centres every voice's melody on that
note instead of in its range (the transposition sweep that calibrates a
range).

`scripts/vocal-audition.py` is the entry point: it sets
`$RESONANCE_AUDITION_OUT` (default `target/audition`; `--banks a,b` and
`--fixture F` set the two knobs above), runs the test,
then shuffles `manifest.json`'s entries behind random 8-character tokens,
renames the WAVs to `<token>.wav` and writes `key.json`
(token → bank/speaker, transposition, text) — so a listen is blind until
`--reveal` prints the key. Neither the WAVs nor the keys are committed.

`scripts/vocal-intelligibility.py <dir>` transcribes every take in an
audition directory (plain or shuffled) with Whisper `small.en` and prints
word accuracy (1 − WER against the lyric) per bank × speaker, `--json` for
the per-file results. Whisper is optional: without it the script re-runs
itself through `uv` with a CPU PyTorch (first run downloads ~0.5 GB).
Singing is hard for a speech recogniser. Renders are bit-identical run to
run now (§ Repeatable renders), but a take still depends on its noise draw
(one seed to the next moves a take by up to ±10 points), so compare means
over several seeds (`RESONANCE_SVS_SEED_SALT`), not single files.
`RESONANCE_AUDITION_BANK_TIMING=0` / `RESONANCE_AUDITION_BANK_PITCH=0`
render the audition with a flag off, `RESONANCE_BANK_VARIANCE=0` skips the
variance model and `RESONANCE_BANK_EXPR=<0..1>` sets the pitch model's
expressiveness (A/B knobs), and `RESONANCE_BANK_DUMP=<dir>` writes what
the bank models were fed and returned per unit (phonemes, groups, notes,
durations, pitch, tension). The manifest records each take's
`render_sec`.

```sh
RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models scripts/vocal-audition.py
# listen and score each file right after it plays (1-5, r replay, n note,
# s skip, q quit; resumable, scores in scores.json, names revealed at the end):
scripts/vocal-audition-score.py target/audition
# or listen by hand, then:
scripts/vocal-audition.py --reveal
scripts/vocal-intelligibility.py target/audition
```

The script clears only what it writes in `--out` (`*.wav`,
`manifest.json`, `key.json`) and refuses a non-empty directory holding
neither a `key.json` nor a `manifest.json`. It shuffles whenever a
`manifest.json` exists — also after a failed render, before reporting the
failure — drops plainly-named WAVs a render that died before its manifest
left, and gives every token file the same modification time, so a listing
by time does not reveal the render order. One run renders every installed
voice in one process (41 voices with the LUNAI singers: about 1.3 s each
after the first of a bank). Which candidates stay in the index is
Jorrit's call — see
[vocal follow-ups](../../intents/compose/vocal-followups.md).

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
| Render state: per-lane stale / error / rendered, control edits don't render, GUI debounce (timer generations), chord edits stale not stuck, reopen reads `rendered`, keyless take reads `stale`, worst-lane summary, a placement added after the render reads `stale` (BUG-73); a whole-line shift (control: stale, GUI: armed), a TIGER velocity edit stale only with `velocity_dynamics`, `song.vocal` articulation follows the lane's rest threshold, the debounce generation survives a load, a playhead-only tick skips the settle | `tests/control/control_vocal_render_state.rs`, `control_vocal_render_state_edits.rs` (`test_finish_vocal_render` / `test_fail_vocal_render` land or fail a render through the real messages) | same |
| Badge goldens | `tests/compose/compose_vocal_render_badge.rs` → `compose_vocal_rail_stale_badge-wgpu.png`, `compose_vocal_rail_error_badge-wgpu.png`, `compose_vocal_roll_stale_badge-wgpu.png` | `RESONANCE_BLESS=1` |
| In-flight / re-render safety | `tests/compose/vocal_render_inflight_placement.rs`, `vocal_rerender_keeps_saved_audio.rs`, `vocal_tempo_mismatch_warning.rs` | same |
| Control render, `svs_backend` in hello | `tests/control/control_endpoint.rs`, `control_vocal.rs`, `control_vocal_render_all_lanes.rs`, `control_jobs.rs`, `tests/e2e_compose_via_control.rs` | same |
| SVS crate | `resonance-svs/tests/{ds_validation,mix_timeline,voicebank_manifest,voicebank_phonetics,smoke}.rs` (`read_curve_support`, two-level layout, library vocoders, `dsdict` parsing, execution-provider names); `smoke.rs::end_to_end_render` is `#[ignore]` and also checks `active_backend()` | `./scripts/run-tests.py -p resonance-svs`; by hand: `SVS_POC_VOICEBANK_DIR=… SVS_POC_DS_FILE=… cargo test -p resonance-svs --test smoke -- --ignored` |
| Bank audition: the phrase fills its notes and fits every built-in voice once transposed (hermetic); every installed bank × speaker renders it non-silently | `tests/vocal/bank_audition.rs` (`audition` is `#[ignore]`, no-ops with no `$RESONANCE_AUDITION_OUT`) | `scripts/vocal-audition.py`, `scripts/vocal-intelligibility.py` (see [Bank audition](#bank-audition)) |
| Bank models, model-free: the score's groups (onsets before the beat, a glide onset too, the note and its group on the vowel, a hold folded into its vowel but kept as a note, breath and rest groups), the flags picking models, keys following the flags only where the bank has the models, vibrato moving no key under the bank's pitch while pitch bend does — also on a bank with tension; a unit whose phonemes the pitch model lacks planned without it (its key hearing vibrato), a replaced acoustic / vocoder / pitch model file moving the keys | `tests/vocal/vocal_bank_models.rs` | `./scripts/run-tests.py -p resonance-app` |
| A unit rendered with a fallback is stitched but not cached and recorded under a marked key; the next render retries it | `tests/vocal/vocal_render_cache.rs` (`a_unit_rendered_with_a_fallback_…`) | same |
| Voice card: both model toggles and the muted vibrato / portamento sliders with their hint on a bank with models (golden), neither toggle nor hint on a bank without | `tests/compose/compose_rail_collapse.rs` (`vocal_rail_installed_bank_with_speakers` → `compose_vocal_rail_installed_bank-wgpu.png`, `vocal_rail_bank_without_models_hides_the_model_toggles`) | `RESONANCE_BLESS=1` |
| Bank model inputs and alignment (word_div / word_dur / ph_midi, onset kept and capped, groups filled, every phoneme at least a frame — squeezed groups and a leading pad under 50 ms — pitch notes with rest fill, a hold after a rest sung, frame fitting, the rule fallback with no model) | `resonance-svs/tests/ds_validation.rs` (`bank_score`) | `./scripts/run-tests.py -p resonance-svs` |
| Discovery: per-model phonemes, language ids mapped by name, file digests; a unit's models narrowed to those that know its phonemes | `resonance-svs/tests/voicebank_manifest.rs` (`the_banks_own_models_are_found_…`, `a_unit_uses_only_the_bank_models_…`) | same |
| Seeded noise: a hand-written ONNX model's `RandomNormalLike` / `RandomUniform` rewritten to fed noise, same seed same output across sessions, another seed other output, prefix-stable stream, a model without random ops not copied; a session feeding a fixed op its size and learning a `*Like` op's count; a `Loop` body's random op counted and the model loaded unseeded; a `*Like` without `dtype` seeded over floats and falling back to the file over a double; opset 11, external data and a double `dtype` left unseeded; a truncated model an error, not a panic | `resonance-svs/tests/smoke.rs` | same |
| Real models: each shipped bank's duration model fills every group (vowels on their anchors, length kept) and differs from the rules, its pitch model sings each long note within 50 cents with no octave jump, both repeatable; a Lilia take renders bit-identically twice, is audible and keeps its rest silent | `tests/vocal/vocal_bank_models.rs` (`real::*`, no-ops unless set) | `RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models cargo test -p resonance-app --test vocal` |
| Dictionary replacements: parsing the section alone, the scan spelling phones through them, read-back to ARPAbet; Lilia spelt `L`/`t0`/…, installed and not; her dictionary entries read back | `resonance-svs/tests/{voicebank_manifest,voicebank_phonetics}.rs`, `tests/vocal/{bank_phonemisation,vocal_phoneme_accessor,vocal_segment_pronunciation}.rs` | `./scripts/run-tests.py` |
| Timbre → gender: Warm neutral on every bank, TIGER a quarter of the swing | `tests/vocal/vocal_expression_segment.rs` | same |
| First-note onset out of the leading pad (vowel on the beat, 0.1 s of silence kept, segment length unchanged); the install starts a take with pre-roll early, or trims it at sample 0 | `tests/vocal/vocal_segment_timing.rs`, `tests/compose/vocal_render_inflight_placement.rs` | same |
| Per-bank comfortable ranges; a lane without a speaker is checked against its default speaker's range (fixture bank with `speaker_ranges`) | `tests/vocal/voicebank_registry.rs`, `tests/vocal/vocal_articulation.rs` (`a_lane_without_a_speaker_is_checked_against_its_default_speakers_range`) | same |
| A take with pre-roll and a lead-in stays its lane's after a reload (saved owner, and by first note without one, and clamped at sample 0) and after undo/redo: reads `rendered`, keeps its keys, is saved trimmed, and a re-render replaces it | `tests/control/control_vocal_take_reload.rs` | same |

Not covered: no hermetic test runs a bank's ONNX models (the seeded-noise
tests run a hand-written one); real audio quality is checked by ear and
Whisper only. The app tests drive the pipeline with injected
results.

## Decisions and deviations

- Durations and f0 come from the voicebank's own duration and pitch
  models by default (`bank_timing`, `bank_pitch`); the app's rules stay as
  the fallback and as the plan (§ Bank models). Before this, the app
  computed both: the combined-acoustic path was the one that worked end
  to end in the PoC.
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
  (after stripping `en/`, and reading a replaced symbol back: Lilia's `L`
  is `w`); other multi-language extras (`dx`, `d0` without a replacement)
  make the word fall through to CMU, because everything after the
  resolver works in ARPAbet (see [pronunciation](pronunciation.md)).
- The bank models run on the render thread, inside the unit render,
  never on the planning thread (which builds segments to fingerprint
  every lane): the keys hash what they are fed, and their output is a
  pure function of that and the seed. (The earlier decision to keep the
  1.6× consonant table instead of `dsdur` rested on one phrase; on the
  held-out phrase the models measure far better, § Measurements.)
- With `bank_pitch`, `vibrato`, `vibrato_rate` and `portamento_ms` are
  ignored rather than layered on: the pitch model already sings vibrato
  and note transitions, and adding ours would double them; `expr` stays 1
  (as trained). The params stay on the lane and are heard again with
  `bank_pitch` off or on a bank without a pitch model. Pitch bend is
  additive on either path.
- The variance model's tension is used with the lane's own tension added
  on top, in the model's units (the slider's ±1 and the modulators are
  small against the predicted ±5–9 swing): the lane's settings still lean
  the delivery. Measured: Lilia held-out 79 → 95 %, Meiji 95 → 98 % with
  the variance model on (3 seeds).
- A held note sings one vowel spanning its notes (DiffSinger's training
  convention) when a bank model runs; the rule path still repeats the
  vowel per note.
- `song.vocal`'s per-note `duration_ms` / `too_short` and the
  articulation report keep the rule layout's estimates: the duration
  model's output needs a model run, and the planner never runs one. The
  rule allocation now sits close to the models' (consonants 1.6×), so the
  estimate is a fair guide; the model's actual timing is not reported.
- Seeding mixes nothing lane-specific in: two lanes singing the same
  notes with the same bank, speaker and params render identical audio (a
  future doubling feature has to salt the seed). Kept deliberately: the
  seed is a function of what the models are fed, which is what makes a
  take reproducible.
- Which bank models a unit uses is decided at planning from the models'
  phoneme lists, not discovered when a model rejects a phoneme at render
  time: the key then always says what the unit sings. A model that fails
  at render time anyway makes its unit uncacheable rather than caching
  audio under a key that promises the model's output.
- The seed keeps the identity it had when seeding was introduced
  (`seed_identity`), while the key also hashes the render revision, the
  model file digests and the knobs: re-keying a unit (a new revision, a
  re-stamped model) re-renders it but with the same noise, so a fix that
  does not touch the model input leaves takes bit-identical.
- Bank model language ids are mapped through the language names of each
  model's `languages.json` rather than assumed equal to the acoustic
  model's (they are on the shipped banks); an id a model does not list
  goes through unchanged.
- A consonant–glide–vowel onset's glide is sung before the beat like any
  onset consonant; the note's group and the pitch model's note start on
  the vowel. Against OpenUtau, which opens the note's group with the
  glide: built that way (the group on the beat, the glide sung from it)
  the held-out phrase measured TIGER 92 % against 96 % (§ Measurements).
  The branch before the review started the group at the glide's rule
  start, a little before the beat, so its note began early.
- Model warnings (a model not used, a failed prediction, unseeded random
  ops, a voicebank folder skipped) are logged once per process per file
  and message (`warn_once`): rescans and renders repeat them otherwise.
- `RESONANCE_SVS_SEED_SALT=<n>` (measurement only) mixes `n` into every
  seed, to measure the spread across noise draws.
- Sampling steps stay 20 (`speedup`): 50 measured no better (TIGER 85 /
  83 %, Meiji 79 / 78 %, Lilia 87 / 75 % on the held-out phrase) at 1.7×
  the time, Meiji's "Acoustic 50" advice notwithstanding.
- TIGER's acoustic config says `mel_base: '10'` and its `tgm_hifigan`
  vocoder names none; converting the mel to natural log before the
  vocoder (×ln 10) made it worse (33 → 17 %, 75 → 8 %), so the mel goes
  through as is. Lilia's bundled `mm_finetune` vocoder and the PC-NSF-
  HiFiGAN next to it measured the same (71 / 78 %, within noise); the
  bundled one stays.
- The `velocity` input all three acoustic models take is DiffSinger's
  speed embed (`use_speed_embed`), fed 1.0 (neutral); Meiji's "velocity
  curve" is the same input.
- `set_params` accepts an installed bank, one of the seven built-in ids,
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
- The phrase-initial onset lead is capped at 0.2 s on the rule path
  (`PAD_ONSET_MAX_SEC`) and 0.25 s under the duration model: a longer
  cluster (`str`, ~0.28 s) is shortened to fit.
- `song.vocal` reports the **rule** timing: its per-note `duration_ms` /
  `too_short` and the articulation report are the rule layout's
  estimates, not what the bank's duration model sings (that needs a model
  run, which planning never does).
- A random op inside a model's control-flow body or function is not
  seeded: such a model renders, but not repeatably (a warning says so).
  None of the shipped banks has one.
- The session cache holds the two most recently used banks' sessions
  (~0.5–0.7 GB resident per bank on the shipped ones); there is no
  setting for it, and two lanes rendering with one bank at once take
  turns per model run.
- On the held-out phrase the remaining errors are mostly "street" heard
  "streets" before the breath (TIGER) and the first word on some seeds.
- Short passing notes (≈ 0.27 s) often sit 50–190 cents off their
  written pitch under the bank's pitch model: it glides through them as
  a singer would.
- The `render_resonance_vocal` example cannot drive Meiji: its G2P copy
  emits bare ARPAbet without Meiji's `en/` prefix or language ids, so the
  `speedup` measurement has no Meiji row.
- Breaths are rendered but not drawn in the vocal roll.
- `speedup` is a misnomer for the shipped banks (it is the `steps` count);
  the knob is not exposed to the user.

## Measurements

**All 41 voices**, 2026-10-13 (the LUNAI English singers installed:
Katyusha, Keiro Revenant, Liam Thorne, Allen Crow v170): Whisper
`small.en` word accuracy of one take per voice (default seed, both bank
models on), each transposed into its comfortable range, on the audition
phrase and the held-out phrase; and the share of notes of at least 0.2 s
whose median pitch (pyin over the middle 60 % of the note) is within 50
cents of the written note. Whisper and pitch both read the rendered WAV.

| Bank | Mode | Transposed | Audition | Held-out | Pitch ≤ 50 c (aud / held) |
|---|---|---|---|---|---|
| tiger | disco | -7 | 100 % | 93 % | 100 / 94 % |
| tiger | electric | -7 | 100 % | 100 % | 100 / 75 % |
| tiger | fresh | -7 | 100 % | 93 % | 100 / 100 % |
| tiger | glam | -7 | 100 % | 93 % | 76 / 94 % |
| tiger | mystic | -7 | 100 % | 93 % | 94 / 94 % |
| tiger | royal | -7 | 100 % | 100 % | 82 / 100 % |
| tiger | vinyl | -7 | 100 % | 100 % | 94 / 100 % |
| lilia | — | -5 | 100 % | 93 % | 94 / 69 % |
| meiji | hunter | -9 | 100 % | 93 % | 94 / 94 % |
| meiji | lilith | -9 | 100 % | 100 % | 88 / 100 % |
| meiji | phantom | -9 | 100 % | 100 % | 82 / 100 % |
| meiji | standard | -9 | 100 % | 100 % | 88 / 100 % |
| katyusha | delicate | -1 | 100 % | 100 % | 65 / 94 % |
| katyusha | lullaby | -1 | 100 % | 100 % | 88 / 100 % |
| katyusha | standard | -1 | 100 % | 100 % | 71 / 94 % |
| katyusha | strong | -1 | 100 % | 100 % | 100 / 94 % |
| keiro_revenant | aggressive | -7 | 100 % | 100 % | 82 / 81 % |
| keiro_revenant | cold | -7 | 100 % | 100 % | 94 / 94 % |
| keiro_revenant | fragile | -7 | 100 % | 100 % | 88 / 88 % |
| keiro_revenant | growl | -7 | 100 % | 100 % | 82 / 100 % |
| keiro_revenant | mellow | -7 | 100 % | 100 % | 82 / 100 % |
| keiro_revenant | native | -7 | 100 % | 100 % | 94 / 94 % |
| keiro_revenant | solid | -7 | 100 % | 100 % | 88 / 94 % |
| keiro_revenant | standard | -7 | 100 % | 93 % | 88 / 94 % |
| keiro_revenant | vf | -7 | 100 % | 100 % | 65 / 44 % |
| keiro_revenant | whisper | -7 | 100 % | 100 % | 0 / 0 % |
| liam_thorne | aggressive | -10 | 100 % | 100 % | 88 / 81 % |
| liam_thorne | crispy | -10 | 100 % | 100 % | 76 / 94 % |
| liam_thorne | deep | -10 | 100 % | 100 % | 100 / 88 % |
| liam_thorne | gentle | -10 | 100 % | 93 % | 82 / 100 % |
| liam_thorne | growl | -10 | 100 % | 100 % | 88 / 94 % |
| liam_thorne | powerful | -10 | 100 % | 100 % | 94 / 100 % |
| liam_thorne | rock | -10 | 100 % | 100 % | 88 / 94 % |
| liam_thorne | standard | -10 | 100 % | 100 % | 88 / 94 % |
| liam_thorne | tired | -10 | 100 % | 100 % | 88 / 94 % |
| liam_thorne | vf | -10 | 100 % | 100 % | 88 / 94 % |
| allen_crow | gentle | -1 | 92 % | 100 % | 88 / 94 % |
| allen_crow | heavy | -7 | 100 % | 100 % | 100 / 88 % |
| allen_crow | open | -1 | 100 % | 100 % | 88 / 94 % |
| allen_crow | standard | -7 | 100 % | 93 % | 94 / 94 % |
| allen_crow | whisper | -7 | 92 % | 93 % | 0 / 0 % |

Per bank (mean over its modes, audition / held-out): TIGER 100 / 96 %,
Lilia 100 / 93 %, Meiji 100 / 98 %, Katyusha 100 / 100 %, Keiro Revenant
100 / 99 %, Liam Thorne 100 / 99 %, Allen Crow 97 / 97 %. No voice falls
under 85 % on either phrase, so nothing in the pipeline needed fixing for
the new banks beyond what made them render at all: their 384-wide
speaker embeddings (see [voicebank library](voicebank-library.md)). The two whisper modes (Keiro's, Allen's)
are understood but unvoiced, so their pitch reads 0 %; vocal fry
(Keiro's `vf`, 65 / 44 %) hits its notes loosely by design. The low
Katyusha pitch figures on the audition phrase (delicate 65 %, standard
71 %, against 94 % held-out) are notes she scoops into, not a wrong
register.

How the LUNAI ranges were chosen (built-in table comments carry the
numbers per bank): every mode of Katyusha, Keiro Revenant, Liam Thorne,
Allen Crow and Meiji sang both phrases centred on G2, A♯2, C♯3, E3, G3,
A♯3, C♯4, E4, G4, A♯4, C♯5 and E5 (`RESONANCE_AUDITION_CENTRE`; 792
takes). Whisper barely separates them below the top: the bank means stay
94–100 % from G2 up to A♯4 for Keiro, Liam and Allen (to C♯5 for
Katyusha, to G4 for Meiji) and
fall to 77–86 % on E5. Pitch accuracy sets the low end: Katyusha's
standard and strong modes hit 36–42 % of their notes centred on G2
against 85–100 % from E3 up; growl modes lose their low notes (Keiro
27 % on G2); Allen Crow's open and gentle 60–91 % below E3, which matches
LUNAI's note that both are "intended for notes above C4". Ranges then
follow the voice types the research notes give (Katyusha soprano/alto
G3–D5, Keiro tenor C3–A4, Liam baritone/bass A2–F4) inside the band
both measures hold; Allen Crow, with no published range, got C3–A4 and
his open and gentle modes G3–D5.

**Review fixes** (session cache, exact noise sizing, per-unit model
coverage, glide anchors, tension from the written pitch), 2026-10-08,
release build, one process per bank (a models root holding only that
bank), mean of 2 runs. "First" is a fresh process's first render of a
two-phrase take (two units); "re-render" is the same take after an edit
that changes one of its units (mean of 4 edits), i.e. an interactive
re-render: `vocal_bank_models::real::measure_one_unit_rerender`
(`#[ignore]`d; peak RSS is `VmHWM` reset before each render).

| Bank | First render (before → after) | Peak RSS, first | One-unit re-render | Peak RSS, re-render | Resident after |
|---|---|---|---|---|---|
| TIGER | 1.23 → 1.22 s | 1272 → 1240 MiB | 0.78 → 0.39 s | 1618 → 768 MiB | 683 → 719 MiB |
| Lilia | 1.33 → 1.31 s | 1127 → 1083 MiB | 0.84 → 0.45 s | 1372 → 732 MiB | 672 → 694 MiB |
| Meiji | 0.93 → 0.91 s | 798 → 778 MiB | 0.61 → 0.28 s | 999 → 577 MiB | 499 → 539 MiB |

A re-render loads nothing now (the sessions are cached), which halves it
and halves its peak; the bank's sessions stay resident (+20–50 MiB over a
process that dropped them and kept the allocator's pages). A first render
still peaks at the model loads, ONNX Runtime's own copies included; our
side holds at most the read model and its rewrite at once.

Renders are bit-identical across processes after the fixes too (the
audition on all 12 voices, rendered twice), and the TIGER audition —
whose phrase has no consonant–glide–vowel onset and whose bank takes no
tension — renders byte-for-byte what the branch did before the fixes:
the exact noise sizing, the session cache and the new keys leave the
audio alone.

Whisper `small.en`, the same seeds before and after (salts 0–7; the
audition phrase on salts 0–2):

| Bank | Audition, branch → fixes | Held-out, branch | Held-out, glide opening the note's group (OpenUtau) | Held-out, fixes (glide before the beat) |
|---|---|---|---|---|
| TIGER (7 modes, 56 takes) | 100 → 100 % | 95.3 % | 92.2 % | 96.2 % |
| Meiji (4 modes, 32 takes) | 100 → 100 % | 96.4 % | 95.3 % | 96.9 % |
| Lilia (8 takes) | 100 → 100 % | 96.4 % | 97.3 % | 97.3 % |

The middle column is the reading of OpenUtau's layout first built for
the review (the glide opens the note's group on the beat): the held-out
phrase's first word, "bright", came out as "red" / "bread" / "Brett" on
more TIGER takes, so the glide is sung before the beat like any onset
consonant (§ Decisions). With it the fixes measure at or above the
branch on every bank (differences of a point are one word in a take or
two).

**Bank models**, 2026-10-08: Whisper `small.en` word accuracy, mean over
3 seeds × every speaker (TIGER 7, Meiji 4; Lilia 11 seeds), the audition
phrase and the held-out phrase; "master" is `e3d187e9` (unseeded, so its
3 takes are 3 noise draws too; Lilia 11 takes), "after" is this build
with both flags on.

| Bank | Audition master | Audition after | Held-out master | Held-out after | After, the default seed (one take) |
|---|---|---|---|---|---|
| TIGER (7 modes) | 100 % | 100 % | 85 % (81–88) | 94 % (86–100) | 100 / 95 % |
| Meiji (4 modes) | 97 % | 100 % | 76 % (69–88) | 95 % (90–100) | 100 / 93 % |
| Lilia | 86 % | 100 % | 76 % | 97 % | 100 / 100 % |

A/B on the same build (3 seeds each; Lilia n = 3, so ±10 points):

| Configuration | TIGER main / held | Meiji main / held | Lilia main / held |
|---|---|---|---|
| Rules, seeded (flags off) | 98 / 87 % | 92 / 74 % | 97 / 79 % |
| Duration model only | 99 / 95 % | 97 / 86 % | 72 / 100 % |
| Pitch (+ variance) model only | 100 / 88 % | 97 / 91 % | 100 / 88 % |
| Both, variance off | — | 100 / 95 % | 100 / 79 % |
| Both (default) | 99 / 95 % | 100 / 98 % | 92 / 95 % |

The duration model carries most of the held-out gain on TIGER, the pitch
and variance models most of Meiji's; both together are best on the
held-out phrase for every bank.

Pitch sanity (`RESONANCE_BANK_DUMP`, both phrases, 12 voices, 360 notes
of at least 0.2 s): median note error 9 cents, mean 21, 323 of 360 within
50 cents over the note's vowel, no frame-to-frame jump over 6 semitones;
26 of the 37 misses are short passing notes (0.26–0.29 s) the model
glides through, the rest two longer notes (0.43 s, 0.83 s) some voices
scoop into. Every group
started exactly on its anchor and every unit kept its length.

Render time (release, idle machine, the 8.35 s audition phrase, per take
including session loads, mean of 2 runs): wall time of one process rendering every speaker of
a bank, divided by its speakers.

| Bank | master | this build, flags off | this build, bank models |
|---|---|---|---|
| TIGER | 2.27 s | 2.18 s | 2.43 s |
| Meiji | 1.48 s | 1.36 s | 1.73 s |
| Lilia | 2.17 s | 1.99 s | 2.66 s |

Loading the acoustic and vocoder sessions once per render instead of
once per unit pays for the seeded noise; the bank models add 0.25–0.7 s
per render (RTF 0.21–0.32 on 8.35 s of audio), most of it loading their
four to six sessions. An edit re-renders only the units it changed and
loads the models only when one misses the cache.


**Intelligibility**, 2026-10-08: Whisper `small.en` word accuracy
(1 − WER) of the [bank audition](#bank-audition) phrase, through the real
render path, mean of three takes per bank × speaker (TIGER 7 modes, Meiji
4, Lilia 1; one take moves ±10 points, the renders are not bit-identical).
"Held-out" is a second phrase not used while tuning ("Bright stars fall on
the qui·et street / AP and I still hear you call·ing _ me", 112 bpm).
"Before" is master `c6cc8fd5` singing the same phrases (old ranges: TIGER
transposed −3, Lilia −1, Meiji −6).

| Bank | Audition before | Audition after | Held-out before | Held-out after |
|---|---|---|---|---|
| TIGER (all modes) | 31 % (19–50) | 99 % (97–100) | 52 % (29–69) | 84 % (81–88) |
| Meiji (all modes) | 82 % (75–86) | 99 % (97–100) | 70 % (52–83) | 77 % (64–86) |
| Lilia | 31 % | 86 % | 43 % | 76 % |

Per mode after, audition: TIGER disco/fresh/mystic/royal/vinyl 100 %,
electric/glam 97 %; Meiji lilith/phantom/standard 100 %, hunter 97 %.
Before, TIGER sang something else entirely ("open the door", "what with
me was the one to live for").

What each fix bought, A/B on the same render units (a Python replica of
the acoustic + vocoder step, several takes per arm):

| Change | Evidence |
|---|---|
| Gender (formants) neutral by default; TIGER ¼ swing | TIGER, 7 modes × 2: gender −0.15 (old Warm) 39 %, −0.05 77 %, 0 92–98 %, −0.0375 93 %, +0.075 93 %, +0.15 71 %. Meiji 89 % at −0.15 vs 91 % at 0; Lilia 69 → 79 %. |
| Lilia spelt through her dictionary's replacements | Lilia 4–12 % → 69–71 % (6 takes). |
| Consonant targets 1.6× | TIGER 92 → 98 %, Meiji 91 → 97 %, Lilia 67 → 90 % (8 takes); running the banks' duration models instead: 94 / 92 / 92 %. On the audition phrase the models gave `k` 111 ms, `s` 155, `l` 125–139, `m` 108–196 where the old table gave 48–75. |
| First note's onset before the beat | Both phrases, 3 takes: TIGER 97 → 99 / 82 → 88 %, Meiji 98 → 97 / 70 → 83 %, Lilia 81 → 89 / 81 → 67 % (Lilia n = 3 per arm). |
| Ranges | Transposition sweep: TIGER 90–95 % anywhere from D3 to F4 (pitch is not what hurt it); Lilia 54 / 67 / 88 / 100 % centred on A4 / E4 / B3 / F♯3; Meiji 88 % on E4 or B3, 98 % on F♯3. |

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
