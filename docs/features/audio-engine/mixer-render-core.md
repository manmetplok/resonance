# Mixer render core

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: `docs/history/specs/realtime-multithreading.md` §4, §11;
> `docs/history/reviews/field-report-2026-10-06.md` §3.

How one block of audio is produced: the realtime callback that PipeWire (or
cpal) calls, the branches it chooses between, and the shared render core
that both the live callback and every offline renderer run. Mixer
*features* (sends, groups, sidechain routing, solo rules) are described in
[../mixer](../mixer/); this page covers how the engine renders them.

## Behaviour

### The callback (`mix_audio`)

Per call, in order:

1. Flush denormals, take the CLAP audio-thread role, mark the callback in
   flight (`CallbackActivity::enter`).
2. Read the offline-render gate. Pick up live hardware MIDI (or only forward
   it when gated).
3. Zero the output; clamp frames to `buf_frames`; read the playhead once
   (every branch renders from and commits against that one value).
4. If the reference A/B monitor is active it replaces the whole output and
   the callback ends.
5. Snapshot the tempo map; read the monitor ring (always, even when gated).
6. Run exactly one branch:
   - gated → silence, transport held;
   - count-in → `count_in::render_count_in_block` (playhead pinned, click
     over monitored input);
   - stopped → `stopped::render_stopped_block` (armed / monitored tracks
     pass input through their chains, busses and master; a tail hold keeps
     effects ringing after a stop, `IDLE_HOLD_SECS` = 3);
   - playing → `play::render_playing_block`.
7. Sum the audition preview on top (bypasses master).

### The playing branch

`seam::detect` checks whether the buffer reaches `loop_out`. If it does, the
block renders as two sub-blocks: up to the seam, an all-notes-off to
instruments, then from `loop_in`. Each sub-block is
`track_block::render_timeline_block` → `render_core::render_block` with
`RenderStrategy::Live`. `master_pass` then runs master FX, the metronome,
master volume with hard clip and peaks, and the mix meter.

### `render_block` (shared with offline renders)

1. Zero the active bus buffers.
2. **Track pass** (`render_track_pass`), split at the summing point:
   - *Prologue (serial):* prepare each slot; decide which silenced tracks
     must still render because their signal keys a sidechain
     (`key_consumed`); build the job order (`build_schedule`, longest cost
     EMA first).
   - *Jobs (parallel on the render pool):* one per top-level track. Source
     (frozen cache, or the instrument fed by this block's MIDI, or audio
     clips + take comp + monitor input), insert chain with bypass
     crossfades, sidechain key capture, track-stage PDC, meters, then the
     multi-output fan-out into its sub-tracks' slots. A job writes only its
     own slots.
   - *Reduction (serial, track-map order):* replay each track's post-fader
     route (bus or master) and its aux sends, then each of its sub-tracks'
     route and sends (`replay_route`). This is the same order the old single
     loop summed in, so output is bit-identical at any thread count.
3. **Dry-path delay:** `LatencyComp::apply_dry` delays everything that went
   straight to master by the bus-stage latency.
4. **Bus pass** (`render_bus_pass`): busses grouped into levels by their
   send graph (`assign_levels`: a bus's level is the longest chain of
   enabled bus → bus sends ending at it, whatever the creation order); a
   bus pulls aux sends from every lower-*level* bus, in source-index
   order, when its job starts, runs its chain, captures keys, applies bus-stage PDC and meters;
   then a serial reduction sums busses into master in index order.

The caller then runs master FX and volume (live) or the offline master pass
([offline-render-and-export.md](offline-render-and-export.md)).

### Live vs offline (`RenderStrategy`)

| | `Live` | `Bounce` |
|---|---|---|
| Plugin locks | `try_lock`, skip on miss, MIDI to `MidiStash` | `try_lock_with_backoff` (spin, then sleep) |
| Gains | ramp from last gains, mute fades | constant per chunk (automation endpoints) |
| Meters / last-gain atomics | written | never written |
| Monitor input | mixed | none |
| Filters | — | `in_filter`, `fan_out_only`, `key_only`, `key_only_bus`, `respect_mute_solo`, `freeze_raw`, `send_filter` |

### Rules worth knowing

- **Sidechain keys are one block old.** `SidechainTaps` double-banks every
  tap: a key reads the previous block's capture. Track order therefore never
  matters, and a track may key off itself.
- **Sub-track aux sends** are applied in the reduction, after the parent's
  group trim (fixed in `8cc428b5`, field report 2026-10-06 §3). Before that,
  a kit tap's sends were silently never rendered.
