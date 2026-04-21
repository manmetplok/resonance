/// Audio mixing callback: runs on the cpal audio thread.
/// Must be allocation-free (uses pre-allocated buffers).

// Metronome constants
const CLICK_DURATION_SECS: f32 = 0.02;
const CLICK_FREQ_DOWNBEAT: f32 = 1500.0;
const CLICK_FREQ_UPBEAT: f32 = 1000.0;
const CLICK_AMPLITUDE: f32 = 0.3;
const CLICK_DECAY_RATE: f32 = 200.0;

use indexmap::IndexMap;
use ringbuf::traits::{Consumer, Observer};
use std::sync::atomic::Ordering;

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::engine::SharedState;
use crate::types::*;

/// Maximum number of plugin output ports the mixer allocates scratch
/// for per block. The drum plugin uses 7; picking 8 leaves a little
/// headroom for future multi-output plugins without blowing past the
/// scratch-array size.
pub(crate) const MAX_PLUGIN_OUTPUT_PORTS: usize = 8;

/// Latch a pre-captured transport snapshot onto a plugin instance so the
/// next `process()` call delivers it through the CLAP transport event.
#[inline]
fn latch_transport(inst: &mut SyncClapInstance, snap: Option<(f64, u16, u16, bool, f64)>) {
    if let Some((bpm, num, den, playing, pos)) = snap {
        inst.0.set_transport(bpm, num, den, playing, pos);
    }
}

/// Fallback playhead advance used when the audio callback couldn't acquire
/// its locks. No audio is rendered on that path, so we only need to move
/// the playhead forward and handle the loop seam by snapping back — stuck
/// notes and audio content leakage aren't possible when we're outputting
/// silence. The sample-accurate seam handling lives inline in `mix_audio`.
fn advance_playhead_silent(shared: &SharedState, playhead: u64, frames: u64) -> u64 {
    let mut new_playhead = playhead + frames;
    if shared.loop_enabled.load(Ordering::Relaxed) {
        let lo = shared.loop_in.load(Ordering::Relaxed);
        let hi = shared.loop_out.load(Ordering::Relaxed);
        // `>=` matches the main path: when `new_playhead == hi` exactly, we
        // still need to snap back, or the next buffer lands past the loop
        // and never catches up.
        if hi > lo && playhead < hi && new_playhead >= hi {
            new_playhead = lo;
        }
    }
    new_playhead
}

/// Compute stereo gains for a track using equal-power pan law.
#[inline]
fn track_stereo_gains(track: &Track) -> (f32, f32) {
    let volume = track.volume();
    let (pan_l, pan_r) = resonance_dsp::constant_power_pan(track.pan());
    (volume * pan_l, volume * pan_r)
}

/// Compute stereo gains for a bus using the same equal-power pan law.
#[inline]
fn bus_stereo_gains(bus: &Bus) -> (f32, f32) {
    let volume = bus.volume();
    let (pan_l, pan_r) = resonance_dsp::constant_power_pan(bus.pan());
    (volume * pan_l, volume * pan_r)
}

/// Accumulate a source track buffer into a destination stereo pair
/// (separate L/R Vecs, as used by bus summing buffers).
#[inline]
fn sum_to_stereo(
    dst_l: &mut [f32],
    dst_r: &mut [f32],
    frames: usize,
    src_l: &[f32],
    src_r: &[f32],
    gain_l: f32,
    gain_r: f32,
) {
    for f in 0..frames {
        dst_l[f] += src_l[f] * gain_l;
        dst_r[f] += src_r[f] * gain_r;
    }
}

