# Strip meters

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none for the strip meters.

Every track, sub-track, bus and the master strip shows a stereo peak
meter beside its fader. This doc covers only those GUI strip meters. The
louder numbers — LUFS, true peak, loudness range, correlation, spectrum —
come from the `resonance-metering` crate, documented in
[plugins-instruments/metering-library.md](../plugins-instruments/metering-library.md);
the `meter.*` control methods are in
[meter-control-api.md](meter-control-api.md).

## Behaviour

- **What is measured.** Per-block sample peaks, post-fader (after the
  gain ramp) for tracks, sub-tracks and busses; post master FX, volume and
  clip for the master. The audio thread folds them into atomics with
  `fetch_max`.
- **Polling.** Each GUI tick (16 ms when fast) the app decays every level
  by `PEAK_DECAY` (0.85) and, while anything is sounding, sends
  `PollPeaks`; the reply `PeakSnapshot` raises a level only when the new
  peak is higher (`update::tick::apply_peak_snapshot`). Levels below a
  settle epsilon snap to 0, and once the transport is stopped, nothing is
  armed, monitored or auditioning and every meter has settled, no poll is
  sent at all and the tick drops to the idle rate.
- **Drawing.** `StereoMeterCanvas` (two 6 px bars, 13 px total) maps
  −60…+6 dB linearly onto the fader height and colours the bar green below
  −6 dB, warm from −6 to 0 dB, red above 0 dB. The canvas cache is only
  cleared when a level changes. There is no peak-hold marker and no dB
  scale on the meter.
- **Collapsed parents** show one mini meter per sub-track.
- The strips show peak only; loudness (LUFS) in the GUI appears only in
  the reference panel ([reference-ab-monitoring.md](reference-ab-monitoring.md))
  and inside the mastering/analysis plugins.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Peak capture (engine) | `resonance-audio/src/mixer/common.rs`, `render/{bus_pass,sub_track,track_pass}.rs`, `mixer/master.rs` | `ramped_stereo_peaks`, `update_peak_l/_r`, `apply_master_volume_and_peaks` |
| Peak polling (engine) | `resonance-audio/src/engine/thread/dispatch/peaks.rs` | `handle_poll_peaks` |
| Tick, decay, gating | `resonance-app/src/update/tick.rs` | `PEAK_DECAY`, `update_vu_meters`, `peaks_have_consumers`, `apply_peak_snapshot`, `needs_fast_tick` |
| Meter widget | `resonance-app/src/view/controls.rs` | `StereoMeterCanvas`, `meter_v`, `level_to_bar_height`, `level_color`, `fader_section` |

## Surfaces

- **GUI:** the meter beside every fader on the Mix tab.
- **Control API / MCP:** the strip peak levels are not readable; agents measure
  through `meter.*` — see [meter-control-api.md](meter-control-api.md).
- **Agent skills:** `mixing`, `mastering`, `spatial` (through `meter.*`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Tick gating (no idle polling) | `resonance-app/tests/mixer/tick_gating.rs` | `./scripts/run-tests.py -p resonance-app` |
| Mix meter reset | `resonance-audio/tests/mixer/mix_meter_reset.rs` | `./scripts/run-tests.py -p resonance-audio` |

The crate's own conformance tests are listed in
[metering-library.md](../plugins-instruments/metering-library.md#how-its-tested).

Not covered: no test pins the strip meter's dB mapping or colour
thresholds, and no golden shows a lit meter.

## Decisions and deviations

- The GUI polls peaks rather than streaming them, split across two ticks,
  so the GUI thread never contends on engine locks.
- Meters are live canvases built outside the strip's `lazy` body;
  everything else on a strip is cached (ux-guidelines.md, "View
  Performance").

## Known gaps

- `docs/guides/ux-guidelines.md` → Metering promises a `METER_BG` token and
  a peak-hold indicator; neither exists (the canvas fills `BG_1` directly
  and has no hold).
