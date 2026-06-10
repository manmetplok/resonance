//! Per-block timeline rendering: walks every active track + bus,
//! mixes audio clips, dispatches MIDI events to instrument plugins,
//! routes per-port multi-output instruments through their sub-tracks,
//! and sums into the master output (or per-bus summing buffer).
//!
//! Called once per buffer in the no-seam path, twice (head + tail)
//! when a buffer crosses a loop boundary. Allocation-free.

use indexmap::IndexMap;

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::types::*;

use super::common::{
    bus_stereo_gains, latch_transport, sum_to_output, sum_to_stereo, track_stereo_gains,
};
use super::midi_events::collect_midi_events;
use super::midi_stash::MidiStash;
use super::monitor::MAX_PLUGIN_OUTPUT_PORTS;

/// Render one contiguous timeline sub-block into a slice of the output.
/// Separated from `mix_audio` so that a buffer which crosses the loop seam
/// can be rendered as two sub-blocks (pre-wrap and post-wrap) with different
/// `playhead` values, giving sample-accurate cycle playback.
///
/// The caller is responsible for:
/// - Passing `data` sliced to exactly `frames * channels` samples.
/// - Passing `monitor_temp` sliced to the corresponding portion of this
///   callback's live input (monitor is timeline-independent — it streams
///   linearly across the full callback, not per sub-block's playhead).
/// - Clearing the output buffer before the first call.
/// - Running the metronome and master-volume passes once over the full
///   callback buffer afterwards.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_timeline_block(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &IndexMap<TrackId, Track>,
    busses_guard: &IndexMap<BusId, Bus>,
    clips_guard: &[AudioClip],
    midi_clips_guard: &[MidiClip],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
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
    midi_stash: &mut MidiStash,
    monitor_temp: &[f32],
    monitor_frames: usize,
    input_channels: usize,
    transport_snap: Option<(f64, u16, u16, bool, f64)>,
) {
    // Zero every active bus summing buffer at the start of the sub-block so
    // tracks can accumulate into them.
    for (buf_l, buf_r) in bus_bufs.iter_mut().take(active_busses) {
        buf_l[..frames].fill(0.0);
        buf_r[..frames].fill(0.0);
    }

    // Per-track processing: (clips + monitor input) -> plugins -> volume -> master.
    // Sub-tracks are skipped here; they're driven by their parent's plugin
    // fan-out later in the same track pass.
    for track in tracks_guard.values() {
        if track.sub_track_of.is_some() {
            continue;
        }
        // Muted / solo-suppressed instrument tracks still run their
        // instrument plugin below (audio discarded) so NoteOffs keep
        // flowing and voices don't stick on unmute; other tracks are
        // skipped outright.
        let silenced = track.muted() || (any_solo && !track.soloed());
        if silenced && track.track_type != TrackType::Instrument {
            continue;
        }

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
            note_event_buf.clear();
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
                    if let Some(mut inst) = mutex.try_lock() {
                        latch_transport(&mut inst, transport_snap);
                        // Replay events parked during earlier lock
                        // contention before this block's events.
                        midi_stash.deliver(instrument_id, &mut *inst);
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
                            {
                                let mut views: [Option<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] =
                                    Default::default();
                                for (i, (pl, pr)) in
                                    port_scratch.iter_mut().take(port_count).enumerate()
                                {
                                    pl[..frames].fill(0.0);
                                    pr[..frames].fill(0.0);
                                    views[i] = Some(StereoBufMut {
                                        left: &mut pl[..frames],
                                        right: &mut pr[..frames],
                                    });
                                }
                                // Build a contiguous slice of StereoBufMut
                                // for the CLAP call. We know ports 0..port_count
                                // are Some.
                                let mut slots: [std::mem::MaybeUninit<StereoBufMut<'_>>;
                                    MAX_PLUGIN_OUTPUT_PORTS] =
                                    [const { std::mem::MaybeUninit::uninit() };
                                        MAX_PLUGIN_OUTPUT_PORTS];
                                for i in 0..port_count {
                                    slots[i].write(views[i].take().unwrap());
                                }
                                // SAFETY: the first `port_count` slots are
                                // initialized above; the slice only refers
                                // to those.
                                let slice: &mut [StereoBufMut<'_>] = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        slots.as_mut_ptr() as *mut StereoBufMut<'_>,
                                        port_count,
                                    )
                                };
                                inst.0.process_multi(slice, frames);
                                // Drop the initialized entries before the
                                // MaybeUninit array goes out of scope.
                                for slot in slots.iter_mut().take(port_count) {
                                    unsafe { slot.assume_init_drop() };
                                }
                            }
                            // Port 0 → main track buffer for effect chain.
                            track_buf_l[..frames].copy_from_slice(&port_scratch[0].0[..frames]);
                            track_buf_r[..frames].copy_from_slice(&port_scratch[0].1[..frames]);
                            extra_ports_filled = port_count;
                            has_audio = true;
                        } else {
                            // Single-output path (legacy plugins): use the
                            // thin wrapper that re-targets onto track_buf_l/r.
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
                                frames,
                            );
                            has_audio = true;
                        }
                    } else {
                        // UI thread holds the plugin lock (param drag /
                        // autosave / reload): park this block's events so
                        // they replay on the next successful lock instead
                        // of dropping them. The one-block audio dropout
                        // is accepted for now (future work: crossfade).
                        midi_stash.stash(instrument_id, note_event_buf);
                    }
                }
            }
            // Silenced track: the instrument ran (voice state stays
            // consistent) but its output is discarded.
            if silenced {
                continue;
            }
            // Effect plugins (skipped when the track's FX are bypassed;
            // the instrument itself still ran above).
            if !track.fx_bypassed() {
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        if let Some(mut inst) = mutex.try_lock() {
                            latch_transport(&mut inst, transport_snap);
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

            // Mix monitor input for all tracks with monitoring enabled.
            // Each track pulls its own channel(s) from the interleaved
            // multi-channel monitor buffer based on its input_port.
            if track.monitor_enabled() && monitor_frames > 0 && input_channels > 0 {
                let is_mono = track.mono();
                let mix_frames = frames.min(monitor_frames);
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
                        if let Some(mut inst) = mutex.try_lock() {
                            latch_transport(&mut inst, transport_snap);
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

        if !has_audio {
            continue;
        }

        // Apply track volume + pan and sum to the track's destination.
        let (gain_l, gain_r) = track_stereo_gains(track);

        // Compute post-fader peak levels for VU meters
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        for f in 0..frames {
            peak_l = peak_l.max((track_buf_l[f] * gain_l).abs());
            peak_r = peak_r.max((track_buf_r[f] * gain_r).abs());
        }
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);

        // Route post-fader audio: either directly to the master interleaved
        // output or into the target bus's summing buffer. If the target bus
        // no longer exists (e.g. removed mid-block), fall back to master so
        // the track isn't silenced.
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
                if sub_track.muted() {
                    continue;
                }
                let (sub_gain_l, sub_gain_r) = track_stereo_gains(sub_track);

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
                            if let Some(mut inst) = mutex.try_lock() {
                                latch_transport(&mut inst, transport_snap);
                                inst.0.process(&mut pl[..frames], &mut pr[..frames], frames);
                            }
                        }
                    }
                }

                // Peak levels for sub-track VU meter.
                let (pl, pr) = &port_scratch[port_idx];
                let mut sub_peak_l = 0.0f32;
                let mut sub_peak_r = 0.0f32;
                for f in 0..frames {
                    sub_peak_l = sub_peak_l.max((pl[f] * sub_gain_l).abs());
                    sub_peak_r = sub_peak_r.max((pr[f] * sub_gain_r).abs());
                }
                sub_track.update_peak_l(sub_peak_l);
                sub_track.update_peak_r(sub_peak_r);

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
            }
        }
    }

    // Per-bus processing: plugin chain, volume/pan, peaks, sum to master.
    for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
        if bus.muted() {
            continue;
        }
        let (bus_buf_l, bus_buf_r) = &mut bus_bufs[bus_idx];

        // Process bus plugin chain in place over the accumulated buffer
        // (skipped when the bus's FX are bypassed).
        if !bus.fx_bypassed() {
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    if let Some(mut inst) = mutex.try_lock() {
                        latch_transport(&mut inst, transport_snap);
                        inst.0
                            .process(&mut bus_buf_l[..frames], &mut bus_buf_r[..frames], frames);
                    }
                }
            }
        }

        // Apply bus volume + pan and compute post-fader peaks.
        let (bus_gain_l, bus_gain_r) = bus_stereo_gains(bus);
        let mut bus_peak_l = 0.0f32;
        let mut bus_peak_r = 0.0f32;
        for f in 0..frames {
            bus_peak_l = bus_peak_l.max((bus_buf_l[f] * bus_gain_l).abs());
            bus_peak_r = bus_peak_r.max((bus_buf_r[f] * bus_gain_r).abs());
        }
        bus.update_peak_l(bus_peak_l);
        bus.update_peak_r(bus_peak_r);

        // Sum the bus output into master.
        sum_to_output(
            data, channels, frames, bus_buf_l, bus_buf_r, bus_gain_l, bus_gain_r,
        );
    }
}
