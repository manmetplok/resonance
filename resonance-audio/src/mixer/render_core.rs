//! Shared per-track render core used by both the live audio callback
//! (`track_block::render_timeline_block`) and the offline bounce
//! renderer (`engine::bounce::render::render_chunk`).
//!
//! The block structure — per-track clip/MIDI/plugin processing, the
//! multi-output instrument fan-out, sub-track routing, latency
//! compensation, and the per-bus pass — is identical on both paths.
//! What differs is captured by [`RenderStrategy`]:
//!
//! - **Live**: non-blocking `try_lock` on plugins (dropping out for one
//!   block on contention, with MIDI parked in the [`MidiStash`]),
//!   transport latching, monitor-input mixing, per-sample gain ramps
//!   from the last-gain atomics (with mute fade-out blocks and the
//!   silenced-instrument path that keeps NoteOffs flowing), and VU peak
//!   metering.
//! - **Bounce**: deterministic blocking locks (spin + back-off so the
//!   audio thread isn't starved), `in_filter` / `respect_mute_solo`
//!   gating, constant gains (a ramp with equal endpoints degenerates to
//!   the constant — bit-identical), and no meter or last-gain atomic
//!   writes, since a bounce may run concurrently with live playback.

use indexmap::IndexMap;
use parking_lot::{Mutex, MutexGuard};

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::latency::LatencyComp;
use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
use crate::types::*;

use super::common::{
    bus_stereo_gains, latch_transport, ramped_stereo_peaks, sum_to_output, sum_to_stereo,
    track_stereo_gains, TransportSnap,
};
use super::midi_events::collect_midi_events;
use super::midi_stash::MidiStash;

/// Per-call policy for the bits of the render block that differ between
/// the live callback and the offline bounce. See the module docs.
pub(crate) enum RenderStrategy<'a> {
    Live {
        midi_stash: &'a mut MidiStash,
        transport_snap: Option<TransportSnap>,
        monitor_temp: &'a [f32],
        monitor_frames: usize,
        input_channels: usize,
    },
    Bounce {
        in_filter: &'a dyn Fn(TrackId) -> bool,
        respect_mute_solo: bool,
    },
}

/// How a top-level track participates in this block, as decided by the
/// strategy's gating rules.
struct TrackDisposition {
    /// `(previous, target)` gain ramp endpoints per channel. Bounce uses
    /// equal endpoints, which the ramp helpers reduce to a constant.
    gain_l: (f32, f32),
    gain_r: (f32, f32),
    /// Live: muted or solo-suppressed. Inherited by sub-tracks so a
    /// silenced parent fades its fan-out in the same block.
    silenced: bool,
    /// Live: the instrument still runs (NoteOffs keep flowing, voices
    /// don't stick on unmute) but its output is discarded once the mute
    /// ramp has fully faded the previous gain to zero.
    discard_after_instrument: bool,
}