/// De-interleave monitor input into track buffers and process through plugins.
/// Returns the number of frames written. `monitor_temp` is interleaved
/// multi-channel input audio (the raw stream straight from the device);
/// `input_channels` tells us how many channels are in each frame, and the
/// track's own `input_port` picks which channel(s) to route into its
/// stereo L/R pair.
fn process_monitor_track(
    track: &Track,
    monitor_temp: &[f32],
    monitor_frames: usize,
    max_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    transport_snap: Option<(f64, u16, u16, bool, f64)>,
) -> usize {
    let is_mono = track.mono();
    let mix_frames = max_frames.min(monitor_frames);

    track_buf_l[..mix_frames].fill(0.0);
    track_buf_r[..mix_frames].fill(0.0);

    if input_channels == 0 {
        return mix_frames;
    }

    let port = (track.input_port() as usize).min(input_channels - 1);
    let right_port = if is_mono {
        port
    } else {
        (port + 1).min(input_channels - 1)
    };

    for f in 0..mix_frames {
        let base = f * input_channels;
        track_buf_l[f] = monitor_temp[base + port];
        track_buf_r[f] = monitor_temp[base + right_port];
    }

    // Process through plugin chain (skipped when FX are bypassed).
    if !track.fx_bypassed() {
        for &plugin_id in &track.plugin_ids {
            if let Some(si) = plugins_guard.get(&plugin_id) {
                if let Some(mut inst) = si.try_lock() {
                    latch_transport(&mut inst, transport_snap);
                    inst.0.process(
                        &mut track_buf_l[..mix_frames],
                        &mut track_buf_r[..mix_frames],
                        mix_frames,
                    );
                }
            }
        }
    }

    mix_frames
}

/// Sum track buffers into the interleaved output with stereo gains.
#[inline]
fn sum_to_output(
    data: &mut [f32],
    channels: usize,
    frames: usize,
    track_buf_l: &[f32],
    track_buf_r: &[f32],
    gain_l: f32,
    gain_r: f32,
) {
    for f in 0..frames {
        let out_idx = f * channels;
        if channels >= 2 {
            data[out_idx] += track_buf_l[f] * gain_l;
            data[out_idx + 1] += track_buf_r[f] * gain_r;
        } else {
            data[out_idx] += track_buf_l[f] * gain_l + track_buf_r[f] * gain_r;
        }
    }
}

/// Run the master FX insert chain over the interleaved `data` buffer in
/// place. De-interleaves into the borrowed `scratch_l`/`scratch_r` pair
/// (the per-track mix buffers are free at this point in `mix_audio`),
/// processes each plugin in order, then re-interleaves back into `data`.
/// Silently no-ops when the chain is empty, the read lock is contended,
/// or a plugin's instance is momentarily locked by the control thread.
#[inline]
fn apply_master_fx_chain(
    data: &mut [f32],
    channels: usize,
    master: &parking_lot::RwLock<MasterBus>,
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    scratch_l: &mut [f32],
    scratch_r: &mut [f32],
    transport_snap: Option<(f64, u16, u16, bool, f64)>,
) {
    let Some(master_guard) = master.try_read() else {
        return;
    };
    if master_guard.plugin_ids.is_empty() {
        return;
    }
    let output_frames = data.len() / channels;
    let frames = output_frames.min(scratch_l.len()).min(scratch_r.len());
    if frames == 0 {
        return;
    }
    // De-interleave into scratch pair. Mono output shares L across R so
    // plugins see a proper stereo input.
    if channels >= 2 {
        for f in 0..frames {
            let idx = f * channels;
            scratch_l[f] = data[idx];
            scratch_r[f] = data[idx + 1];
        }
    } else {
        for f in 0..frames {
            let s = data[f * channels];
            scratch_l[f] = s;
            scratch_r[f] = s;
        }
    }
    for &plugin_id in &master_guard.plugin_ids {
        if let Some(mutex) = plugins_guard.get(&plugin_id) {
            if let Some(mut inst) = mutex.try_lock() {
                latch_transport(&mut inst, transport_snap);
                inst.0
                    .process(&mut scratch_l[..frames], &mut scratch_r[..frames], frames);
            }
        }
    }
    // Interleave back into data.
    if channels >= 2 {
        for f in 0..frames {
            let idx = f * channels;
            data[idx] = scratch_l[f];
            data[idx + 1] = scratch_r[f];
        }
    } else {
        for f in 0..frames {
            data[f * channels] = 0.5 * (scratch_l[f] + scratch_r[f]);
        }
    }
}

