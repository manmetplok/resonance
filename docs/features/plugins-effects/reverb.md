# Reverb (`com.resonance.reverb`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/reverb-algorithms.md` (R0–R9; §5.3 as-built deviations); `docs/history/specs/warmth-width-depth.md` §6.4 (return channel, slice W8).

An algorithmic reverb with nine room types: Classic, Plate, Room, Chamber,
Hall, Ambience, Spring, Nonlinear and Shimmer. It also has the
return-channel moves a mix would otherwise need a second plugin for: wet
EQ before the tank, ducking of the wet return from a sidechain key or the
dry input, and an early-reflections-to-tail balance. Pre-delay and decay can
follow the host tempo. A mix engineer picks a type first and tunes second.
The skills do the same, and they check the tail length by number with the
`decay` meter detail.

## Behaviour

- **Algorithm** (`algorithm`; labels `ALGORITHM_LABELS`). A new instance
  runs **Room** (`Algorithm::DEFAULT`). A saved state without the key loads
  as **Classic** (`params::upgrade_state`, the plugin's `STATE_UPGRADE`
  hook, which runs on every load path), so old projects sound unchanged.
  - *Classic*: the original tank (4-step diffusion network → FDN loop,
    parallel ER), unchanged.
  - *Plate*: Dattorro figure-of-eight with per-line absorption. Below a
    0.87 s decay the tank shrinks, overriding `size`. The onset is an
    8-stage allpass cascade.
  - *Room* / *Chamber*: `ShoeboxEr` early reflections into a 16-line FDN
    with lines of 6.4–75 ms. Chamber is a Room voicing: a smaller, harder
    box with denser second-order reflections, slightly longer lines and
    little modulation; its extra warmth comes from its presets.
  - *Hall*: sparse ER over 30–120 ms, 16-line FDN of 40–200 ms lines,
    `SmoothRandom` modulation, and `tail_build`, which spreads when energy
    enters the loop.
  - *Ambience*: a dense ER cluster and a very short 8-line FDN, for a few
    percent on a bus. Its EDT, not its T60, carries the space.
  - *Spring*: two dispersive springs (`spring_tension` sets the chirp,
    `spring_drip` adds a transient-gated drip), mono in, no low bass.
  - *Nonlinear*: a dense burst shaped by `nl_shape` (Gated, Reverse, Flat)
    over `nl_length`, retriggered by input transients.
  - *Shimmer*: the Hall with pitch shifters in the loop (`shimmer_pitch`:
    +12, +7, +5, −12, +19, +24; `shimmer_amount`), low-passed and capped so
    the loop cannot run away, even frozen.
- **Shared room params:** `predelay`, `er_level`, `er_time`, `size`,
  `decay`, `damping`, `diffusion`, `mod_rate`, `mod_depth`, `width`, `mix`,
  `freeze`, plus the decay shape `low_decay_mult`, `low_xover`,
  `high_decay_mult`. Params an algorithm does not use are greyed out in
  the editor, never hidden.
- **Switching.** Every engine is built in `initialize`; a switch never
  allocates. It is a 50 ms fade (`SWITCH_FADE_MS`) that ramps the outgoing
  engine's input and output down and the incoming engine's input up. A
  switch asked for during a fade waits (newest wins). While Freeze is on, a
  switch waits until Freeze is released, unless both engines ignore Freeze
  (Spring, Nonlinear).
- **Freeze** on the FDN engines is a 100 ms ramp to a lossless loop, the
  same on engage and release. Fresh input during freeze cannot build up.
- **Tempo sync.** `predelay_sync` (Off, 1/128 … 1/8) and `decay_sync`
  (Off, 1/4, 1/2, 1 bar, 2 bars, 4 bars) override the knobs when the host
  gives a tempo, and fall back to them otherwise.
- **Return channel** (all no-ops by default): `wet_hpf_on/_freq`,
  `wet_lpf_on/_freq`, `wet_filter_slope` (12 or 18 dB/oct), applied
  *before* the tank. `duck_amount`, `duck_threshold`, `duck_attack`,
  `duck_release` drive `resonance_dsp::dynamics::Ducker` from the
  sidechain key, or from the dry input when no key is connected. The key
  never reaches the output. `er_tail_balance` crossfades early and late.
- **Sidechain.** `SIDECHAIN_INPUT = Some(2)`, listed in
  `first_party::SECONDARY_KEY_PLUGINS`.
- **Latency:** none.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin | `plugins/resonance-reverb/src/lib.rs` | `ResonanceReverb` |
| Params, legacy upgrade | `plugins/resonance-reverb/src/params.rs` | 35 params (`PARAM_COUNT`), `ALGORITHM_LABELS`, `upgrade_state`, `SHIMMER_PITCH_LABELS`, `NL_SHAPE_LABELS`, `WET_SLOPE_LABELS` |
| Tempo sync | `plugins/resonance-reverb/src/sync.rs` | `PREDELAY_SYNC_LABELS`, `DECAY_SYNC_LABELS` |
| Chain (pre-delay, return EQ, balance, width, duck, mix) | `src/dsp/{chain,return_eq,duck,er,diffusion,fdn,modulation}.rs` | |
| Engines | `src/dsp/algo/{classic,plate,room,chamber,hall,ambience,spring,nonlinear,shimmer}.rs` (+ `room/`, `hall/`, `shimmer/`) | `Algorithm`, `Algorithm::BUILT` |
| Switch | `src/dsp/algo/switch.rs` | `EngineBank`, `SWITCH_FADE_MS` |
| Editor state | `src/viz.rs` | meters, tank energies, ER taps, tail-RMS ring |
| Editor | `src/editor/{mod,center,controls,impulse_view,meters,tank_view}.rs`, `tank_view/{ring,plate,spring}.rs` | |
| Presets | `src/presets.rs`, `presets/*.json` | 20 factory presets |
| Shared DSP | `resonance-dsp/src/reverb/` | `Fdn`, `FdnConfig`, `Absorption`, `DecayBands`, `Allpass`, `DispersiveAllpass`, `ShoeboxEr`, `SmoothRandom`; also `DelayLine`, `Biquad`, `Lfo`, `OnePole`, `dynamics::Ducker` |
| Decay measurement | `resonance-metering/src/decay.rs`, `decay/` | EDT/T30 harness used by tests and by `meter.measure` |

