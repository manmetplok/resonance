//! Shared chunked render core for both bounce entry points.
//!
//! `to_wav` and `to_audio_clip` both drive `render_chunk` in a loop;
//! the only differences are where the output goes and whether the
//! master FX chain runs. The chunk scratch buffers live here so both
//! call sites can size and allocate them identically.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, MutexGuard};

use crate::clap_host::SyncClapInstance;
use crate::latency::LatencyComp;
use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
use crate::mixer;
use crate::types::*;

use super::super::vocal_render::TuningOverlay;
use super::super::{SharedState, MAX_BUSSES};

pub const BOUNCE_CHUNK: usize = 1024;

/// CLAP activation minimum block size: plugins are activated with
/// `activate(…, min_frames = 32, max_frames = 8192)`
/// (`clap_host::bundle`), so no offline `process()` call may run fewer
/// frames — see [`chunk_span`].
pub const MIN_CLAP_FRAMES: usize = 32;

/// How many `try_lock` spins before falling back to sleeping. A spin is
/// a `std::hint::spin_loop` + immediate retry — cheap and only useful
/// for the rare case where the audio thread is on the verge of
/// releasing the lock. Anything beyond that wastes CPU.
const PLUGIN_LOCK_SPIN_ITERS: u32 = 8;
/// Initial sleep duration after spin-wait fails. Audio callbacks at
/// typical buffer sizes (256–1024 frames @ 48 kHz = ~5–21 ms) hold any
/// given plugin's mutex only for the slice of process() spent on that
/// plugin, so 100 µs is enough to clear most contention windows
/// without yielding the bounce thread for an entire callback.
const PLUGIN_LOCK_INITIAL_SLEEP: Duration = Duration::from_micros(100);
/// Cap on the exponential back-off — at 2 ms we're already comfortably
/// past a single audio quantum at 48 kHz / 96 frames, so doubling
/// further just delays the bounce without helping the audio thread.
const PLUGIN_LOCK_MAX_SLEEP: Duration = Duration::from_micros(2000);

/// Take a plugin's mutex without blocking the audio thread. The audio
/// callback uses `try_lock` everywhere (see `engine/plugins.rs`,
/// `mixer/track_block.rs`, etc.) and silently drops out for the
/// current block if the lock is held — so a blocking `lock()` from the
/// bounce thread would force the audio thread's `try_lock` to fail,
/// glitching live playback for the duration of the bounce thread's
/// process() call.
///
/// Instead we spin briefly, then back off with progressively longer
/// sleeps. Lock holders on either side run a single plugin's process()
/// (sub-millisecond for cheap plugins, a few ms for heavy ones), so
/// the back-off catches the audio thread on its release without
/// burning CPU.
#[inline]
pub(super) fn lock_plugin_for_bounce(
    mutex: &Mutex<SyncClapInstance>,
) -> MutexGuard<'_, SyncClapInstance> {
    try_lock_with_backoff(mutex)
}

/// Generic backbone for [`lock_plugin_for_bounce`]. Lives separately so
/// integration tests can hammer it against a plain `Mutex<u32>` without
/// having to materialise a real CLAP plugin. Exposed via the
/// `test_support` module in `lib.rs`.
#[inline]
pub fn try_lock_with_backoff<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Fast path: no contention.
    if let Some(g) = mutex.try_lock() {
        return g;
    }
    // Brief spin — covers the case where the audio thread is one or
    // two instructions away from releasing.
    for _ in 0..PLUGIN_LOCK_SPIN_ITERS {
        std::hint::spin_loop();
        if let Some(g) = mutex.try_lock() {
            return g;
        }
    }
    // Back off with sleeps capped by `PLUGIN_LOCK_MAX_SLEEP`. We don't
    // poll any cancel flag inside the loop because contention windows
    // are sub-millisecond and the per-chunk cancel check in the bounce
    // loops above is plenty responsive.
    let mut sleep = PLUGIN_LOCK_INITIAL_SLEEP;
    loop {
        std::thread::sleep(sleep);
        if let Some(g) = mutex.try_lock() {
            return g;
        }
        sleep = (sleep * 2).min(PLUGIN_LOCK_MAX_SLEEP);
    }
}