impl RenderStrategy<'_> {
    /// Live-only side effects: VU peak meters and the last-gain atomics
    /// that seed the next block's ramp. Bounce must not touch either —
    /// it can run while live playback owns them.
    #[inline]
    fn is_live(&self) -> bool {
        matches!(self, Self::Live { .. })
    }

    /// Acquire an effect plugin's lock. Live: non-blocking, skipping the
    /// plugin for this block on contention (and latching the transport
    /// snapshot on success). Bounce: blocking with spin + back-off.
    #[inline]
    fn lock_fx<'p>(
        &self,
        mutex: &'p Mutex<SyncClapInstance>,
    ) -> Option<MutexGuard<'p, SyncClapInstance>> {
        match self {
            Self::Live { transport_snap, .. } => {
                let mut inst = mutex.try_lock()?;
                latch_transport(&mut inst, *transport_snap);
                Some(inst)
            }
            Self::Bounce { .. } => Some(crate::engine::try_lock_with_backoff(mutex)),
        }
    }

    /// Acquire an instrument plugin's lock. Live additionally replays
    /// events parked during earlier lock contention before the caller
    /// queues this block's events.
    #[inline]
    fn lock_instrument<'p>(
        &mut self,
        mutex: &'p Mutex<SyncClapInstance>,
        id: PluginInstanceId,
    ) -> Option<MutexGuard<'p, SyncClapInstance>> {
        match self {
            Self::Live {
                midi_stash,
                transport_snap,
                ..
            } => {
                let mut inst = mutex.try_lock()?;
                latch_transport(&mut inst, *transport_snap);
                midi_stash.deliver(id, &mut *inst);
                Some(inst)
            }
            Self::Bounce { .. } => Some(crate::engine::try_lock_with_backoff(mutex)),
        }
    }

    /// Live: the UI thread holds the plugin lock (param drag / autosave /
    /// reload) — park this block's events so they replay on the next
    /// successful lock instead of dropping them. The one-block audio
    /// dropout is accepted for now (future work: crossfade). Bounce
    /// locks never fail, so this is unreachable there.
    #[inline]
    fn instrument_lock_failed(&mut self, id: PluginInstanceId, events: &[PendingNoteEvent]) {
        if let Self::Live { midi_stash, .. } = self {
            midi_stash.stash(id, events);
        }
    }

    /// Decide whether and how a top-level track renders this block.
    fn track_disposition(&self, track: &Track, any_solo: bool) -> Option<TrackDisposition> {
        match self {
            Self::Live { .. } => {
                // Muted / solo-suppressed instrument tracks still run
                // their instrument plugin (audio discarded) so NoteOffs
                // keep flowing; other tracks are skipped outright —
                // except for one extra block after silencing, which
                // renders normally with a target gain of 0.0 so the
                // mute ramps out instead of hard-cutting.
                let silenced = track.muted() || (any_solo && !track.soloed());
                let (last_gain_l, last_gain_r) = track.last_gains();
                let faded_out = last_gain_l == 0.0 && last_gain_r == 0.0;
                if silenced && faded_out && track.track_type != TrackType::Instrument {
                    return None;
                }
                let (target_gain_l, target_gain_r) = if silenced {
                    (0.0, 0.0)
                } else {
                    track_stereo_gains(track)
                };
                Some(TrackDisposition {
                    gain_l: (last_gain_l, target_gain_l),
                    gain_r: (last_gain_r, target_gain_r),
                    silenced,
                    discard_after_instrument: silenced && faded_out,
                })
            }
            Self::Bounce {
                in_filter,
                respect_mute_solo,
            } => {
                // For `to_wav` we honour the user's mix (muted /
                // non-soloed tracks drop out). For bounce-in-place
                // `in_filter` already gates to the source + sub-tracks
                // — and the source is explicitly muted by
                // `finalize_bounce` after every successful bounce, so
                // respecting `muted` would silence every re-bounce.
                if *respect_mute_solo && (track.muted() || (any_solo && !track.soloed())) {
                    return None;
                }
                if !in_filter(track.id) {
                    return None;
                }
                let (gain_l, gain_r) = track_stereo_gains(track);
                Some(TrackDisposition {
                    gain_l: (gain_l, gain_l),
                    gain_r: (gain_r, gain_r),
                    silenced: false,
                    discard_after_instrument: false,
                })
            }
        }
    }

    /// Decide whether and how a sub-track renders its parent's port.
    /// Returns the `(gain_l, gain_r)` ramp endpoints.
    fn sub_track_disposition(
        &self,
        sub_track: &Track,
        any_solo: bool,
        parent_silenced: bool,
    ) -> Option<((f32, f32), (f32, f32))> {
        match self {
            Self::Live { .. } => {
                // A silenced parent fades its sub-tracks out in the same
                // block; once fully faded the fan-out stops running and
                // the subs stay at zero.
                let sub_silenced = sub_track.muted() || parent_silenced;
                let (sub_last_l, sub_last_r) = sub_track.last_gains();
                if sub_silenced && sub_last_l == 0.0 && sub_last_r == 0.0 {
                    return None;
                }
                let (sub_target_l, sub_target_r) = if sub_silenced {
                    (0.0, 0.0)
                } else {
                    track_stereo_gains(sub_track)
                };
                Some(((sub_last_l, sub_target_l), (sub_last_r, sub_target_r)))
            }
            Self::Bounce {
                in_filter,
                respect_mute_solo,
            } => {
                if *respect_mute_solo
                    && (sub_track.muted() || (any_solo && !sub_track.soloed()))
                {
                    return None;
                }
                if !in_filter(sub_track.id) {
                    return None;
                }
                let (gain_l, gain_r) = track_stereo_gains(sub_track);
                Some(((gain_l, gain_l), (gain_r, gain_r)))
            }
        }
    }

    /// Decide whether and how a bus renders. Live fades a muted bus out
    /// (its FX keep running until the ramp lands on zero); bounce skips
    /// muted busses outright.
    fn bus_disposition(&self, bus: &Bus) -> Option<((f32, f32), (f32, f32))> {
        match self {
            Self::Live { .. } => {
                let bus_silenced = bus.muted();
                let (bus_last_l, bus_last_r) = bus.last_gains();
                if bus_silenced && bus_last_l == 0.0 && bus_last_r == 0.0 {
                    return None;
                }
                let (bus_target_l, bus_target_r) = if bus_silenced {
                    (0.0, 0.0)
                } else {
                    bus_stereo_gains(bus)
                };
                Some(((bus_last_l, bus_target_l), (bus_last_r, bus_target_r)))
            }
            Self::Bounce { .. } => {
                if bus.muted() {
                    return None;
                }
                let (gain_l, gain_r) = bus_stereo_gains(bus);
                Some(((gain_l, gain_l), (gain_r, gain_r)))
            }
        }
    }

    /// Live-only: mix the track's live input channel(s) from the
    /// interleaved multi-channel monitor buffer. Returns whether any
    /// monitor audio was added.
    fn mix_monitor(
        &self,
        track: &Track,
        track_buf_l: &mut [f32],
        track_buf_r: &mut [f32],
        frames: usize,
    ) -> bool {
        let Self::Live {
            monitor_temp,
            monitor_frames,
            input_channels,
            ..
        } = self
        else {
            return false;
        };
        if !track.monitor_enabled() || *monitor_frames == 0 || *input_channels == 0 {
            return false;
        }
        let is_mono = track.mono();
        let mix_frames = frames.min(*monitor_frames);
        let port = (track.input_port() as usize).min(input_channels - 1);
        let right_port = if is_mono {
            port
        } else {
            (port + 1).min(input_channels - 1)
        };
        for f in 0..mix_frames {
            let base = f * input_channels;
            track_buf_l[f] += monitor_temp[base + port];
            track_buf_r[f] += monitor_temp[base + right_port];
        }
        true
    }
}