/// Apply master volume, hard clip at [-1.0, 1.0], and update master peak level atomics.
#[inline]
fn apply_master_volume_and_peaks(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
) {
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let output_frames = data.len() / channels;
    let mut master_peak_l = 0.0f32;
    let mut master_peak_r = 0.0f32;
    for f in 0..output_frames {
        let idx = f * channels;
        if channels >= 2 {
            data[idx] = (data[idx] * master_vol).clamp(-1.0, 1.0);
            data[idx + 1] = (data[idx + 1] * master_vol).clamp(-1.0, 1.0);
            master_peak_l = master_peak_l.max(data[idx].abs());
            master_peak_r = master_peak_r.max(data[idx + 1].abs());
        } else {
            data[idx] = (data[idx] * master_vol).clamp(-1.0, 1.0);
            master_peak_l = master_peak_l.max(data[idx].abs());
        }
    }
    if channels < 2 {
        master_peak_r = master_peak_l;
    }
    shared
        .master_peak_l_bits
        .fetch_max(master_peak_l.to_bits(), Ordering::Relaxed);
    shared
        .master_peak_r_bits
        .fetch_max(master_peak_r.to_bits(), Ordering::Relaxed);
}

/// Maximum MIDI events collected per buffer per track. Prevents the
/// pre-allocated `note_event_buf` Vec from reallocating on the audio
/// thread. 512 events = 256 simultaneous note-on + note-off pairs,
/// far beyond what a single buffer can practically contain.
pub(crate) const MAX_MIDI_EVENTS_PER_BUFFER: usize = 512;

/// Collect sample-accurate note events from MIDI clips for a given track and buffer range.
/// Converts tick-based note positions to absolute sample positions using the tempo map.
/// `out` must be pre-allocated and is cleared before use. Stops collecting
/// once `MAX_MIDI_EVENTS_PER_BUFFER` is reached to avoid allocation on the
/// real-time thread.
fn collect_midi_events(
    midi_clips: &[MidiClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    tempo_map: &TempoMap,
    sample_rate: u32,
    out: &mut Vec<PendingNoteEvent>,
) {
    let buf_end = playhead + frames as u64;

    for clip in midi_clips.iter().filter(|c| c.track_id == track_id) {
        let visible_start = clip.trim_start_ticks;
        let visible_end = clip.duration_ticks.saturating_sub(clip.trim_end_ticks);

        for note in &clip.notes {
            // Skip notes outside the visible (trimmed) range
            if note.start_tick + note.duration_ticks <= visible_start {
                continue;
            }
            if note.start_tick >= visible_end {
                continue;
            }

            // Clamp note start/end to visible range
            let effective_start = note.start_tick.max(visible_start);
            let effective_end = (note.start_tick + note.duration_ticks).min(visible_end);

            // Convert to absolute sample positions using the tempo map
            // so tick→sample accounts for tempo changes across the clip.
            let note_abs_start = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_start - visible_start,
                sample_rate,
            );
            let note_abs_end = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_end - visible_start,
                sample_rate,
            );

            // Emit NoteOn if it falls in this buffer
            if note_abs_start >= playhead && note_abs_start < buf_end {
                if out.len() >= MAX_MIDI_EVENTS_PER_BUFFER {
                    break;
                }
                out.push(PendingNoteEvent {
                    is_note_on: true,
                    note: note.note,
                    velocity: note.velocity,
                    sample_offset: (note_abs_start - playhead) as u32,
                });
            }

            // Emit NoteOff if it falls in this buffer
            if note_abs_end >= playhead && note_abs_end < buf_end {
                if out.len() >= MAX_MIDI_EVENTS_PER_BUFFER {
                    break;
                }
                out.push(PendingNoteEvent {
                    is_note_on: false,
                    note: note.note,
                    velocity: 0.0,
                    sample_offset: (note_abs_end - playhead) as u32,
                });
            }
        }
    }

    // Sort by sample offset for CLAP compliance
    out.sort_by_key(|e| e.sample_offset);
}

