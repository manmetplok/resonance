# EQ (`com.resonance.eq`)

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.4 (slice W8: M/S, Tilt/Lift+Dip/Air, auto-gain, dynamic bands).

An 8-band parametric EQ for tracks, busses and the master. Each band is a
bell, shelf, cut or one of three one-knob "warmth" shapes. It can filter the
stereo pair, only the mid or only the side, and it can be dynamic: it cuts
only while its own region (or a sidechain key) is loud. A live pre/post
spectrum sits behind the response curve, and an optional auto-gain trims the
output so a boost can be judged at matched level.

## Behaviour

- **Bands.** Eight bands, `band0`…`band7`, all off by default, with
  default frequencies 40, 120, 250, 600, 1500, 4000, 9000 and 16000 Hz and
  default kinds Low Cut, Low Shelf, Bell ×4, High Shelf, High Cut.
- **Kinds** (`band{n}_kind`, labels in `band::KIND_LABELS`): Bell, Low
  Shelf, High Shelf, Low Cut, High Cut, Tilt (pivot at `freq`, top moves
  by `gain`, bottom by the opposite), LF Lift+Dip (Pultec-style: lift at
  `freq`, dip at 3.5× by half as much, fixed Q), Air (a broad first-order
  high shelf centred on `freq`; near the top of the range its corner sits
  above the audible band, 40 kHz for +12 dB at 20 kHz).
- **Cut slopes** (`band{n}_slope`): 12 / 24 / 48 dB/oct, used only by Low
  Cut and High Cut. The cascades are true Butterworth (−3 dB at the
  cutoff, maximally flat), so each stage's Q comes from the slope and the
  band's own `q` is unused: the strip shows no Q on a cut band and the
  node's scroll does nothing there.
- **M/S** (`band{n}_ms`): Stereo (default), Mid or Side. A Mid/Side band
  splits `m = (L+R)/2`, `s = (L−R)/2`, filters one component and
  recombines. A Stereo band runs the original arithmetic exactly.
- **Dynamic bands** (`band{n}_dyn_on`, `_dyn_threshold`, `_dyn_ratio`,
  `_dyn_attack`, `_dyn_release`): a detector filter matched to the band
  (band-pass for a bell, low-pass under a low shelf, high-pass over a high
  shelf or Air) feeds a peak detector, a soft-knee gain computer and
  ballistics. The band is re-voiced at `gain − GR` every
  `DYN_UPDATE_SAMPLES`. Only Bell, Low Shelf, High Shelf and Air support it
  (`BandKind::supports_dyn`). A bell at 0 dB with dynamics on is a pure
  de-harsh cut: it does nothing until its region is loud.
- **Sidechain** (`band{n}_dyn_sc`): the dynamic band detects on the
  plugin's key input instead of its own signal when a key is connected.
  The EQ declares `SIDECHAIN_INPUT = Some(2)` and is listed in
  `first_party::SECONDARY_KEY_PLUGINS`, so an unqualified "key this track"
  prefers a compressor or gate over it.
- **Output.** `output_gain` (smoothed in linear space) and `auto_gain`
  (off by default). Auto-gain is static: `dsp::static_gain_db` estimates
  the curve's loudness change from the parameters alone, with Mid/Side
  bands weighted by an assumed programme split. It never pumps.
- **Parameter changes** update biquad coefficients once per block and keep
  the stage state, so sweeps do not click. A kind change restarts the
  band's stages and crossfades from the old ones over about 5 ms.
- **Latency:** none.
- **Param layout:** the original six per-band params come first, then
  `output_gain`, `auto_gain`, then the W8 per-band block, then `dyn_sc`.
  Indices of older params never moved. Widening `band{n}_kind` from 0–4 to
  0–7 changes what a normalised automation value lands on. That was left
  unmigrated on purpose.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin + key port | `plugins/resonance-eq/src/lib.rs` | `ResonanceEq`, `process_with_key` |