/// Mutable scratch buffers reused across chunks. Allocated once by the
/// caller and lent to [`render_chunk`].
pub(super) struct ChunkScratch {
    pub sidechain: crate::types::SidechainTaps,
    /// Per-track render slots (`mixer::render::slots`), grown before each
    /// chunk to the graph's track count — this is not the audio thread.
    pub track_slots: crate::mixer::render::slots::TrackSlots,
    pub track_buf_l: Vec<f32>,
    pub track_buf_r: Vec<f32>,
    pub bus_bufs: Vec<(Vec<f32>, Vec<f32>)>,
    /// Per-output-port scratch for multi-output instruments (e.g.
    /// `resonance-drums` with 7 ports). Populated by `process_multi`,
    /// then drained: port 0 feeds the parent track's effect chain,
    /// ports 1..N feed their matching sub-tracks' chains.
    pub port_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    pub note_buf: Vec<PendingNoteEvent>,
    pub mix_buf: Vec<f32>,
    /// Dry staging for the bypass crossfades. An offline render only ever
    /// sees *settled* bypass states (see `crate::bypass`), so this never
    /// actually stages a fade — it is here because the shared render core
    /// requires it, and because it keeps the offline path structurally
    /// identical to the live one.
    pub fx_dry: crate::bypass::FxDryScratch,
    /// The worker pool the chunk's track and bus jobs spread over
    /// (realtime-multithreading.md §4.7): normal priority, owned by this
    /// render, joined when it ends.
    pub pool: crate::render_pool::RenderPool,
}

impl ChunkScratch {
    pub(super) fn new() -> Self {
        Self {
            sidechain: crate::types::SidechainTaps::new(BOUNCE_CHUNK),
            track_slots: crate::mixer::render::slots::TrackSlots::new(0, BOUNCE_CHUNK),
            track_buf_l: vec![0.0f32; BOUNCE_CHUNK],
            track_buf_r: vec![0.0f32; BOUNCE_CHUNK],
            bus_bufs: (0..MAX_BUSSES)
                .map(|_| (vec![0.0f32; BOUNCE_CHUNK], vec![0.0f32; BOUNCE_CHUNK]))
                .collect(),
            port_scratch: (0..MAX_PLUGIN_OUTPUT_PORTS)
                .map(|_| (vec![0.0f32; BOUNCE_CHUNK], vec![0.0f32; BOUNCE_CHUNK]))
                .collect(),
            note_buf: Vec::with_capacity(256),
            mix_buf: vec![0.0f32; BOUNCE_CHUNK * 2],
            fx_dry: crate::bypass::FxDryScratch::new(BOUNCE_CHUNK),
            pool: crate::render_pool::RenderPool::new(crate::render_pool::PoolConfig::offline(
                BOUNCE_CHUNK,
            )),
        }
    }
}

/// Read-only context shared by every chunk in a bounce run.
///
/// No map lock (code review ARCH-02): [`render_chunk`] loads the
/// published render graph once per chunk — audio and MIDI clips, busses,
/// master chain, tracks, plugin instances — so the offline render sees
/// edits at chunk granularity, as it did through the per-chunk read
/// guards, and an engine-thread edit never queues behind a chunk (the
/// last guard, the audio clips', went in B-5). A plugin *instance* is
/// still locked per `process()` through `lock_plugin_for_bounce`; the
/// graph a chunk pinned keeps a removed instance or clip alive until the
/// chunk ends, and the engine's retire sweep — not this worker — frees it.
pub(super) struct ChunkCtx<'a> {
    pub shared: &'a Arc<SharedState>,
    /// The vocal-tuning retune caches this render built
    /// ([`ensure_tuning_caches`](crate::engine::vocal_render::ensure_tuning_caches)),
    /// patched over each chunk's clips until the engine thread has
    /// attached them to the live ones. Empty for a render that builds
    /// none.
    pub tuning: &'a TuningOverlay,
    pub tempo_map: &'a TempoMap,
    pub sample_rate: u32,
    pub master_vol: f32,
    /// Parameter-automation snapshot for this bounce run, captured once
    /// when the bounce was spawned. Drives gain / pan / mute / plugin-
    /// param automation identically to live playback.
    pub automation: &'a crate::engine::AutomationSnapshot,
    /// Plugin-delay-compensation table for this bounce run, built once
    /// by [`build_latency_comp`] so the offline render aligns tracks
    /// exactly like live playback. The bounce drivers additionally trim
    /// the leading `max_latency()` frames from the output so the
    /// rendered audio lands on the timeline with zero net shift.
    pub latency_comp: &'a LatencyComp,
    /// A depth measurement's send treatment (warmth-width-depth.md §7.6);
    /// `None` for every other render. See
    /// `mixer::RenderStrategy::Bounce::send_filter`.
    pub send_filter: Option<(&'a (dyn Fn(TrackId) -> bool + Sync), mixer::SendFilter)>,
    /// Hard-clip the master output to `[-1, 1]` after master volume
    /// (only read when a chunk runs with `include_master_fx`). The
    /// normalized export turns it off (code review ENG-06): its gain
    /// trim + true-peak limiter must see the float overs, not a
    /// flat-topped signal, and the encoder clamps at quantisation.
    pub hard_clip: bool,
}

