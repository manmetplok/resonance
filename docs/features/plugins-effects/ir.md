# IR (`com.resonance.ir`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none of its own. Audit finding I1 in `docs/history/reviews/plugin-audit.md` (latency not controllable or visible) was fixed by ba todo #1300.

An impulse-response convolution plugin for guitar and bass cabinets and
sampled rooms. The user loads a `.wav` IR and can step through the other
IRs in the same folder. They set dry/wet and output gain, and choose how
much latency to spend on convolution. The editor shows the IR's waveform
and frequency response, in/out meters, and the latency in force.

## Behaviour

- **Loading.** "Load IR…" opens a file picker. ◀ / ▶ step through the
  `.wav` files of that folder. `file_select` is an index into the
  machine's directory listing (0–999) and is excluded from presets.
  Presets and saved state carry the IR's path (`ir_path`) instead. A
  background loader thread decodes the file, resamples it to the session
  rate, builds a stereo convolver, precomputes the waveform envelope and
  a log-frequency magnitude response, and publishes the convolver through
  a `Mailbox`. The audio thread swaps convolvers with a 64-sample crossfade
  (`SWAP_FADE_SAMPLES`). A swap that cannot be admitted is retried on a
  later block, never dropped.
- **Mono or stereo IRs.** A mono file feeds both channels.
- **Resample gain.** A resampled IR is rescaled by `source_rate /
  target_rate`, so a 96 kHz IR loaded at 48 kHz keeps its level.
- **Mix:** `dry_wet`, `output_gain`, both smoothed. The dry path is
  delayed to match the convolver.
- **Latency mode** (`latency_mode`: Tracking, Normal (default), Efficient).
  The convolution block size is the reported latency: about 2.7–2.9 ms in
  Normal (128 samples at ≤ 50 kHz, 256 to 88 kHz, 512 above), a quarter of
  that in Tracking (roughly 4× the CPU), four times that in Efficient,
  clamped to 32–2048 samples. The sound is identical in every mode. A
  change while active is reported through `HostHandle::set_latency_samples`;
  the host cycles the plugin and `initialize()` applies the new size. Until
  then the editor shows the pending figure next to the active one.
- **Missing or failed IR.** The loader's error lands in `ir_name` as
  `"Error: …"`, and the editor draws it as a banner with a `Locate…`
  button instead of a file name.
- **State.** `IrExtraState` saves the path and, on load, rescans the
  directory and kicks the loader, because the CLAP bridge restores state
  after the plugin has moved into the audio processor.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin, latency-change report | `plugins/resonance-ir/src/lib.rs` | `ResonanceIr`, `target_block_size`, `latency_samples` |
| Params | `plugins/resonance-ir/src/params.rs` | `IrParams`: `file_select`, `dry_wet`, `output_gain`, `latency_mode`; `ir_path`, `file_list`; `MAX_FILE_INDEX` |
| DSP | `plugins/resonance-ir/src/dsp.rs` | `StereoConvolver`, `IrEngine`, `LatencyMode`, `block_size_for`, `SWAP_FADE_SAMPLES` |
| WAV decode + resample gain | `plugins/resonance-ir/src/ir_loader.rs` | `load_ir`, `load_ir_from_bytes` |
| Loader thread, viz precompute | `plugins/resonance-ir/src/loader.rs` | `LoaderHandle`, `LoaderDeps` |
| Saved state | `plugins/resonance-ir/src/state.rs` | `IrExtraState` |
| Latency readout | `plugins/resonance-ir/src/latency.rs` | `LatencyReadout`, `readout` |
| Editor | `plugins/resonance-ir/src/editor/{app,header,controls,latency,meters,missing_banner,response_view,waveform_view}.rs` | |
| Shared DSP | `resonance-dsp` | `FftConvolver`, `DelayLine`, `SwapFader`, `BlockPeaks`; `resonance_common::decode_wav_channels`, `scan_directory`; `resonance_plugin::{Mailbox, rescan_directory}` |

## Editor

A header with "Load IR…", ◀ / ▶ and the file name and info. A centre area
with the waveform envelope (left channel above the centre line, right below)
and the log-frequency magnitude response (0 dB reference, 12 dB grid).
Under the mode picker, the latency in force ("2.90 ms · 128 samples") and
any pending change. Dry/wet and output knobs, in/out meters, and the
shared preset bar, which holds user presets only.

## Factory presets

None. The plugin declares no `FACTORY_PRESETS`. User presets work through
the shared `PresetSession` and carry `ir_path`.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). Over the control API an agent can
  set `dry_wet`, `output_gain`, `latency_mode` and `file_select` (an index
  into a folder it cannot list), but cannot pick an IR file by path.
- **Agent skills:** none reference it, and the crate has no
  `tests/skill_keys.rs`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-ir` |
| Block engine vs the old per-sample loop, swap crossfade, bypass alignment | `tests/dsp_block.rs` | same |
| Deferred swaps are retried, not dropped | `tests/swap_retry.rs` | same |
| Resample keeps level | `tests/resample_gain.rs` | same |
| Latency mode arithmetic and readout | `tests/latency_mode.rs` | same |
| Latency change across the real CLAP ABI (reported = delivered, then re-reported) | `tests/latency_mode_host.rs` | same |
| State round trip incl. path rescan | `tests/state.rs` | same |
| Missing-file banner, knob binding, viz | `tests/missing_banner.rs`, `tests/editor_bindings.rs`, `tests/viz.rs` | same |
| Bench | `benches/ir_dsp.rs` | `cargo bench -p resonance-ir` |

Not covered: no pixel snapshot of the editor; no test drives the file
picker or ◀ / ▶ browsing.

## Decisions and deviations

- The long block is called "Efficient", not "HQ": it is cheaper, not
  better.
- An out-of-range `latency_mode` falls back to Normal, so a bad automation
  value cannot park the plugin at 46 ms.

## Known gaps

- An agent cannot load an IR by path, and no skill uses the plugin.
- The response plot analyses only the left channel, truncated to 4096
  samples (audit LOW finding; still true in `loader.rs`).