/// Multi-output instrument fan-out: zero the first `port_count` port
/// scratch pairs, build a contiguous `StereoBufMut` slice over them,
/// and run `process_multi`.
fn process_multi_port(
    inst: &mut SyncClapInstance,
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    port_count: usize,
    frames: usize,
) {
    let mut views: [Option<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] = Default::default();
    for (i, (pl, pr)) in port_scratch.iter_mut().take(port_count).enumerate() {
        pl[..frames].fill(0.0);
        pr[..frames].fill(0.0);
        views[i] = Some(StereoBufMut {
            left: &mut pl[..frames],
            right: &mut pr[..frames],
        });
    }
    // Build a contiguous slice of StereoBufMut for the CLAP call. We
    // know ports 0..port_count are Some.
    let mut slots: [std::mem::MaybeUninit<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] =
        [const { std::mem::MaybeUninit::uninit() }; MAX_PLUGIN_OUTPUT_PORTS];
    for i in 0..port_count {
        slots[i].write(views[i].take().unwrap());
    }
    // SAFETY: the first `port_count` slots are initialized above; the
    // slice only refers to those.
    let slice: &mut [StereoBufMut<'_>] = unsafe {
        std::slice::from_raw_parts_mut(slots.as_mut_ptr() as *mut StereoBufMut<'_>, port_count)
    };
    inst.0.process_multi(slice, frames);
    // Drop the initialized entries before the MaybeUninit array goes
    // out of scope.
    for slot in slots.iter_mut().take(port_count) {
        unsafe { slot.assume_init_drop() };
    }
}

/// Render one contiguous timeline block into the interleaved output:
/// walks every active track + bus, mixes audio clips, dispatches MIDI
/// events to instrument plugins, routes per-port multi-output
/// instruments through their sub-tracks, and sums into the output (or
/// per-bus summing buffer). Allocation-free.
///
/// The caller is responsible for:
/// - Passing `data` sliced to exactly `frames * channels` samples and
///   cleared before the first call.
/// - Running any master FX / metronome / master-volume passes afterwards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_block(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &IndexMap<TrackId, Track>,
    busses_guard: &IndexMap<BusId, Bus>,
    clips_guard: &[AudioClip],
    midi_clips_guard: &[MidiClip],
    plugins_guard: &IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>,
    tempo_map: &TempoMap,
    sample_rate: u32,
    any_solo: bool,
    active_busses: usize,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    latency_comp: &LatencyComp,
    strategy: &mut RenderStrategy<'_>,
) {
    // Zero every active bus summing buffer at the start of the block so
    // tracks can accumulate into them.
    for (buf_l, buf_r) in bus_bufs.iter_mut().take(active_busses) {
        buf_l[..frames].fill(0.0);
        buf_r[..frames].fill(0.0);
    }

    // Per-track processing: (clips + monitor input) -> plugins -> volume
    // -> master. Sub-tracks are skipped here; they're driven by their
    // parent's plugin fan-out later in the same track pass.
    for track in tracks_guard.values() {
        if track.sub_track_of.is_some() {
            continue;
        }
        let Some(TrackDisposition {
            gain_l,
            gain_r,
            silenced,
            discard_after_instrument,
        }) = strategy.track_disposition(track, any_solo)
        else {
            continue;
        };

        // Zero per-track buffers
        track_buf_l[..frames].fill(0.0);
        track_buf_r[..frames].fill(0.0);

        let mut has_audio = false;
        // Sub-track fan-out book-keeping: how many extra output ports the
        // instrument plugin filled on this block, so the post-plugin loop
        // knows how many `port_scratch` entries to route to sub-tracks.
        let mut extra_ports_filled: usize = 0;

        if track.track_type == TrackType::Instrument {
            // -- Instrument track: collect MIDI events, send to instrument plugin --
            collect_midi_events(
                midi_clips_guard,
                track.id,
                playhead,
                frames,
                tempo_map,
                sample_rate,
                note_event_buf,
            );

            // Process: first plugin is the instrument (receives note events),
            // remaining plugins are effects (audio-only).
            let track_plugins = track.plugins();
            let mut plugin_iter = track_plugins.iter();
            if let Some(&instrument_id) = plugin_iter.next() {
                if let Some(mutex) = plugins_guard.get(&instrument_id) {
                    if let Some(mut inst) = strategy.lock_instrument(mutex, instrument_id) {
                        for event in note_event_buf.iter() {
                            if event.is_note_on {
                                inst.0.queue_note_on(
                                    event.note,
                                    event.velocity,
                                    event.sample_offset,
                                );
                            } else {
                                inst.0.queue_note_off(event.note, event.sample_offset);
                            }
                        }

                        let port_count = inst.0.output_port_count().min(port_scratch.len());
                        if port_count > 1 {
                            // Multi-output instrument: fan out into the
                            // per-port scratch pool, then copy port 0 back
                            // into the track's main buffer so the rest of
                            // the track chain (effects + fader + bus
                            // routing) runs unchanged.
                            process_multi_port(&mut inst, port_scratch, port_count, frames);
                            track_buf_l[..frames].copy_from_slice(&port_scratch[0].0[..frames]);
                            track_buf_r[..frames].copy_from_slice(&port_scratch[0].1[..frames]);
                            extra_ports_filled = port_count;
                        } else {
                            // Single-output path (legacy plugins): use the
                            // thin wrapper that re-targets onto track_buf_l/r.
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
                                frames,
                            );
                        }
                        has_audio = true;
                    } else {
                        strategy.instrument_lock_failed(instrument_id, note_event_buf);
                    }
                }
            }
            // Silenced track (live): the instrument ran (voice state stays
            // consistent) but its output is discarded — once the mute
            // ramp has finished fading the previous gain to zero.
            if discard_after_instrument {
                continue;
            }
            // Effect plugins (skipped when the track's FX are bypassed;
            // the instrument itself still ran above).
            if !track.fx_bypassed() {
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        if let Some(mut inst) = strategy.lock_fx(mutex) {
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
                                frames,
                            );
                            has_audio = true;
                        }
                    }
                }
            }
        } else {
            // -- Audio track: mix clips + monitor input + plugin chain --

            // Mix monitor input for all tracks with monitoring enabled
            // (live path only).
            if strategy.mix_monitor(track, track_buf_l, track_buf_r, frames) {
                has_audio = true;
            }

            // Accumulate all clips for this track into de-interleaved track buffers
            for clip in clips_guard.iter() {
                if clip.track_id != track.id {
                    continue;
                }

                let clip_frames = clip.duration_frames();
                let clip_start = clip.start_sample;
                let clip_end = clip_start + clip_frames;
                let buf_start = playhead;
                let buf_end = playhead + frames as u64;

                if buf_end <= clip_start || buf_start >= clip_end {
                    continue;
                }

                let overlap_start = buf_start.max(clip_start);
                let overlap_end = buf_end.min(clip_end);

                let clip_data = clip.source.as_frames();
                for timeline_frame in overlap_start..overlap_end {
                    let frame_offset = (timeline_frame - buf_start) as usize;
                    let clip_frame =
                        (timeline_frame - clip_start) as usize + clip.trim_start_frames as usize;
                    let clip_idx = clip_frame * 2;
                    if clip_idx + 1 < clip_data.len() {
                        track_buf_l[frame_offset] += clip_data[clip_idx];
                        track_buf_r[frame_offset] += clip_data[clip_idx + 1];
                        has_audio = true;
                    }
                }
            }

            // Process through plugin chain (skipped when FX are bypassed).
            let track_plugins = track.plugins();
            if !track_plugins.is_empty() && !track.fx_bypassed() {
                for &plugin_id in track_plugins.iter() {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        if let Some(mut inst) = strategy.lock_fx(mutex) {
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
                                frames,
                            );
                            has_audio = true;
                        }
                    }
                }
            }
        }

        // Plugin-delay compensation: delay the post-chain signal so
        // every track reaches master with the same total latency (see
        // `crate::latency`). Runs even when the track produced no audio
        // this block so delayed tails keep flushing.
        if latency_comp.apply(
            track.id,
            &mut track_buf_l[..frames],
            &mut track_buf_r[..frames],
            playhead,
        ) {
            has_audio = true;
        }

        if !has_audio {
            // Nothing to ramp over: snap the remembered gain to the
            // target so changes made during silence don't ramp later.
            if strategy.is_live() {
                track.set_last_gains(gain_l.1, gain_r.1);
            }
            continue;
        }

        // Compute post-fader peak levels for VU meters (live only).
        if strategy.is_live() {
            let (peak_l, peak_r) =
                ramped_stereo_peaks(track_buf_l, track_buf_r, frames, gain_l, gain_r);
            track.update_peak_l(peak_l);
            track.update_peak_r(peak_r);
        }

        // Route post-fader audio: either directly to the interleaved
        // output or into the target bus's summing buffer. If the target
        // bus no longer exists (e.g. removed mid-block), fall back to
        // master so the track isn't silenced.
        let routed_to_bus = match track.output() {
            TrackOutput::Bus(bus_id) => busses_guard
                .get_index_of(&bus_id)
                .filter(|idx| *idx < active_busses)
                .map(|idx| {
                    let (bl, br) = &mut bus_bufs[idx];
                    sum_to_stereo(bl, br, frames, track_buf_l, track_buf_r, gain_l, gain_r);
                })
                .is_some(),
            TrackOutput::Master => false,
        };
        if !routed_to_bus {
            sum_to_output(
                data,
                channels,
                frames,
                track_buf_l,
                track_buf_r,
                gain_l,
                gain_r,
            );
        }
        if strategy.is_live() {
            track.set_last_gains(gain_l.1, gain_r.1);
        }

        // Sub-track fan-out: for every non-main plugin output port that
        // was filled by the instrument above, look up the matching
        // sub-track (if any) and route its scratch buffer through the
        // sub-track's fader / pan / bus.
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
                let Some((sub_gain_l, sub_gain_r)) =
                    strategy.sub_track_disposition(sub_track, any_solo, silenced)
                else {
                    continue;
                };

                // Run the sub-track's own effect chain in place on its
                // port buffer, before peak metering and bus/master routing.
                // Sub-tracks never host an instrument, so every entry in
                // the plugin chain is treated as an audio effect and is
                // subject to the sub-track's own FX-bypass flag.
                if !sub_track.fx_bypassed() {
                    let (pl, pr) = &mut port_scratch[port_idx];
                    let sub_plugins = sub_track.plugins();
                    for &plugin_id in sub_plugins.iter() {
                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                            if let Some(mut inst) = strategy.lock_fx(mutex) {
                                inst.0.process(&mut pl[..frames], &mut pr[..frames], frames);
                            }
                        }
                    }
                }

                // Plugin-delay compensation for the sub-track's chain.
                {
                    let (pl, pr) = &mut port_scratch[port_idx];
                    latency_comp.apply(
                        sub_track.id,
                        &mut pl[..frames],
                        &mut pr[..frames],
                        playhead,
                    );
                }

                // Peak levels for sub-track VU meter (live only).
                let (pl, pr) = &port_scratch[port_idx];
                if strategy.is_live() {
                    let (sub_peak_l, sub_peak_r) =
                        ramped_stereo_peaks(pl, pr, frames, sub_gain_l, sub_gain_r);
                    sub_track.update_peak_l(sub_peak_l);
                    sub_track.update_peak_r(sub_peak_r);
                }

                // Route post-fader audio to the sub-track's destination.
                let routed = match sub_track.output() {
                    TrackOutput::Bus(bus_id) => busses_guard
                        .get_index_of(&bus_id)
                        .filter(|idx| *idx < active_busses)
                        .map(|idx| {
                            let (bl, br) = &mut bus_bufs[idx];
                            sum_to_stereo(bl, br, frames, pl, pr, sub_gain_l, sub_gain_r);
                        })
                        .is_some(),
                    TrackOutput::Master => false,
                };
                if !routed {
                    sum_to_output(data, channels, frames, pl, pr, sub_gain_l, sub_gain_r);
                }
                if strategy.is_live() {
                    sub_track.set_last_gains(sub_gain_l.1, sub_gain_r.1);
                }
            }
        }
    }

    // Per-bus processing: plugin chain, volume/pan, peaks, sum to master.
    for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
        let Some((bus_gain_l, bus_gain_r)) = strategy.bus_disposition(bus) else {
            continue;
        };
        let (bus_buf_l, bus_buf_r) = &mut bus_bufs[bus_idx];

        // Process bus plugin chain in place over the accumulated buffer
        // (skipped when the bus's FX are bypassed).
        if !bus.fx_bypassed() {
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    if let Some(mut inst) = strategy.lock_fx(mutex) {
                        inst.0
                            .process(&mut bus_buf_l[..frames], &mut bus_buf_r[..frames], frames);
                    }
                }
            }
        }

        // Compute post-fader peaks (live only).
        if strategy.is_live() {
            let (bus_peak_l, bus_peak_r) =
                ramped_stereo_peaks(bus_buf_l, bus_buf_r, frames, bus_gain_l, bus_gain_r);
            bus.update_peak_l(bus_peak_l);
            bus.update_peak_r(bus_peak_r);
        }

        // Sum the bus output into master.
        sum_to_output(
            data, channels, frames, bus_buf_l, bus_buf_r, bus_gain_l, bus_gain_r,
        );
        if strategy.is_live() {
            bus.set_last_gains(bus_gain_l.1, bus_gain_r.1);
        }
    }
}
