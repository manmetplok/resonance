# Delay (`com.resonance.delay`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.4 (the planned "Haas — Safe" preset was dropped; Haas lives in [stereo](stereo.md)). Audit gaps D1, D2 and P7 in `docs/history/reviews/plugin-audit.md` were fixed since.

A tempo-synced stereo delay for echoes, slapback, dub throws and ping-pong.
It has a clean Digital and a tape-like Analog character, tone filters and
drive that shape both the echo and the loop, a freeze that holds the buffer,
and two wet-only dynamics: a tempo-synced gate and a ducker that pulls the
echoes down while the dry signal plays.

## Behaviour

- **Time.** `sync` (on by default) picks `division` from 1/1 to 1/16T
  (straight, dotted and triplet; `sync::DIVISION_LABELS`) against the
  host tempo. Off, `time_ms` sets 1–2000 ms. A sync toggle, division
  change or tempo change glides the read tap (a smoothed delay in samples)
  instead of jumping.
- **Feedback** 0–95 %. The in-loop saturator is normalised to unity
  small-signal gain (`tanh(x·d)/d`), so the knob is the true loop gain and
  the loop does not self-oscillate at high drive.
- **Routing** (`routing`: Stereo, Ping-Pong, Dual). Each route has its own
  right delay line. `stereo_offset` moves the right tap on all three
  routes: on Stereo it offsets the two repeat trains, on Ping-Pong it makes
  the bounce uneven (a swung ping-pong), on Dual it offsets two lines fed
  the mono input.
- **Character** (`character`: Digital, Analog). Analog darkens the
  effective hi-cut as the delay time grows, multiplies the wow depth by 1.5
  and adds a little extra in-loop drive.
- **Tone.** `hi_cut`, `lo_cut` and `drive` act on the echo itself, not only
  on the recirculation, so a single echo at feedback 0 is shaped too.
  `mod_rate` / `mod_depth` add wow.
- **Freeze** holds the buffer: input muted, loop at unity, saturator
  skipped. The tone filters stay in the loop, so a closed hi-cut dulls the
  frozen loop.
- **Wet gate** (`gate_on`, `gate_rate` as a division, `gate_width`,
  `gate_shape`, `gate_depth`) and **duck** (`duck_amount`,
  `duck_threshold`, `duck_release`; fixed fast attack) act on the wet
  signal after the tap and before the mix. The loop never sees them, so the
  tail keeps its shape. The ducker is `resonance_dsp::dynamics::Ducker`,
  keyed off the plugin's own dry input, so it needs no sidechain routing.
- **Latency:** none.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin, tempo resolution | `plugins/resonance-delay/src/lib.rs` | `ResonanceDelay::process` |
| Params | `plugins/resonance-delay/src/params.rs` | 22 params; `CHARACTER_LABELS`, `ROUTING_LABELS` |
| Sync | `plugins/resonance-delay/src/sync.rs` | `DIVISION_LABELS`, `division_beats`, `delay_samples` |
| DSP | `plugins/resonance-delay/src/dsp.rs` | delay lines, routing, character, freeze |
| Wet gate + duck | `plugins/resonance-delay/src/gate.rs` | wet gate window, `Ducker` |
| Editor state | `plugins/resonance-delay/src/viz.rs` | in/out peaks, echo taps per channel |
| Editor | `plugins/resonance-delay/src/editor/{app,controls,echo_view}.rs` | header, wrapped control strip, echo-tap view |
| Presets | `plugins/resonance-delay/src/presets.rs`, `presets/*.json` | 6 factory presets |
| Shared DSP | `resonance-dsp` | `DelayLine`, `Biquad`, `OnePole`, `dynamics::Ducker`, `BlockPeaks` |

## Editor

A header with the preset bar, the division (or free time), host BPM and the
effective delay in ms, an echo view that
draws each channel's repeat train (time and level per tap, so ping-pong and
stereo offset show as two trains), and a control strip that wraps to fit
the window down to its minimum width. Choice params show labels ("1/8D",
"Ping-Pong").

## Factory presets

Quarter Note, Dotted Eighth, Slapback, Dub, Ping-Pong Eighth, Lo-Fi Tape.
Each is a full snapshot of all 22 params, so loading one resets the gate
and duck too.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces).
- **Agent skills:** `spatial/references/depth.md` §4 (delay throws on a
  return: start from `Dotted Eighth` or `Quarter Note`, `mix` 1.0, filters,
  low `feedback`, `duck_amount`). The `spatial` and `mixing` SKILL.md text
  mention delay returns without a keys block.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-delay` |
| Loop gain, freeze holds | `tests/feedback_loop.rs` | same |
| Tone filters shape the echo | `tests/wet_tone.rs` | same |
| Stereo offset on every route | `tests/stereo_offset.rs` | same |
| Sync glide | `tests/sync_glide.rs` | same |
| Wet gate and duck | `tests/gate_duck.rs` | same |
| Echo-tap levels | `tests/echo_taps.rs`, `tests/viz.rs` | same |
| Presets complete | `tests/presets.rs` | same |
| State round trip | `tests/state.rs` | same |
| Division labels | `tests/division_labels.rs` | same |
| Editor fits its window, controls bind and announce | `tests/editor_layout.rs`, `tests/editor_controls.rs` | same |
| Plugin smoke, skill keys | `tests/plugin.rs`, `tests/skill_keys.rs` | same |
| Bench | `benches/delay_dsp.rs` | `cargo bench -p resonance-delay` |

Not covered: no pixel snapshot of the editor.

## Decisions and deviations

- Gate and duck sit after the tap on purpose. Chopping inside the loop
  would re-record the chopped signal and the repeats would decay into a
  stutter.
- The ducker's attack is fixed: a ducking delay always wants the wet out of
  the way at once. Only the recovery (`duck_release`) is a choice.
- The warmth spec's "Haas — Safe" delay preset was dropped because the
  [stereo](stereo.md) plugin owns Haas widening.

## Known gaps

None known beyond the fleet-wide ones (no editor pixel snapshots).
