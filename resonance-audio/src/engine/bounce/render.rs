//! Shared chunked render core for both bounce entry points.
//!
//! `to_wav` and `to_audio_clip` both drive `render_chunk` in a loop;
//! the only differences are where the output goes and whether the
//! master FX chain runs. The chunk scratch buffers live here so both
//! call sites can size and allocate them identically.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use parking_lot::{Mutex, MutexGuard, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::latency::LatencyComp;
use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
use crate::mixer;
use crate::types::*;

use super::super::{SharedState, MAX_BUSSES};

pub(super) const BOUNCE_CHUNK: usize = 1024;

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
/// `__test_support` module in `lib.rs`.
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
}

impl ChunkScratch {
    pub(super) fn new() -> Self {
        Self {
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
        }
    }
}

/// Read-only context shared by every chunk in a bounce run. Holds
/// references to the engine's locked state so the render loop can
/// re-acquire each lock per chunk (matching live playback's contention
/// pattern).
pub(super) struct ChunkCtx<'a> {
    pub shared: &'a Arc<SharedState>,
    pub tracks: &'a Arc<RwLock<IndexMap<TrackId, Track>>>,
    pub busses: &'a Arc<RwLock<IndexMap<BusId, Bus>>>,
    pub master: &'a Arc<RwLock<MasterBus>>,
    pub clips: &'a Arc<RwLock<Vec<AudioClip>>>,
    pub midi_clips: &'a Arc<RwLock<Vec<MidiClip>>>,
    pub plugins: &'a Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    pub tempo_map: &'a TempoMap,
    pub sample_rate: u32,
    pub master_vol: f32,
    /// Plugin-delay-compensation table for this bounce run, built once
    /// by [`build_latency_comp`] so the offline render aligns tracks
    /// exactly like live playback. The bounce drivers additionally trim
    /// the leading `max_latency()` frames from the output so the
    /// rendered audio lands on the timeline with zero net shift.
    pub latency_comp: &'a LatencyComp,
}

/// Build a fresh compensation table from the current topology, reading
/// each plugin's activation-time latency. Runs on the bounce thread —
/// allocation is fine here.
pub(super) fn build_latency_comp(
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
) -> LatencyComp {
    let tracks_guard = tracks.read();
    let busses_guard = busses.read();
    let plugins_guard = plugins.read();
    let chains = crate::latency::chain_latencies(&tracks_guard, &busses_guard, |id| {
        plugins_guard
            .get(&id)
            .map(|m| lock_plugin_for_bounce(m).0.latency_samples() as u64)
            .unwrap_or(0)
    });
    let (max, delays) = crate::latency::compensation_delays(&chains);
    LatencyComp::new(max, &delays)
}

/// Reset every plugin so the bounce starts from a clean state. Without
/// this, leftover envelope phase / reverb tail / etc. from previous
/// playback would bleed into the first frame.
pub(super) fn reset_plugins(
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
) {
    let plugins_guard = plugins.read();
    for mutex in plugins_guard.values() {
        let mut inst = lock_plugin_for_bounce(mutex);
        inst.0.reset_processing();
    }
}

/// Render one chunk into `scratch.mix_buf`. The output is interleaved
/// stereo of length `frames * 2`. When `include_master_fx` is true,
/// master FX, master volume and hard-clip are applied; otherwise the
/// raw bus-summed mix is left for the caller (used by bounce-in-place
/// so master FX aren't applied twice on playback).
///
/// The closure `in_filter` decides which tracks contribute. Any track
/// for which the closure returns false is skipped exactly like a muted
/// one — but its bus isn't drained either, so reverb tails on shared
/// buses still come from the in-filter tracks only.
///
/// Reference A/B exclusion: this shared bounce core renders the mix
/// only. It takes no [`crate::engine::reference::ReferenceMonitor`] and
/// never reads `ctx.shared.reference`, so the live A/B selection cannot
/// leak into any offline export — the reference monitor tap lives solely
/// in the live callback (`mixer::mix_audio`).
pub(super) fn render_chunk(
    ctx: &ChunkCtx<'_>,
    scratch: &mut ChunkScratch,
    pos: u64,
    frames: usize,
    in_filter: &dyn Fn(TrackId) -> bool,
    include_master_fx: bool,
    respect_mute_solo: bool,
) {
    scratch.mix_buf[..frames * 2].fill(0.0);

    let tracks_guard = ctx.tracks.read();
    let busses_guard = ctx.busses.read();
    let clips_guard = ctx.clips.read();
    let midi_guard = ctx.midi_clips.read();
    let plugins_guard = ctx.plugins.read();

    let active_busses = busses_guard.len().min(scratch.bus_bufs.len());
    let any_solo = any_top_level_solo(tracks_guard.values());

    // Aux-send snapshot: the offline bounce taps + sums sends identically
    // to the live path so a bounced/exported WAV matches playback.
    let aux_guard = ctx.shared.aux_sends.load();

    // Per-track / sub-track / bus rendering: shared with the live audio
    // callback (`mixer/render_core.rs`). The Bounce strategy swaps the
    // live path's non-blocking locks for deterministic blocking ones
    // (spin + back-off via `lock_plugin_for_bounce`), applies the
    // `in_filter` / `respect_mute_solo` gating, uses constant gains
    // instead of per-block ramps, and skips meter / last-gain atomic
    // writes so a bounce can run concurrently with live playback.
    let mut strategy = mixer::RenderStrategy::Bounce {
        in_filter,
        respect_mute_solo,
    };
    mixer::render_block(
        &mut scratch.mix_buf[..frames * 2],
        2,
        &tracks_guard,
        &busses_guard,
        &clips_guard,
        &midi_guard,
        &plugins_guard,
        ctx.tempo_map,
        ctx.sample_rate,
        any_solo,
        active_busses,
        &aux_guard,
        pos,
        frames,
        &mut scratch.track_buf_l,
        &mut scratch.track_buf_r,
        &mut scratch.bus_bufs,
        &mut scratch.port_scratch,
        &mut scratch.note_buf,
        ctx.latency_comp,
        &mut strategy,
    );

    // Master FX chain: run over the summed mix in place. Skipped when
    // the caller asked us to leave the raw bus-summed mix alone (so the
    // master FX won't be applied twice when the result plays back).
    if include_master_fx && !ctx.shared.master_fx_bypassed.load(Ordering::Relaxed) {
        let master_guard = ctx.master.read();
        if !master_guard.plugin_ids.is_empty() {
            for f in 0..frames {
                scratch.track_buf_l[f] = scratch.mix_buf[f * 2];
                scratch.track_buf_r[f] = scratch.mix_buf[f * 2 + 1];
            }
            for &plugin_id in &master_guard.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    let mut inst = lock_plugin_for_bounce(mutex);
                    inst.0.process(
                        &mut scratch.track_buf_l[..frames],
                        &mut scratch.track_buf_r[..frames],
                        frames,
                    );
                }
            }
            for f in 0..frames {
                scratch.mix_buf[f * 2] = scratch.track_buf_l[f];
                scratch.mix_buf[f * 2 + 1] = scratch.track_buf_r[f];
            }
        }
    }

    drop(plugins_guard);
    drop(clips_guard);
    drop(busses_guard);
    drop(tracks_guard);

    if include_master_fx {
        for s in &mut scratch.mix_buf[..frames * 2] {
            *s = (*s * ctx.master_vol).clamp(-1.0, 1.0);
        }
    }
}