- **Sends into an older bus are audible.** Levels come from the send
  graph, not bus index, so a send from a newer bus into an older one runs
  the older bus after it (BUG-04; before, such a send was silently
  dropped). Sources are still summed in index order, so a send graph that
  only ever pointed forward renders bit-identically to before.
- **Silenced tracks** still render key-only when a consumer needs their key
  (the muted ghost kick that ducks the bass).
- **Live MIDI and contention.** A note for an instrument whose lock is busy
  is parked in `MidiStash` and delivered next block; the stash is lent to
  jobs as each slot's `carry` and returned in the reduction.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Callback + branches | `resonance-audio/src/mixer/callback/{mod,play,stopped,count_in,idle_mix,master_pass,seam,monitor_input,reference,context}.rs` | `mix_audio`, `CallbackInputs`, `CallbackScratch`, `Seam` |
| Phase order | `resonance-audio/src/mixer/render_core.rs` | `render_block` |
| Track pass | `resonance-audio/src/mixer/render/track_pass.rs` | `render_track_pass`, `run_track_job`, `reduce_track_pass`, `replay_route`, `render_one_track` |
| Bus pass | `resonance-audio/src/mixer/render/bus_pass.rs` | `render_bus_pass`, levels |
| Routing + sends | `resonance-audio/src/mixer/render/routing.rs` | `route_post_fader`, `apply_track_aux_sends`, `pull_bus_aux_sends` |
| Strategy | `resonance-audio/src/mixer/render/strategy.rs` | `RenderStrategy`, `SendFilter`, `TrackDisposition` |
| Slots | `resonance-audio/src/mixer/render/slots.rs` | `TrackSlot`, `TrackSlots`, `LiveSlots`, `SlotSupply`, `PassStats` |
| Sub-tracks, ports, frozen source, clips | `mixer/render/{sub_track,ports,frozen,clips}.rs` | `fan_out_to_sub_tracks`, `process_multi_port`, `fill_from_frozen_source`, `mix_track_clips` |
| Master, click, take comp, stash | `mixer/{master,click,take_comp,midi_stash,midi_events}.rs` | `apply_master_fx_chain`, `MidiStash`, `build_comp_table` |
| Sidechain taps | `resonance-audio/src/types/sidechain.rs` | `SidechainTaps` |

## Surfaces

- **GUI:** everything audible.
- **Control API / MCP:** indirectly, through every mixer tool.
- **Agent skills:** none directly.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-identity goldens for `render_block` and `mix_audio` | `resonance-audio/tests/mixer/render_block_parity.rs`, `mix_audio_parity.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Parallel = serial | `tests/mixer/parallel_render.rs`; whole suite with `RESONANCE_RENDER_THREADS=8` | same |
| Loop seams | `tests/mixer/loop_seam_edges.rs`, `bus_first_block_loop_seam.rs`, `silent_advance_loop_seam.rs`, `loop_range_snapshot.rs` | same |
| Sends, sub-tracks, sidechain | `tests/mixer/aux_send_render.rs`, `sub_track_aux_sends.rs`, `sub_track_parent_fader.rs`, `sidechain_key_delivery.rs`, `tests/sidechain_taps.rs` | same |
| Stopped / monitor / count-in | `tests/mixer/stopped_mix_stage.rs`, `stopped_request_process.rs`, `stopped_instrument_preview.rs`, `recorded_monitor_gating.rs` | same |
| Gains, solo, MIDI | `tests/mixer/mixer_gain_ramp.rs`, `solo_predicate.rs`, `solo_race.rs`, `held_note_release.rs`, `midi_stash.rs`, `midi_event_window.rs` | same |
| Harnesses | `mixer/test_support/{callback,render_block}.rs` (`MixAudioHarness`, `RenderBenchHarness`) | — |
| Benchmarks | `resonance-audio/benches/render_path.rs` | `cargo bench -p resonance-audio --bench render_path` |

Not covered: real-device timing; the multi-output harness drives fake
plugins, not the real drums plugin.

## Decisions and deviations

- Spec §4.5 proposed busses pushing sends level by level; the build has each
  bus *pull* its sends in source-index order, because pushing would reorder
  float additions and break bit-identity.
- Key-only decisions moved to a serial prologue; this changes, at most,
  whether a muted key source renders once more (unheard) in the block where
  its consumer finishes fading out.

## Known gaps

- Clip warp settings are stored but not rendered: see
  [clip-warp-render](../../intents/audio-engine/clip-warp-render.md).