| Params | `plugins/resonance-eq/src/params.rs` | `EqParams`, `make_band!`, `NUM_BANDS`, `PARAM_COUNT` |
| Band shapes | `plugins/resonance-eq/src/band.rs` | `BandKind`, `BandMs`, `BandSlope`, `KIND_LABELS` |
| DSP | `plugins/resonance-eq/src/dsp.rs` | biquad cascade, dynamic-band detector, `static_gain_db` |
| Analyzer | `plugins/resonance-eq/src/analyzer.rs` | pre/post taps on `resonance-metering` spectrum workers |
| Editor curve | `plugins/resonance-eq/src/response_curve.rs`, `src/editor/response.rs`, `src/editor/nodes.rs` | response plot, draggable band nodes |
| Editor strip | `plugins/resonance-eq/src/editor/control_strip.rs`, `src/editor/app.rs` | per-band controls, header, preset bar |
| Presets | `plugins/resonance-eq/src/presets.rs`, `presets/*.json` | 11 factory presets |
| Shared DSP | `resonance-dsp` | `Biquad`, `dynamics::{soft_knee_gain_reduction_db, Ballistics}`, `db_to_linear` |

The analyzer is display-only and RT-safe: `process()` pushes a mono
downmix into a preallocated lock-free ring. A background worker per tap
owns the FFT and publishes a 1/6-octave peak-hold snapshot through an
`ArcSwap`. `initialize()` spawns the workers and dropping the plugin joins
them.

## Editor

A response plot with the live spectrum behind it (pre-EQ or post-EQ, one at
a time), one node per enabled band and a control strip below. Node
interactions: drag moves frequency and gain (frequency only for cuts),
scrolling adjusts Q (not on cuts or the one-knob kinds), right-click opens type/slope/disable, double-click
toggles the band. The strip uses the shared `editor_widgets::param_slider`.
The `freq` params parse both "1200 Hz" and "1.2 kHz" from text.

## Factory presets

Kick — Punch, Kick — Sub, Snare — Crack, Snare — Body, Bass — Tight,
Bass — Warm, Guitar — Body, Guitar — Air, Vocal — Clarity, Synth — Wide,
Master — Polish.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). Keys are `band{n}_*` with
  n = 0–7, plus `output_gain` and `auto_gain`. Keying a dynamic band:
  `track.set_sidechain` → `mcp__resonance__track_set_sidechain`, then
  `band{n}_dyn_sc` On.
- **Agent skills:** `mixing/references/character.md` (two keys blocks:
  static/dynamic cuts and unmasking, and the tilt/shelf/Lift+Dip/Air warmth
  moves), `spatial/references/width.md` §5 (side-only shelves and cuts).

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-eq` |
| W8 band types, M/S, auto-gain | `tests/warmth_bands.rs` → `golden/warmth_bands.f32` | same |
| Dynamic bands, sidechain | `tests/dynamic_bands.rs` → `golden/dynamic_bands.f32` | same |
| Pre-W8 state renders unchanged | `tests/legacy_state.rs` → `golden/legacy_state.f32` | never re-bless for a "transparent" change |
| Butterworth cuts | `tests/butterworth.rs` | same |
| Kind change does not click | `tests/kind_change.rs` | same |
| Output gain smoothing | `tests/output_gain.rs` | same |
| Analyzer: display-only, no heap on the audio thread, worker teardown | `tests/analyzer.rs`, `tests/analyzer_rt.rs`, `tests/analyzer_teardown.rs` | same |
| Editor binding, typed kHz entry | `tests/editor_controls.rs`, `tests/freq_typed_entry.rs` | same |
| Plugin smoke, skill keys | `tests/plugin.rs`, `tests/skill_keys.rs` | same |

Not covered: there is no `state.rs` save/load round trip for the EQ (the
compressor lacks one too), and no pixel snapshot of the editor.

## Decisions and deviations

- The dynamic band is a separate, simpler feature from the mastering
  chain's [de-harsh](deharsh.md) suppressor (decision D4 of the warmth spec).
- New params were appended, not interleaved, so old automation lanes and
  saved projects keep their indices.

## Known gaps

- Only one spectrum (pre or post) is drawn at a time, although both are
  computed.
- Picking a sidechain key has no GUI; it is set through the control API
  (see [sidechain-routing.md](../mixer/sidechain-routing.md)).