/// Public version of collect_midi_events for the bounce path.
pub(crate) fn collect_midi_events_bounce(
    midi_clips: &[MidiClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    tempo_map: &TempoMap,
    sample_rate: u32,
    out: &mut Vec<PendingNoteEvent>,
) {
    out.clear();
    collect_midi_events(midi_clips, track_id, playhead, frames, tempo_map, sample_rate, out);
}

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
fn render_timeline_block(
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
        if track.muted() {
            continue;
        }
        if any_solo && !track.soloed() {
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
            let mut plugin_iter = track.plugin_ids.iter();
            if let Some(&instrument_id) = plugin_iter.next() {
                if let Some(mutex) = plugins_guard.get(&instrument_id) {
                    if let Some(mut inst) = mutex.try_lock() {
                        latch_transport(&mut inst, transport_snap);
                        for event in note_event_buf.iter() {
                            if event.is_note_on {
                                inst.0.queue_note_on(event.note, event.velocity, event.sample_offset);
                            } else {
                                inst.0.queue_note_off(event.note, event.sample_offset);
                            }
                        }

                        let port_count = inst
                            .0
                            .output_port_count()
                            .min(port_scratch.len());
                        if port_count > 1 {
                            // Multi-output instrument: fan out into the
                            // per-port scratch pool, then copy port 0 back
                            // into the track's main buffer so the rest of
                            // the track chain (effects + fader + bus
                            // routing) runs unchanged.
                            {
                                let mut views: [Option<StereoBufMut<'_>>;
                                    MAX_PLUGIN_OUTPUT_PORTS] = Default::default();
                                for (i, (pl, pr)) in port_scratch
                                    .iter_mut()
                                    .take(port_count)
                                    .enumerate()
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
                                    unsafe { std::mem::MaybeUninit::uninit().assume_init() };
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
                                for i in 0..port_count {
                                    unsafe { slots[i].assume_init_drop() };
                                }
                            }
                            // Port 0 → main track buffer for effect chain.
                            track_buf_l[..frames]
                                .copy_from_slice(&port_scratch[0].0[..frames]);
                            track_buf_r[..frames]
                                .copy_from_slice(&port_scratch[0].1[..frames]);
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
                    }
                }
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
            if !track.plugin_ids.is_empty() && !track.fx_bypassed() {
                for &plugin_id in &track.plugin_ids {
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
                data, channels, frames, track_buf_l, track_buf_r, gain_l, gain_r,
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
                // `plugin_ids` is treated as an audio effect and is
                // subject to the sub-track's own FX-bypass flag.
                if !sub_track.fx_bypassed() {
                    let (pl, pr) = &mut port_scratch[port_idx];
                    for &plugin_id in &sub_track.plugin_ids {
                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                            if let Some(mut inst) = mutex.try_lock() {
                                latch_transport(&mut inst, transport_snap);
                                inst.0.process(
                                    &mut pl[..frames],
                                    &mut pr[..frames],
                                    frames,
                                );
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
                        inst.0.process(
                            &mut bus_buf_l[..frames],
                            &mut bus_buf_r[..frames],
                            frames,
                        );
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

/// Fire all-notes-off on every instrument track's primary plugin. Used at
/// the loop seam to prevent notes started before `loop_out` from hanging
/// after the playhead snaps back to `loop_in`.
fn panic_instrument_tracks(
    tracks_guard: &IndexMap<TrackId, Track>,
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
) {
    for track in tracks_guard.values() {
        if track.track_type != TrackType::Instrument {
            continue;
        }
        let Some(&inst_id) = track.plugin_ids.first() else {
            continue;
        };
        let Some(mutex) = plugins_guard.get(&inst_id) else {
            continue;
        };
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.all_notes_off();
        }
    }
}

/// Mix audio from all active clips into the output buffer.
/// This runs on the cpal audio callback thread -- must be allocation-free
/// (uses pre-allocated track_buf_l/track_buf_r).
pub(crate) fn mix_audio(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    tracks: &parking_lot::RwLock<IndexMap<TrackId, Track>>,
    busses: &parking_lot::RwLock<IndexMap<BusId, Bus>>,
    master: &parking_lot::RwLock<MasterBus>,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    midi_clips: &parking_lot::RwLock<Vec<MidiClip>>,
    plugins: &parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>,
    tempo_map: &parking_lot::RwLock<TempoMap>,
    sample_rate: u32,
    track_buf_l: &mut Vec<f32>,
    track_buf_r: &mut Vec<f32>,
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    // Per-plugin-output-port scratch used for multi-output instruments
    // (e.g. resonance-drums with its 7 group/overhead ports). Sized to
    // `MAX_PLUGIN_OUTPUT_PORTS` pairs by the engine; mix_audio only
    // touches the first N slots on any given block, where N is the
    // active plugin's declared port count.
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    monitor_cons: &mut ringbuf::HeapCons<f32>,
    monitor_temp: &mut Vec<f32>,
    buf_frames: usize,
    quantum: usize,
) {
    // Zero the output buffer
    data.fill(0.0);

    let output_frames = data.len() / channels;
    let frames = output_frames.min(buf_frames);

    // Snapshot tempo once per block. Hold the read guard so the bar
    // table is available for tempo-map-aware MIDI tick→sample conversion
    // in the rendering path. The read lock is held for one audio buffer
    // (~1 ms) — writers (engine thread) wait only during tempo changes.
    let playhead_now = shared.playhead.load(std::sync::atomic::Ordering::Relaxed);
    struct TempoSnap { bpm: f64, num: u16, den: u16, metronome: bool }
    let tempo_guard = tempo_map.try_read();
    let tempo_snap = tempo_guard.as_ref().map(|tm| {
        TempoSnap {
            bpm: tm.bpm as f64,
            num: tm.numerator as u16,
            den: tm.denominator as u16,
            metronome: tm.metronome_enabled,
        }
    });
    let snap_bpm = tempo_snap.as_ref().map(|s| s.bpm).unwrap_or(120.0);

    let transport_snap: Option<(f64, u16, u16, bool, f64)> = tempo_snap.as_ref().map(|s| {
        let playing = shared.playing.load(std::sync::atomic::Ordering::Relaxed);
        let pos = playhead_now as f64 / sample_rate as f64 * s.bpm / 60.0;
        (s.bpm, s.num, s.den, playing, pos)
    });

    // Read monitor input with jitter margin to avoid underflows.
    // Skip stale monitor data to keep latency at ~1 buffer period.
    // The monitor stream carries raw interleaved multi-channel data now
    // (one `input_channels` block per frame), so sample counts scale by
    // the current input channel count.
    let input_channels = shared.input_channels.load(Ordering::Relaxed) as usize;
    let frame_stride = input_channels.max(1);
    let needed = frames * frame_stride;
    let available = monitor_cons.occupied_len();
    if available > needed + quantum * frame_stride {
        monitor_cons.skip(available - needed);
    }
    let to_read = needed.min(monitor_cons.occupied_len());
    let monitor_samples = monitor_cons.pop_slice(&mut monitor_temp[..to_read]);
    let monitor_frames = monitor_samples / frame_stride;

    // Count-in branch: hold the playhead, skip track/clip rendering,
    // and emit metronome ticks from a count-in-local elapsed counter
    // so the last click lands exactly one beat before the punch-in
    // line. `count_in_active` stays set across the brief window
    // between `count_in_remaining` hitting zero and the engine
    // control thread opening the recording stream, so the playhead
    // stays pinned to the punch-in line throughout.
    if shared.count_in_active.load(Ordering::Relaxed) {
        let count_in_remaining = shared.count_in_remaining.load(Ordering::Relaxed);
        let count_in_total = shared.count_in_total.load(Ordering::Relaxed);
        let elapsed_at_start = count_in_total.saturating_sub(count_in_remaining);
        let click_frames = (output_frames as u64).min(count_in_remaining) as usize;

        // Monitor pass-through so the performer can hear themselves
        // warm up during the count-in. Mirrors the playing=false
        // monitor branch but is best-effort on lock contention —
        // dropping monitor audio for one buffer is acceptable; losing
        // the count-in tick is not.
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            if let (Some(tracks_guard), Some(plugins_guard)) =
                (tracks.try_read(), plugins.try_read())
            {
                let any_solo = tracks_guard.values().any(|t| t.soloed());
                let is_audible = |t: &&Track| -> bool {
                    t.monitor_enabled() && !t.muted() && (!any_solo || t.soloed())
                };
                if let Some(track) = tracks_guard.values().find(|t| is_audible(&t)) {
                    let processed_frames = process_monitor_track(
                        track,
                        monitor_temp,
                        monitor_frames,
                        monitor_frames,
                        input_channels,
                        track_buf_l,
                        track_buf_r,
                        &plugins_guard,
                        transport_snap,
                    );
                    let (gain_l, gain_r) = track_stereo_gains(track);
                    let mut peak_l = 0.0f32;
                    let mut peak_r = 0.0f32;
                    for f in 0..processed_frames {
                        peak_l = peak_l.max((track_buf_l[f] * gain_l).abs());
                        peak_r = peak_r.max((track_buf_r[f] * gain_r).abs());
                    }
                    track.update_peak_l(peak_l);
                    track.update_peak_r(peak_r);
                    sum_to_output(
                        data,
                        channels,
                        processed_frames,
                        track_buf_l,
                        track_buf_r,
                        gain_l,
                        gain_r,
                    );
                }
            }
        }

        // Metronome click synthesis using a count-in-local timeline.
        // Beats are indexed from the start of the count-in; with
        // `count_in_total == precount_bars * numerator * spb`, the
        // final click in the loop lands at elapsed
        // `(precount_bars * numerator - 1) * spb`, leaving exactly
        // one beat of silence before the punch-in line.
        if let Some(tm) = tempo_map.try_read() {
            let spb = tm.samples_per_beat(sample_rate);
            let numerator = tm.numerator as u64;
            let click_duration_samples =
                (sample_rate as f32 * CLICK_DURATION_SECS) as u64;
            for frame_offset in 0..click_frames {
                let elapsed = elapsed_at_start + frame_offset as u64;
                let beat_index = (elapsed as f64 / spb).floor();
                let beat_start = (beat_index * spb).round() as u64;
                let beat_pos = elapsed.saturating_sub(beat_start);
                if beat_pos < click_duration_samples {
                    let t = beat_pos as f32 / sample_rate as f32;
                    let beat_in_bar = (beat_index as u64) % numerator;
                    let freq = if beat_in_bar == 0 {
                        CLICK_FREQ_DOWNBEAT
                    } else {
                        CLICK_FREQ_UPBEAT
                    };
                    let amplitude = CLICK_AMPLITUDE * (-t * CLICK_DECAY_RATE).exp();
                    let click =
                        amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();
                    let out_idx = frame_offset * channels;
                    if channels >= 2 {
                        data[out_idx] += click;
                        data[out_idx + 1] += click;
                    } else {
                        data[out_idx] += click;
                    }
                }
            }
        }

        // Master volume + peaks so the count-in audio hits meters the
        // same way normal playback does.
        apply_master_volume_and_peaks(data, channels, shared);

        // Decrement the remaining-clicks counter. Once it hits zero
        // the metronome goes quiet, but `count_in_active` keeps the
        // mixer in this branch until the engine control thread has
        // actually opened the recording stream — that cross-thread
        // handoff is what guarantees the playhead doesn't start
        // advancing until recording is armed.
        let new_remaining = count_in_remaining.saturating_sub(output_frames as u64);
        shared
            .count_in_remaining
            .store(new_remaining, Ordering::Relaxed);
        return;
    }

    if !shared.playing.load(Ordering::Relaxed) {
        // Even when stopped, output monitored audio for armed tracks
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            let (Some(tracks_guard), Some(plugins_guard)) =
                (tracks.try_read(), plugins.try_read())
            else {
                return;
            };
            let any_solo = tracks_guard.values().any(|t| t.soloed());
            let is_audible =
                |t: &&Track| -> bool { t.monitor_enabled() && !t.muted() && (!any_solo || t.soloed()) };
            let any_monitor = tracks_guard.values().any(|t| is_audible(&t));

            if any_monitor {
                if let Some(track) = tracks_guard.values().find(|t| is_audible(&t)) {
                    let processed_frames = process_monitor_track(
                        track,
                        monitor_temp,
                        monitor_frames,
                        monitor_frames,
                        input_channels,
                        track_buf_l,
                        track_buf_r,
                        &plugins_guard,
                        transport_snap,
                    );

                    let (gain_l, gain_r) = track_stereo_gains(track);

                    // Compute post-fader peak levels for VU meters
                    let mut peak_l = 0.0f32;
                    let mut peak_r = 0.0f32;
                    for f in 0..processed_frames {
                        peak_l = peak_l.max((track_buf_l[f] * gain_l).abs());
                        peak_r = peak_r.max((track_buf_r[f] * gain_r).abs());
                    }
                    track.update_peak_l(peak_l);
                    track.update_peak_r(peak_r);

                    sum_to_output(
                        data,
                        channels,
                        processed_frames,
                        track_buf_l,
                        track_buf_r,
                        gain_l,
                        gain_r,
                    );
                }

                // Apply master volume and compute master peak levels
                apply_master_volume_and_peaks(data, channels, shared);
            }
        }
        return;
    }

    let playhead = shared.playhead.load(Ordering::Relaxed);

    let (
        Some(tracks_guard),
        Some(busses_guard),
        Some(clips_guard),
        Some(midi_clips_guard),
        Some(plugins_guard),
    ) = (
        tracks.try_read(),
        busses.try_read(),
        clips.try_read(),
        midi_clips.try_read(),
        plugins.try_read(),
    )
    else {
        // Lock contended -- advance playhead to avoid desync, output silence this buffer
        let new_playhead =
            advance_playhead_silent(shared, playhead, output_frames as u64);
        shared.playhead.store(new_playhead, Ordering::Relaxed);
        return;
    };

    let active_busses = busses_guard.len().min(bus_bufs.len());

    // Resolve a &TempoMap for tempo-map-aware MIDI tick→sample conversion.
    let default_tm = TempoMap::default();
    let tm_ref: &TempoMap = tempo_guard.as_deref().unwrap_or(&default_tm);

    let any_solo = tracks_guard
        .values()
        .filter(|t| t.sub_track_of.is_none())
        .any(|t| t.soloed());

    // Detect a loop seam inside this buffer. When the callback reaches or
    // crosses `loop_out`, we render two sub-blocks: the pre-wrap portion
    // from the current playhead, then (after an all-notes-off on instrument
    // plugins) the post-wrap portion starting from `loop_in`. This gives
    // sample-accurate cycle playback — no silent gap and no stray audio
    // from past `loop_out` bleeding across the seam.
    //
    // The `>=` on the end-of-block check is load-bearing: when the buffer
    // size divides the loop length exactly (common with small pro-audio
    // quanta like 128 frames), a strict `>` would miss the seam every time
    // — the block would end exactly on `loop_out` and the next block would
    // start past it, failing the `playhead < hi` test. With `>=`, that
    // aligned case renders the full block as `head` and sets `tail = 0`,
    // snapping the playhead back to `loop_in` for the next buffer.
    let seam_split: Option<(usize, usize, u64)> = if shared
        .loop_enabled
        .load(Ordering::Relaxed)
    {
        let lo = shared.loop_in.load(Ordering::Relaxed);
        let hi = shared.loop_out.load(Ordering::Relaxed);
        if hi > lo && playhead < hi && playhead + frames as u64 >= hi {
            let head = (hi - playhead) as usize;
            let tail = frames - head;
            Some((head, tail, lo))
        } else {
            None
        }
    } else {
        None
    };

    let new_playhead = if let Some((head_frames, tail_frames, loop_in)) = seam_split {
        // ---- Pre-wrap sub-block (plays to `loop_out`) ---------------------
        let head_monitor_frames = monitor_frames.min(head_frames);
        render_timeline_block(
            &mut data[..head_frames * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            playhead,
            head_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp[..head_monitor_frames * frame_stride],
            head_monitor_frames,
            input_channels,
            transport_snap,
        );

        // Flush instrument voices at the seam.
        panic_instrument_tracks(&tracks_guard, &plugins_guard);

        // ---- Post-wrap sub-block (plays from `loop_in`) -------------------
        let tail_monitor_start = head_monitor_frames * frame_stride;
        let tail_monitor_avail = monitor_frames.saturating_sub(head_monitor_frames);
        let tail_monitor_frames = tail_monitor_avail.min(tail_frames);
        render_timeline_block(
            &mut data[head_frames * channels..(head_frames + tail_frames) * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            loop_in,
            tail_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp[tail_monitor_start..tail_monitor_start + tail_monitor_frames * frame_stride],
            tail_monitor_frames,
            input_channels,
            transport_snap,
        );

        loop_in + tail_frames as u64
    } else {
        render_timeline_block(
            &mut data[..frames * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            playhead,
            frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp[..monitor_frames * frame_stride],
            monitor_frames,
            input_channels,
            transport_snap,
        );
        playhead + output_frames as u64
    };

    // Master FX chain: run over the full callback buffer post-bus-sum,
    // before the metronome click is layered in and before the master
    // volume pass. Skipped when globally bypassed.
    if !shared.master_fx_bypassed.load(Ordering::Relaxed) {
        apply_master_fx_chain(
            data,
            channels,
            master,
            &plugins_guard,
            track_buf_l,
            track_buf_r,
            transport_snap,
        );
    }

    drop(plugins_guard);

    // Metronome click synthesis. When a loop seam split the callback, the
    // mapping from output frame index to timeline frame changes at the seam:
    // frames before `head_frames` play from `playhead`, frames after play
    // from `loop_in`.
    if let Some(snap) = tempo_snap.as_ref() {
        if snap.metronome {
            let click_duration_samples = (sample_rate as f32 * CLICK_DURATION_SECS) as u64;

            // Collect beat boundaries near the buffer into a small stack
            // array: (beat_sample, is_downbeat). At most ~8 beats can
            // overlap one audio buffer even at extreme tempos.
            let mut beats = [(0u64, false); 16];
            let mut n_beats = 0usize;

            // Determine the timeline ranges covered by this buffer.
            // With a loop seam there are two disjoint ranges.
            let ranges: [(u64, u64); 2];
            let n_ranges;
            match seam_split {
                Some((head, tail, loop_in)) => {
                    ranges = [
                        (playhead, playhead + head as u64),
                        (loop_in, loop_in + tail as u64),
                    ];
                    n_ranges = 2;
                }
                None => {
                    ranges = [
                        (playhead, playhead + output_frames as u64),
                        (0, 0),
                    ];
                    n_ranges = 1;
                }
            }

            if tm_ref.bar_count() > 0 {
                for ri in 0..n_ranges {
                    let (r_start, r_end) = ranges[ri];
                    let search_start = r_start.saturating_sub(click_duration_samples);
                    let Some(start_bar) = tm_ref.bar_index_at(search_start) else {
                        continue;
                    };
                    let mut bar_idx = start_bar;
                    'bar: loop {
                        let num_beats = tm_ref.beats_in_bar(bar_idx);
                        for beat in 0..num_beats {
                            let Some(bs) = tm_ref.beat_sample_in_bar(bar_idx, beat, sample_rate)
                            else {
                                break 'bar;
                            };
                            if bs >= r_end {
                                break 'bar;
                            }
                            if bs + click_duration_samples > r_start && n_beats < 16 {
                                beats[n_beats] = (bs, beat == 0);
                                n_beats += 1;
                            }
                        }
                        bar_idx += 1;
                        if bar_idx >= tm_ref.bar_count() {
                            break;
                        }
                    }
                }
            } else {
                // No bar table — flat BPM beat positions.
                let spb = sample_rate as f64 * 60.0 / snap_bpm;
                let numerator = snap.num as u64;
                for ri in 0..n_ranges {
                    let (r_start, r_end) = ranges[ri];
                    let search_start = r_start.saturating_sub(click_duration_samples);
                    let first_beat = (search_start as f64 / spb).floor() as u64;
                    let mut bi = first_beat;
                    loop {
                        let bs = (bi as f64 * spb).round() as u64;
                        if bs >= r_end {
                            break;
                        }
                        if bs + click_duration_samples > r_start && n_beats < 16 {
                            beats[n_beats] = (bs, bi % numerator == 0);
                            n_beats += 1;
                        }
                        bi += 1;
                    }
                }
            }

            // Render clicks: for each frame, check if any beat is active.
            for frame_offset in 0..output_frames {
                let timeline_frame = match seam_split {
                    Some((head, _, loop_in)) if frame_offset >= head => {
                        loop_in + (frame_offset - head) as u64
                    }
                    _ => playhead + frame_offset as u64,
                };
                for bi in 0..n_beats {
                    let (beat_sample, is_downbeat) = beats[bi];
                    let beat_pos = timeline_frame.saturating_sub(beat_sample);
                    if beat_pos < click_duration_samples && timeline_frame >= beat_sample {
                        let t = beat_pos as f32 / sample_rate as f32;
                        let freq = if is_downbeat {
                            CLICK_FREQ_DOWNBEAT
                        } else {
                            CLICK_FREQ_UPBEAT
                        };
                        let amplitude = CLICK_AMPLITUDE * (-t * CLICK_DECAY_RATE).exp();
                        let click =
                            amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();
                        let out_idx = frame_offset * channels;
                        if channels >= 2 {
                            data[out_idx] += click;
                            data[out_idx + 1] += click;
                        } else {
                            data[out_idx] += click;
                        }
                        break;
                    }
                }
            }
        }
    }

    // Apply master volume, hard clip, and compute master peak levels
    apply_master_volume_and_peaks(data, channels, shared);

    shared.playhead.store(new_playhead, Ordering::Relaxed);
}
