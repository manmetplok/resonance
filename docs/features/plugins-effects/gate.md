# Gate (`com.resonance.gate`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none of its own; gaps C3–C5 and P6 in `docs/history/reviews/plugin-audit.md` were fixed since.

A stereo noise gate and downward expander. With nothing on its key port it
gates its own signal. With a key connected it opens on another track
instead: a pad that opens only when the kick hits, or a trance gate keyed
from a rhythm part. The editor's main job is to say which detector is
running and what it is doing.

## Behaviour

- **Curve.** Detector → static downward curve → ballistics, in the log
  domain. Signal below `threshold` is attenuated at `ratio`, capped at
  `range` dB, so a closed gate can duck instead of muting.
- **Hysteresis** (`hysteresis`, dB): the close threshold sits that far
  below the open threshold, so a signal hovering at the threshold does not
  chatter.
- **Hold** (`hold`, ms): the gate stays open that long after the signal
  falls below the close threshold.
- **Ballistics.** `attack` is how fast the gate opens, `release` how fast
  it closes. The shared `Ballistics` run on the gain-reduction envelope,
  which rises as the gate closes, so `gate_ballistics` swaps them.
- **Detector.** A peak envelope with instant attack and a fixed 15 ms
  release (`DETECTOR_RELEASE_MS`), so the threshold never sees zero
  crossings. `key_hpf` high-passes the detector signal.
- **Key.** `SIDECHAIN_INPUT = Some(2)`. A connected key replaces the
  detector source and never reaches the output.
- **Latency:** none. A non-finite sample cannot wedge the envelopes.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin + key port | `plugins/resonance-gate/src/lib.rs` | `ResonanceGate`, `process_with_key` |
| Params | `plugins/resonance-gate/src/params.rs` | `threshold`, `ratio`, `attack`, `hold`, `release`, `range`, `hysteresis`, `key_hpf` |
| DSP | `plugins/resonance-gate/src/dsp.rs` | gate state machine, `gate_ballistics`, `DETECTOR_RELEASE_MS` |
| Editor state | `plugins/resonance-gate/src/viz.rs` | `GateViz` (`key_connected`, detector level, open/hold/closed) |
| Editor | `plugins/resonance-gate/src/editor/{mod,status,widgets}.rs` | `GROUPS` table, `DetectorSummary` |
| Presets | `plugins/resonance-gate/src/presets.rs`, `presets/*.json` | 9 factory presets |
| Shared DSP | `resonance-dsp` | `dynamics::Ballistics` |

## Editor

A header band with the preset bar and a detector readout ("INPUT" or
"EXTERNAL KEY", and "OPEN" / "HOLD" / "CLOSED"), then three knob cards from
the data-driven `editor::GROUPS` table: DETECTION (threshold, hysteresis,
key HPF), TIMING (attack, hold, release) and DEPTH (ratio, range). There is
no signal plot on purpose: the host's channel meters already show level.
The editor applies the shared lavender theme and its knobs follow each
param's declared skew.

## Factory presets

Init — Default, Vocal — Noise Gate, Drums — Snare Gate, Drums — Tom Gate,
Guitar — Noise Floor, Gentle Expander, Dialogue — Room Tone,
Keyed — Open on Kick, Keyed — Trance Gate.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). Keying it:
  `track.set_sidechain` → `mcp__resonance__track_set_sidechain` (and the
  bus/master forms).
- **Agent skills:** none. No skill has a `com.resonance.gate` keys block,
  although `tests/skill_keys.rs` is in place for one.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-gate` |
| Curve, open/close, key replaces detector | `tests/gate.rs` | same |
| Attack opens, release closes | `tests/ballistics.rs` | same |
| Detector status readout | `tests/viz.rs` | same |
| Presets: full snapshots, in range | `tests/presets.rs` | same |
| State round trip | `tests/state.rs` | same |
| Live param values for hosts | `tests/live_values.rs` | same |
| NaN hardening | `tests/nan_hardening.rs` | same |
| Editor: layout table, knob binding, edit announcements | `tests/editor_layout.rs`, `tests/editor_param_binding.rs`, `tests/editor_announce.rs` | same |
| Editor lifecycle on a real compositor | `tests/editor_open.rs` (`#[ignore]`) | `cargo test -p resonance-gate --test editor_open -- --ignored --nocapture` from a Wayland session |
| Same on macOS | `tests/editor_open_cocoa.rs` (`harness = false`, hand-rolled `--ignored`) | `cargo test -p resonance-gate --test editor_open_cocoa -- --ignored --nocapture` from a logged-in macOS session |

The two `editor_open*` tests guard the editor lifecycle that every plugin
shares (`resonance-plugin/src/editor_host.rs`). The gate is just the
vehicle. They run under a wall-clock watchdog because their failure mode is
a hang. They assert the sizes the plugin asked for, never the mapped size.

Not covered: no pixel snapshot of the editor. The by-hand tests are not
run by `./scripts/run-tests.py`.

## Decisions and deviations

- The plain editor is deliberate. What a user cannot see from outside is
  which detector is running, so that is what the header shows.

## Known gaps

- Picking the key source has no GUI in the app; only the control API sets
  it (see [sidechain-routing.md](../mixer/sidechain-routing.md)).