/// Build a fresh compensation table from the current topology, reading
/// each plugin's activation-time latency and folding the published
/// external-instrument round-trip offsets — exactly like the live
/// refresh, so offline renders place external takes at the same
/// relative position as live playback (doc #260 finding #4). Runs on
/// the bounce thread — allocation is fine here.
pub(super) fn build_latency_comp(shared: &Arc<SharedState>) -> LatencyComp {
    let graph = shared.graph.load();
    let plugins_guard = &graph.plugins;
    let latency_of = |id: PluginInstanceId| {
        plugins_guard
            .get(&id)
            .map(|slot| {
                // Locking the instance is only worth it for a slot that
                // actually runs (ba doc #275 finding X3).
                let host_bypassed = slot.host_bypassed();
                let reported = (!host_bypassed)
                    .then(|| lock_plugin_for_bounce(slot).0.latency_samples() as u64)
                    .unwrap_or(0);
                crate::latency::slot_latency(reported, host_bypassed)
            })
            .unwrap_or(0)
    };
    let mut chains = crate::latency::chain_latencies(&graph.tracks, latency_of);
    let offsets = shared.external_offsets.load();
    crate::latency::add_external_offsets(&mut chains, |id| {
        offsets.get(&id).copied().unwrap_or(0)
    });
    let bus_chains = crate::latency::bus_chain_latencies(&graph.busses, latency_of);
    let (track_max, track_delays) = crate::latency::compensation_delays(&chains);
    let (bus_max, bus_delays) = crate::latency::compensation_delays(&bus_chains);
    LatencyComp::new(track_max, &track_delays, bus_max, &bus_delays)
}

/// Effective master-chain latency for renders that include the master
/// FX pass (`include_master_fx = true`): those paths must pre-roll and
/// trim by this on top of [`build_latency_comp`]'s `max_latency()` so
/// the export starts at t=0 with its full tail (doc #260 finding #8).
/// 0 while the master FX are bypassed — the chunk render skips the
/// chain then.
pub(super) fn master_fx_latency(shared: &Arc<SharedState>) -> u64 {
    let graph = shared.graph.load();
    let plugins_guard = &graph.plugins;
    crate::latency::master_chain_latency(
        &graph.master.plugin_ids,
        shared.master_fx_bypass.bypassed(),
        |id| {
            plugins_guard
                .get(&id)
                .map(|slot| {
                    let host_bypassed = slot.host_bypassed();
                    let reported = (!host_bypassed)
                        .then(|| lock_plugin_for_bounce(slot).0.latency_samples() as u64)
                        .unwrap_or(0);
                    crate::latency::slot_latency(reported, host_bypassed)
                })
                .unwrap_or(0)
        },
    )
}

/// Split the remaining render range into this iteration's chunk:
/// `.0` is the frame count to *process* — padded up to
/// [`MIN_CLAP_FRAMES`] so a short tail chunk still honours the CLAP
/// activation contract (`activate(…, min_frames=32, …)`) — and `.1` is
/// the frame count actually *consumed* from the front of the rendered
/// chunk (the padding frames are rendered and discarded). Always
/// `emit <= render <= BOUNCE_CHUNK`.
pub fn chunk_span(remaining: u64) -> (usize, usize) {
    let emit = (remaining.min(BOUNCE_CHUNK as u64)) as usize;
    (emit.max(MIN_CLAP_FRAMES), emit)
}

/// Reset every plugin so the bounce starts from a clean state. Without
/// this, leftover envelope phase / reverb tail / etc. from previous
/// playback would bleed into the first frame.
///
/// The stop/start cycle alone carries no reset semantics in CLAP (and is
/// a no-op for the first-party clack plugins), so it is followed by the
/// real `clap_plugin.reset()` (code review ENG-04), which is what clears
/// tails and kills voices — first-party plugins map it to their
/// `Plugin::reset`.
///
/// A plugin the reset leaves deactivated (it refused to restart) is
/// silent from now on; it is queued on
/// [`SharedState::plugins_dead_after_reset`] for the engine loop to
/// report (FU-M8b).
pub(super) fn reset_plugins(shared: &SharedState) {
    let plugins = shared.plugins();
    for (&id, mutex) in plugins.iter() {
        let mut inst = lock_plugin_for_bounce(mutex);
        let was_active = inst.0.is_active();
        if !inst.0.reset_processing() && was_active {
            shared.plugins_dead_after_reset.lock().push(id);
        }
        inst.0.reset();
    }
}

