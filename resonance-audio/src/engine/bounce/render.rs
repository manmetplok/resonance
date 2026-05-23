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

use crate::clap_host::{StereoBufMut, SyncClapInstance};
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
    for (bl, br) in scratch.bus_bufs.iter_mut().take(active_busses) {
        bl[..frames].fill(0.0);
        br[..frames].fill(0.0);
    }

    let any_solo = tracks_guard.values().any(|t| t.soloed());

    for track in tracks_guard.values() {
        // Sub-tracks are driven by their parent's plugin fan-out below;
        // skip them here so they don't run as standalone instrument
        // tracks (they have no MIDI clips of their own and no plugins).
        if track.sub_track_of.is_some() {
            continue;
        }
        // For `to_wav` we honour the user's mix (muted/non-soloed tracks
        // drop out). For `to_audio_clip` (bounce-in-place) `in_filter`
        // already gates to the source + sub-tracks — and the source is
        // explicitly muted by `finalize_bounce` after every successful
        // bounce, so respecting `muted` here would silence every
        // re-bounce of the same track.
        if respect_mute_solo {
            if track.muted() {
                continue;
            }
            if any_solo && !track.soloed() {
                continue;
            }
        }
        if !in_filter(track.id) {
            continue;
        }

        scratch.track_buf_l[..frames].fill(0.0);
        scratch.track_buf_r[..frames].fill(0.0);
        let mut has_audio = false;
        // How many extra non-main output ports the source instrument
        // filled this chunk. Drives the sub-track fan-out below — set
        // for multi-output plugins (e.g. resonance-drums) and left at
        // zero for single-output synths.
        let mut extra_ports_filled: usize = 0;

        if track.track_type == TrackType::Instrument {
            // Instrument track: collect MIDI events and process.
            scratch.note_buf.clear();
            mixer::collect_midi_events_bounce(
                &midi_guard,
                track.id,
                pos,
                frames,
                ctx.tempo_map,
                ctx.sample_rate,
                &mut scratch.note_buf,
            );
            let mut plugin_iter = track.plugin_ids.iter();
            if let Some(&inst_id) = plugin_iter.next() {
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    let mut inst = lock_plugin_for_bounce(mutex);
                    for ev in scratch.note_buf.iter() {
                        if ev.is_note_on {
                            inst.0.queue_note_on(ev.note, ev.velocity, ev.sample_offset);
                        } else {
                            inst.0.queue_note_off(ev.note, ev.sample_offset);
                        }
                    }

                    let port_count = inst.0.output_port_count().min(scratch.port_scratch.len());
                    if port_count > 1 {
                        // Multi-output instrument (e.g. drums). Build a
                        // port_count-element StereoBufMut slice into the
                        // per-port scratch pool, render via process_multi,
                        // then copy port 0 into the track buffer for the
                        // parent's effect chain. Ports 1..N are routed to
                        // their sub-tracks below.
                        let mut views: [Option<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] =
                            Default::default();
                        for (i, (pl, pr)) in
                            scratch.port_scratch.iter_mut().take(port_count).enumerate()
                        {
                            pl[..frames].fill(0.0);
                            pr[..frames].fill(0.0);
                            views[i] = Some(StereoBufMut {
                                left: &mut pl[..frames],
                                right: &mut pr[..frames],
                            });
                        }
                        let mut slots: [std::mem::MaybeUninit<StereoBufMut<'_>>;
                            MAX_PLUGIN_OUTPUT_PORTS] =
                            [const { std::mem::MaybeUninit::uninit() };
                                MAX_PLUGIN_OUTPUT_PORTS];
                        for i in 0..port_count {
                            slots[i].write(views[i].take().unwrap());
                        }
                        // SAFETY: the first `port_count` slots above are
                        // initialized; the slice only refers to those.
                        let slice: &mut [StereoBufMut<'_>] = unsafe {
                            std::slice::from_raw_parts_mut(
                                slots.as_mut_ptr() as *mut StereoBufMut<'_>,
                                port_count,
                            )
                        };
                        inst.0.process_multi(slice, frames);
                        for slot in slots.iter_mut().take(port_count) {
                            unsafe { slot.assume_init_drop() };
                        }
                        scratch.track_buf_l[..frames]
                            .copy_from_slice(&scratch.port_scratch[0].0[..frames]);
                        scratch.track_buf_r[..frames]
                            .copy_from_slice(&scratch.port_scratch[0].1[..frames]);
                        extra_ports_filled = port_count;
                    } else {
                        // Single-output: render directly into the track buf.
                        inst.0.process(
                            &mut scratch.track_buf_l[..frames],
                            &mut scratch.track_buf_r[..frames],
                            frames,
                        );
                    }
                    has_audio = true;
                }
            }
            if !track.fx_bypassed() {
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        let mut inst = lock_plugin_for_bounce(mutex);
                        inst.0.process(
                            &mut scratch.track_buf_l[..frames],
                            &mut scratch.track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
                    }
                }
            }
        } else {
            // Audio track: mix clips + plugin chain.
            for clip in clips_guard.iter() {
                if clip.track_id != track.id {
                    continue;
                }
                let clip_start = clip.start_sample;
                let clip_end = clip_start + clip.duration_frames();
                let buf_end = pos + frames as u64;
                if buf_end <= clip_start || pos >= clip_end {
                    continue;
                }
                let overlap_start = pos.max(clip_start);
                let overlap_end = buf_end.min(clip_end);
                let clip_data = clip.source.as_frames();
                for timeline_frame in overlap_start..overlap_end {
                    let frame_offset = (timeline_frame - pos) as usize;
                    let clip_frame = (timeline_frame - clip_start) as usize
                        + clip.trim_start_frames as usize;
                    let clip_idx = clip_frame * 2;
                    if clip_idx + 1 < clip_data.len() {
                        scratch.track_buf_l[frame_offset] += clip_data[clip_idx];
                        scratch.track_buf_r[frame_offset] += clip_data[clip_idx + 1];
                        has_audio = true;
                    }
                }
            }

            // Process through plugin chain (skipped when bypassed).
            if !track.plugin_ids.is_empty() && !track.fx_bypassed() {
                for &plugin_id in &track.plugin_ids {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        let mut inst = lock_plugin_for_bounce(mutex);
                        inst.0.process(
                            &mut scratch.track_buf_l[..frames],
                            &mut scratch.track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
                    }
                }
            }
        }

        if !has_audio {
            continue;
        }

        // Apply track volume + pan, route to master or bus.
        let volume = track.volume();
        let (pan_l, pan_r) = resonance_dsp::constant_power_pan(track.pan());
        let gain_l = volume * pan_l;
        let gain_r = volume * pan_r;

        let routed_to_bus = match track.output() {
            TrackOutput::Bus(bus_id) => busses_guard
                .get_index_of(&bus_id)
                .filter(|idx| *idx < active_busses)
                .map(|idx| {
                    let (bl, br) = &mut scratch.bus_bufs[idx];
                    for f in 0..frames {
                        bl[f] += scratch.track_buf_l[f] * gain_l;
                        br[f] += scratch.track_buf_r[f] * gain_r;
                    }
                })
                .is_some(),
            TrackOutput::Master => false,
        };
        if !routed_to_bus {
            for f in 0..frames {
                scratch.mix_buf[f * 2] += scratch.track_buf_l[f] * gain_l;
                scratch.mix_buf[f * 2 + 1] += scratch.track_buf_r[f] * gain_r;
            }
        }

        // Sub-track fan-out: for every non-main output port the source
        // instrument filled this chunk, look up the matching sub-track
        // and route its scratch buffer through the sub-track's effect
        // chain + fader + bus/master. Mirrors `track_block.rs`'s live
        // path so a multi-output drum kit bounces every kit piece, not
        // just port 0 (Main).
        if extra_ports_filled > 1 {
            for sub_track in tracks_guard.values() {
                let Some((parent_id, port_idx)) = sub_track.sub_track_of else {
                    continue;
                };
                if parent_id != track.id {
                    continue;
                }
                let port_idx = port_idx as usize;
                if port_idx == 0 || port_idx >= extra_ports_filled {
                    continue;
                }
                if respect_mute_solo {
                    if sub_track.muted() {
                        continue;
                    }
                    if any_solo && !sub_track.soloed() {
                        continue;
                    }
                }
                if !in_filter(sub_track.id) {
                    continue;
                }

                // Sub-track effect chain runs in place on its port buffer.
                if !sub_track.fx_bypassed() {
                    let (pl, pr) = &mut scratch.port_scratch[port_idx];
                    for &plugin_id in &sub_track.plugin_ids {
                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                            let mut inst = lock_plugin_for_bounce(mutex);
                            inst.0.process(&mut pl[..frames], &mut pr[..frames], frames);
                        }
                    }
                }

                let sub_volume = sub_track.volume();
                let (sub_pan_l, sub_pan_r) = resonance_dsp::constant_power_pan(sub_track.pan());
                let sub_gain_l = sub_volume * sub_pan_l;
                let sub_gain_r = sub_volume * sub_pan_r;

                let (pl, pr) = &scratch.port_scratch[port_idx];
                let routed = match sub_track.output() {
                    TrackOutput::Bus(bus_id) => busses_guard
                        .get_index_of(&bus_id)
                        .filter(|idx| *idx < active_busses)
                        .map(|idx| {
                            let (bl, br) = &mut scratch.bus_bufs[idx];
                            for f in 0..frames {
                                bl[f] += pl[f] * sub_gain_l;
                                br[f] += pr[f] * sub_gain_r;
                            }
                        })
                        .is_some(),
                    TrackOutput::Master => false,
                };
                if !routed {
                    for f in 0..frames {
                        scratch.mix_buf[f * 2] += pl[f] * sub_gain_l;
                        scratch.mix_buf[f * 2 + 1] += pr[f] * sub_gain_r;
                    }
                }
            }
        }
    }

    // Per-bus plugin chain + volume/pan + sum to master.
    for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
        if bus.muted() {
            continue;
        }
        let (bl, br) = &mut scratch.bus_bufs[bus_idx];
        if !bus.fx_bypassed() {
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    let mut inst = lock_plugin_for_bounce(mutex);
                    inst.0.process(&mut bl[..frames], &mut br[..frames], frames);
                }
            }
        }
        let bus_volume = bus.volume();
        let (bus_pan_l, bus_pan_r) = resonance_dsp::constant_power_pan(bus.pan());
        let bus_gain_l = bus_volume * bus_pan_l;
        let bus_gain_r = bus_volume * bus_pan_r;
        for f in 0..frames {
            scratch.mix_buf[f * 2] += bl[f] * bus_gain_l;
            scratch.mix_buf[f * 2 + 1] += br[f] * bus_gain_r;
        }
    }

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