## Editor

Header: name, algorithm selector, preset bar, live readouts, freeze
indicator. Centre: the impulse-tail view on the left, a per-algorithm tank
view on the right (an energy ring for the FDN engines, the figure-of-eight
for Plate, the echo train for Spring), and stereo peak meters. Bottom: two
rows of room controls and the return-channel row. Minimum size 800×700.

## Factory presets

Tight Room, Vocal Plate, Warm Hall, Cathedral, Ambient Bloom,
Shimmer Drone, Snare Plate, Snare Tight, Snare Ambient, Snare Gated,
Bright Plate, Drum Room, Vocal Chamber, String Hall, Mix Glue,
Short Ambience, Surf Spring, 80s Gate, Octave Halo, Fifth Bloom. The
originals were re-voiced onto the new algorithms (decision D1). Every
preset carries `meta` (category, instrument, character, genres) for
`presets.search`.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). `algorithm` is an ordinary
  choice param (decision D3), so `track_set_plugin_param` takes "Plate".
- **Decay check:** `meter.measure` / `meter.stems` with `detail: ["decay"]`
  → `mcp__resonance__meter_measure` reports EDT and T30 of the last free
  decay in a range. Read it on the track that *sends* to the reverb
  return. A bus target hears only tracks routed into the bus, so a
  send-fed return reads silent. That part lives in
  [meter-control-api.md](../mixer/meter-control-api.md) and [aux-sends.md](../mixer/aux-sends.md).
- **Agent skills:** `spatial` (SKILL.md and `references/depth.md`: the
  type table per return job, decay from tempo or `decay_sync`, pre-delay,
  return EQ, ducking), `mixing/references/roles.md` (per-role type and
  preset choices), `mastering/SKILL.md` (a `Mix Glue` insert before the
  chain, and a hall tail over an abrupt ending).

## How it's tested

| What | Where | Run |
|---|---|---|
| Classic bit-exact | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-reverb` |
| Per-algorithm goldens, acceptance numbers (decay, density, colour), stability | `tests/algorithms.rs` with modules `baseline`, `plate`, `room/{acceptance,decay_verification,golden,stability}`, `hall`, `spring`, `nonlinear`, `shimmer` → `golden/{plate,room,chamber,hall,ambience,spring,nonlinear,shimmer}_golden.f32` | same |
| Switching, tempo sync, editor under every algorithm | `tests/algorithms/{switching,sync,editor}.rs` | same |
| Return channel | `tests/return_channel.rs` → `golden/return_channel.f32` | same |
| Pre-W8 state renders unchanged | `tests/legacy_state.rs` → `golden/legacy_state.f32` | never re-bless for a "transparent" change |
| Freeze bounded, reset, glide, viz ring | `tests/freeze.rs`, `tests/reset.rs`, `tests/glide.rs`, `tests/viz.rs` | same |
| State round trip, skill keys | `tests/state.rs`, `tests/skill_keys.rs` | same |
| IR envelope printout (diagnostic, asserts nothing) | `tests/plugin.rs::debug_impulse_response_envelope` (`#[ignore]`) | `cargo test -p resonance-reverb --test plugin -- --ignored --nocapture` |
| Bench | `benches/reverb_dsp.rs`, `benches/dsp.rs` | `cargo bench -p resonance-reverb` |
| Shared primitives | `resonance-dsp/tests/reverb.rs` | `./scripts/run-tests.py -p resonance-dsp` |

`tests/fixtures/pre_revoice_presets/` holds the pre-D1 preset JSONs, which
`tests/legacy_state.rs` loads to prove an old project still renders as
before.

Not covered: no pixel snapshot of the editor. The spec's exit checks that
need a person or the running app were not done: a live agent session
mixing a real project with the type table (R7), and visual checks of the
per-algorithm editor in a Wayland session. See the intents below.

## Decisions and deviations

From the spec's §5.3 and §8, confirmed in code:

- Default algorithm Room, legacy states load as Classic (D2). The upgrade
  hook inserts the key on every load path.
- Factory presets moved onto the new types (D1).
- Room lines are 6.4–75 ms, not 5–60.
- Freeze is a 100 ms reversible ramp, not a hard gate. A gate clicked, and
  the click was then held forever in the lossless loop.
- The decay meter reads the sending track, not the return bus.

## Known gaps

- Live agent mixing session with the type table not done:
  [effects-live-agent-session](../../intents/plugins-effects/effects-live-agent-session.md).
- No visual check of the editors:
  [plugin-editor-visual-checks](../../intents/plugins-effects/plugin-editor-visual-checks.md).