/// Render one chunk into `scratch.mix_buf`. The output is interleaved
/// stereo of length `frames * 2`. When `include_master_fx` is true,
/// master FX, master volume and (per [`ChunkCtx::hard_clip`]) hard-clip
/// are applied; otherwise the raw bus-summed mix is left for the caller (used by bounce-in-place
/// so master FX aren't applied twice on playback).
///
/// The closure `in_filter` decides which tracks contribute. Any track
/// for which the closure returns false is skipped exactly like a muted
/// one — but its bus isn't drained either, so reverb tails on shared
/// buses still come from the in-filter tracks only.
///
/// `fan_out_only` narrows that for the multi-output case (ba todo
/// #1242): a track it returns true for is in the filter *only* to drive
/// its sub-tracks' port fan-out, so its instrument runs but its own main
/// output (port 0) is discarded before the track's FX chain, fader, aux
/// sends and routing. Callers with no sub-track stems pass `&|_| false`.
///
/// `freeze_raw` selects the freeze-cache capture mode (see
/// [`mixer::RenderStrategy::Bounce`]): in-filter tracks render their raw
/// post-FX signal at unity gain straight to master so the cache is fader-
/// and route-independent. Every non-freeze caller passes `false`.
///
/// Reference A/B exclusion: this shared bounce core renders the mix
/// only. It takes no [`crate::engine::reference::ReferenceMonitor`] and
/// never reads `ctx.shared.reference`, so the live A/B selection cannot
/// leak into any offline export — the reference monitor tap lives solely
/// in the live callback (`mixer::mix_audio`).
#[allow(clippy::too_many_arguments)]
pub(super) fn render_chunk(
    ctx: &ChunkCtx<'_>,
    scratch: &mut ChunkScratch,
    pos: u64,
    frames: usize,
    in_filter: &(dyn Fn(TrackId) -> bool + Sync),
    fan_out_only: &(dyn Fn(TrackId) -> bool + Sync),
    key_only: &(dyn Fn(TrackId) -> bool + Sync),
    key_only_bus: &(dyn Fn(BusId) -> bool + Sync),
    include_master_fx: bool,
    respect_mute_solo: bool,
    freeze_raw: bool,
) {
    scratch.mix_buf[..frames * 2].fill(0.0);
    // CLAP `thread-check`: this thread renders for the chunk (a bounce
    // thread makes main-thread calls between chunks, so only a scope).
    let _audio = crate::clap_host::thread_check::AudioThreadScope::enter();

    // The render graph (audio and MIDI clips, busses, master chain,
    // tracks, plugins), loaded once for the chunk (ARCH-02 A2-4…A2-8): no
    // guard, so no engine-thread edit ever queues behind the chunk.
    let graph = ctx.shared.graph.load();
    let clips = ctx.tuning.apply(&graph.clips);
    let tracks_guard = &*graph.tracks;
    let plugins_guard = &*graph.plugins;

    let active_busses = graph.busses.len().min(scratch.bus_bufs.len());
    // Snapshot solo once (FU-B3a): a bounce can run concurrently with live
    // playback, so the control thread can flip a track's solo mid-chunk.
    // `any_solo` and each track's own flag read later in `track_silenced`
    // must come from this one scan, or the export can render an all-silent
    // chunk the same way the live callback could.
    let any_solo = snapshot_top_level_solo(tracks_guard.values().map(|t| &**t));

    // Aux-send snapshot: the offline bounce taps + sums sends identically
    // to the live path so a bounced/exported WAV matches playback.
    let aux_guard = ctx.shared.aux_sends.load();
    // The bounce renders the same graph as live, so it honours the same
    // key routes. Its taps live in the bounce scratch (a separate audio
    // path with its own block cadence) rather than the audio thread's.
    let sidechain_guard = ctx.shared.sidechain_routes.load();
    scratch.sidechain.begin_block(&sidechain_guard);

    // Take-comp playback table: the offline bounce reads the same published
    // table the live callback does, so a comped / active-take selection
    // renders into the exported WAV exactly as it plays back.
    let take_comp_guard = ctx.shared.take_comp.load();

    // Per-track / sub-track / bus rendering: shared with the live audio
    // callback (`mixer/render_core.rs`). The Bounce strategy swaps the
    // live path's non-blocking locks for deterministic blocking ones
    // (spin + back-off via `lock_plugin_for_bounce`), applies the
    // `in_filter` / `respect_mute_solo` gating, uses constant gains
    // instead of per-block ramps, and skips meter / last-gain atomic
    // writes so a bounce can run concurrently with live playback.
    let strategy = mixer::RenderStrategy::Bounce {
        in_filter,
        fan_out_only,
        key_only,
        key_only_bus,
        send_filter: ctx.send_filter,
        respect_mute_solo,
        freeze_raw,
    };
    scratch.track_slots.ensure(tracks_guard.len());
    mixer::render_block(
        mixer::BlockInputs {
            channels: 2,
            tracks: tracks_guard,
            busses: &graph.busses,
            clips: &clips,
            midi_clips: &graph.midi_clips,
            plugins: plugins_guard,
            tempo_map: ctx.tempo_map,
            sample_rate: ctx.sample_rate,
            any_solo,
            active_busses,
            aux_sends: &aux_guard,
            sidechain_routes: &sidechain_guard,
            take_comp: &take_comp_guard,
            playhead: pos,
            frames,
            latency_comp: ctx.latency_comp,
            automation: ctx.automation,
        },
        &mut mixer::BlockScratch {
            data: &mut scratch.mix_buf[..frames * 2],
            bus_bufs: &mut scratch.bus_bufs,
            slots: &mut scratch.track_slots,
            stash: None,
            port_scratch: &mut scratch.port_scratch,
            note_event_buf: &mut scratch.note_buf,
            sidechain: &mut scratch.sidechain,
            fx_dry: &mut scratch.fx_dry,
            pool: Some(&scratch.pool),
        },
        &strategy,
    );

    // Master FX chain: run over the summed mix in place. Skipped when
    // the caller asked us to leave the raw bus-summed mix alone (so the
    // master FX won't be applied twice when the result plays back).
    if include_master_fx && !ctx.shared.master_fx_bypass.bypassed() {
        let master = &graph.master;
        if !master.plugin_ids.is_empty() {
            for f in 0..frames {
                scratch.track_buf_l[f] = scratch.mix_buf[f * 2];
                scratch.track_buf_r[f] = scratch.mix_buf[f * 2 + 1];
            }
            for &plugin_id in &master.plugin_ids {
                if let Some(slot) = plugins_guard.get(&plugin_id) {
                    // Offline renders see settled bypass states only: a
                    // host-bypassed slot is skipped for the whole file
                    // rather than faded out over its first few
                    // milliseconds (`crate::bypass`).
                    if slot.host_bypassed() {
                        continue;
                    }
                    let mut inst = lock_plugin_for_bounce(slot);
                    slot.sync_own_bypass(&mut inst.0);
                    // Same key routing as the live master chain, so a
                    // bounced mix pumps exactly like playback.
                    let key = scratch.sidechain.key_for(&sidechain_guard, plugin_id);
                    let mut outs = [crate::clap_host::StereoBufMut {
                        left: &mut scratch.track_buf_l[..frames],
                        right: &mut scratch.track_buf_r[..frames],
                    }];
                    inst.0.process_multi_with_key(&mut outs, key, frames);
                }
            }
            for f in 0..frames {
                scratch.mix_buf[f * 2] = scratch.track_buf_l[f];
                scratch.mix_buf[f * 2 + 1] = scratch.track_buf_r[f];
            }
        }
    }

    drop(clips);

    if include_master_fx {
        // A master-gain automation lane ramps across the chunk (start..end
        // sampled at the chunk boundaries); otherwise the static master
        // volume applies as a constant. Evaluated at the comp-delayed
        // position — the summed mix at the master pass is max_latency()
        // behind the raw render position — matching the live mixer
        // (doc #260 finding #9).
        let comp_shift = ctx.latency_comp.max_latency();
        let auto_start =
            crate::mixer::auto_master_volume(ctx.automation, pos.saturating_sub(comp_shift));
        let auto_end = crate::mixer::auto_master_volume(
            ctx.automation,
            (pos + frames as u64).saturating_sub(comp_shift),
        );
        if let (Some(g0), Some(g1)) = (auto_start, auto_end) {
            let inv = if frames > 0 { 1.0 / frames as f32 } else { 0.0 };
            for f in 0..frames {
                let g = g0 + (g1 - g0) * ((f + 1) as f32 * inv);
                scratch.mix_buf[f * 2] *= g;
                scratch.mix_buf[f * 2 + 1] *= g;
            }
        } else {
            for s in &mut scratch.mix_buf[..frames * 2] {
                *s *= ctx.master_vol;
            }
        }
        if ctx.hard_clip {
            for s in &mut scratch.mix_buf[..frames * 2] {
                *s = s.clamp(-1.0, 1.0);
            }
        }
    }
}
