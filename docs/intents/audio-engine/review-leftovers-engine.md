# Engine and CLAP host: review leftovers

> Stage: **intent**
> Status: **idea** (checklist)
> Area: audio-engine
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-S2a, FU-S2b, FU-R2c, FU-R2d, FU-R3d, FU-H1b, FU-H1e, FU-W4d, FU-A1a6;
> [refactor intent](../../history/refactor/refactor-intent.md) C-3 leftovers, FU-B4b;
> [code review 2026-09-26](../../history/reviews/code-review-todo.md) LIB-01 naming;
> [field report 2026-10-06](../../history/reviews/field-report-2026-10-06.md) §1 follow-up

## Why

Code-quality, robustness and test items in `resonance-audio` and the
plugin bridge that the review campaigns left open. User-visible engine
defects are in [`../bugs.md`](../bugs.md) and
[live-path-robustness](live-path-robustness.md), not here.

## What it would do

- [ ] **FU-S2a** — the disconnect latch is a process-wide static and leaks
      between test apps (`engine/audio_engine.rs:63,287`); make it per engine.
- [ ] **FU-S2b** — `SaveClipsToProjectDir`'s doc names only
      `ClipsSavedToProjectDir`, not `ClipsSaveFailed` (`types/commands.rs:863-869`).
- [ ] **FU-R2c** — `handle_set_plugin_param` runs flush and `value_to_text`
      under the instance lock (`engine/plugins.rs:495-545`); move them out
      (feeds BUG-30's lock misses).
- [ ] **FU-R3d** — `set_param`'s dropped result is ignored (`engine/plugins.rs:504`).
- [ ] **FU-R2d** — `PluginScanFailure` has no warning kind for duplicate
      plugin ids (`types/tempo/mod.rs:180-185`).
- [ ] **FU-H1b** — the host offers no `clap_host_note_ports`.
- [ ] **FU-H1e** — `timed_params` (capacity 1024) can grow on the audio
      thread (`resonance-plugin/src/clap_bridge/process.rs:100,149`).
- [ ] **FU-W4d** — unannounced third-party param changes with the editor
      closed are not caught by freeze auto-stale.
- [ ] **FU-A1a6** — the app still links `clack-plugin` through
      resonance-plugin's preset/browser library (`resonance-app/Cargo.toml:17`,
      `resonance-plugin/Cargo.toml:23-24`); split the library out.
- [ ] **C-3 leftovers** — `AudioEvent::ImportFailed.reason: String`
      (`types/events.rs:209`) and `recording.rs::take_clip_source ->
      Result<_, String>` (`recording.rs:1209`); the last stringly errors.
- [ ] **FU-B4b** — the drop-order test still models the old
      `Arc<RwLock<IndexMap>>` teardown
      (`tests/clap_host/clap_plugin_drop_order.rs`); test
      `release_all_plugins` through a real engine thread instead.
- [ ] **LIB-01 naming** — `linear_resample*` and
      `StreamingLinearResampler` (`resonance-common/src/wav.rs:870-883`,
      `resonance-audio/src/decode.rs:11`) now run the sinc resampler;
      rename them.
- [ ] **mlockall opt-in** — a host-wide `mlockall` option for live and
      recording sessions, if page-fault overruns persist after the
      per-plugin prefault fix.

## What exists to build on

[`../../features/audio-engine/clap-hosting.md`](../../features/audio-engine/clap-hosting.md),
[`../../features/audio-engine/rt-safety.md`](../../features/audio-engine/rt-safety.md),
[`../../features/audio-engine/engine-errors.md`](../../features/audio-engine/engine-errors.md).

## Open decisions

1. mlockall: build it only on evidence. **Recommended:** wait for a
   `RESONANCE_AUDIO_STATS` report that shows page-fault overruns.

## Slices

One commit per checkbox; FU-R2c together with BUG-30.

## How we'd know it works

Per item: a test that would have failed (e.g. a two-app test for the
latch; an allocation-counting test for `timed_params`).
