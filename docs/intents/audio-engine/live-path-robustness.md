# Live-path robustness: lock misses, PDC transitions, tails, stopped mix, port rescans

> Status: **idea**
> Area: audio-engine
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-R1c, FU-R2a, FU-R3a, FU-R3b, FU-R4b, FU-R4c, FU-H1c;
> moved out of [`../bugs.md`](../bugs.md) (BUG-21, 30, 32, 33, 34, 35, 37) by the 2026-10-15 triage, which re-checked each against master `ef215b08`

## Why

Seven defects on the live audio path that are real, but each one is
realtime design work rather than a bug fix. They all show up only while
audio runs live: offline renders (bounce, freeze, stems) take the blocking
lock and build fresh compensation tables, so none of them is audible in an
export. As a result, the bit-exact bounce goldens do not cover them either.

## What it would do

Each item is a slice of its own, with a regression test in the named group.

1. **Count-in tail (was BUG-21).** The block in which the count-in ends
   renders only clicks. The backing track is silent for the frames after
   the downbeat, and `recording` turns on only at the end of that block, so
   the take starts up to one block late (`audio/mixer/callback/count_in.rs`
   ~:121-139). Split the block the same way `callback/seam.rs` does: render
   `[click_frames, frames)` through the playing path, latch the take at
   `punch_in`, and skip the matching input frames. Test home:
   `tests/engine/recording_alignment.rs`
   (`a_count_in_starts_the_take_on_its_last_frame` pins today's arithmetic).
2. **Lock-miss concealment (was BUG-30).** When `try_lock_counted` misses
   (`audio/clap_host/slot_locking.rs`), the plugin is skipped for that
   block. Dry audio then leaks through an effect, and an instrument drops
   out. The short-term fix is to crossfade an effect to dry, or hold and
   fade the previous block, using pre-allocated per-slot buffers. The real
   fix is for the engine thread to stop taking the instance lock while
   audio runs. Its first step is review leftover FU-R2c in
   [review-leftovers-engine](review-leftovers-engine.md). Call sites:
   `mixer/render/strategy.rs` ~:254, `mixer/master.rs` ~:90,
   `mixer/monitor.rs` ~:322.
3. **Fresh delay line starts from silence (was BUG-32).**
   `follow_comp_map` (`audio/latency.rs` ~:391-403) gives a track that had
   no delay a `TrackComp::fresh(delay)`, which reads zeros until it fills.
   As a result, adding the first latent plugin during playback silences
   every *other* track for `delay` samples. Delay-0 lines are also only
   about 2 samples long, so `adopt` has no history after a 0→large change.
   To fix it, either keep a history line with headroom even at delay 0, or
   crossfade from the undelayed signal into the new delay.
4. **Latent whole-chain bypass combs (was BUG-33).** During the 5 ms
   bypass fade, the dry copy is the undelayed chain input, and the comp
   table drops the chain's latency in the same window (`audio/bypass.rs`
   ~:55-58 says so). Delay the dry copy by the chain latency during the
   fade, and keep the comp-table crossfade consistent with it.
5. **CLAP tail (was BUG-34).** A stop holds busses and master for a fixed
   3 s (`limits.rs` `IDLE_HOLD_SECS`). Track insert effects get no stop tail
   at all, because they only run when `wants_idle_process()` is true.
   Query `clap_plugin_tail` at activation and on `tail.changed`, cache the
   value per slot in an atomic, and size the hold from the maximum across
   every chain, track inserts included. `u32::MAX` means "until silent",
   ended by a silence detector.
6. **Stopped mix: no PDC, no multi-out (was BUG-35).** While stopped,
   `idle_mix.rs` uses an empty `LatencyComp`, and `monitor.rs` processes
   only the main stereo port. Sub-tracks are skipped. To fix it, give the
   idle comp a free-running sample clock instead of the playhead, and reuse
   the track pass's multi-out fan-out. The stopped branch is hashed in
   `tests/mixer/mix_audio_parity.rs`, so expect to re-bless it.
7. **Port layout rescan (was BUG-37).** Ports are read once
   (`clap_host/bundle.rs` `declared_port_channels`), and the host does not
   offer `clap_host_audio_ports`. Engine side: serve `rescan`, re-read the
   ports while deactivated, and rebuild `PortBuffers` under the instance
   lock. App side: re-sync auto-created sub-tracks from the new port
   names.

## What exists to build on

- `callback/seam.rs`: head/tail sub-block rendering (for 1).
- `BypassFade` and the comp-table crossfade in `latency.rs` (for 2, 3 and 4).
- `tests/clap_host/plugin_lock_contention.rs`, `tests/mixer/latency_comp_republish.rs`,
  `tests/mixer/bypass_settle_race.rs`, `tests/mixer/stopped_mix_stage.rs`,
  `tests/clap_host/clap_port_layout.rs`, `tests/clap_host/plugin_rescan.rs`.
- [clap-hosting](../../features/audio-engine/clap-hosting.md) "Known gaps"
  already lists the missing `clap.tail` and host `audio-ports`.

## Open decisions

1. BUG-30: conceal now, or go straight to a lock-free engine thread?
   Recommended: conceal now, since it is small and contained, and track the
   lock-free work separately.
2. BUG-32: history headroom per track (costs memory) or a crossfade from
   the undelayed signal? Recommended: the crossfade, which allocates
   nothing new.

## Slices

One per numbered item. They are independent, and none changes the save
format.

## How we'd know it works

A regression test per item in the test group named above, run with
`RESONANCE_RENDER_THREADS=8`. Items 2-6 should also get a live listen
during a [live stress run](rt-live-stress-run.md).
