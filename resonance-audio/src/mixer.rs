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

use crate::clap_host::SyncClapInstance;
use crate::engine::SharedState;
use crate::types::*;

/// Compute stereo gains for a track using equal-power pan law.
#[inline]
fn track_stereo_gains(track: &Track) -> (f32, f32) {
    let volume = track.volume();
    let (pan_l, pan_r) = resonance_dsp::constant_power_pan(track.pan());
    (volume * pan_l, volume * pan_r)
}

/// De-interleave monitor input into track buffers and process through plugins.
/// Returns the number of frames written.
fn process_monitor_track(
    track: &Track,
    monitor_temp: &[f32],
    monitor_frames: usize,
    max_frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
) -> usize {
    let is_mono = track.mono();
    let mix_frames = max_frames.min(monitor_frames);

    track_buf_l[..mix_frames].fill(0.0);
    track_buf_r[..mix_frames].fill(0.0);

    for f in 0..mix_frames {
        let l = monitor_temp[f * 2];
        if is_mono {
            track_buf_l[f] = l;
            track_buf_r[f] = l;
        } else {
            track_buf_l[f] = l;
            track_buf_r[f] = monitor_temp[f * 2 + 1];
        }
    }

    // Process through plugin chain
    for &plugin_id in &track.plugin_ids {
        if let Some(si) = plugins_guard.get(&plugin_id) {
            if let Some(mut inst) = si.try_lock() {
                inst.0.process(
                    &mut track_buf_l[..mix_frames],
                    &mut track_buf_r[..mix_frames],
                    mix_frames,
                );
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

/// Mix audio from all active clips into the output buffer.
/// This runs on the cpal audio callback thread -- must be allocation-free
/// (uses pre-allocated track_buf_l/track_buf_r).
pub(crate) fn mix_audio(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    tracks: &parking_lot::RwLock<IndexMap<TrackId, Track>>,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    plugins: &parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>,
    tempo_map: &parking_lot::RwLock<TempoMap>,
    sample_rate: u32,
    track_buf_l: &mut Vec<f32>,
    track_buf_r: &mut Vec<f32>,
    monitor_cons: &mut ringbuf::HeapCons<f32>,
    monitor_temp: &mut Vec<f32>,
    buf_frames: usize,
    quantum: usize,
) {
    // Zero the output buffer
    data.fill(0.0);

    let output_frames = data.len() / channels;
    let frames = output_frames.min(buf_frames);

    // Read monitor input with jitter margin to avoid underflows.
    // Skip stale monitor data to keep latency at ~1 buffer period.
    let needed = frames * 2; // stereo samples
    let available = monitor_cons.occupied_len();
    if available > needed + quantum * 2 {
        monitor_cons.skip(available - needed);
    }
    let to_read = needed.min(monitor_cons.occupied_len());
    let monitor_samples = monitor_cons.pop_slice(&mut monitor_temp[..to_read]);
    let monitor_frames = monitor_samples / 2;

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
                        track_buf_l,
                        track_buf_r,
                        &plugins_guard,
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

    let (Some(tracks_guard), Some(clips_guard), Some(plugins_guard)) =
        (tracks.try_read(), clips.try_read(), plugins.try_read())
    else {
        // Lock contended -- advance playhead to avoid desync, output silence this buffer
        let new_playhead = playhead + output_frames as u64;
        shared.playhead.store(new_playhead, Ordering::Relaxed);
        return;
    };

    // Per-track processing: (clips + monitor input) -> plugins -> volume -> master
    let any_solo = tracks_guard.values().any(|t| t.soloed());
    let mut monitor_consumed = false;
    for track in tracks_guard.values() {
        if track.muted() {
            continue;
        }
        if any_solo && !track.soloed() {
            continue;
        }

        // Zero per-track buffers
        track_buf_l[..frames].fill(0.0);
        track_buf_r[..frames].fill(0.0);

        // Mix monitor input for tracks with monitoring enabled (first monitoring track only)
        let mut has_audio = false;
        if track.monitor_enabled() && monitor_frames > 0 && !monitor_consumed {
            monitor_consumed = true;
            let is_mono = track.mono();
            let mix_frames = frames.min(monitor_frames);
            for f in 0..mix_frames {
                let l = monitor_temp[f * 2];
                if is_mono {
                    track_buf_l[f] += l;
                    track_buf_r[f] += l;
                } else {
                    track_buf_l[f] += l;
                    track_buf_r[f] += monitor_temp[f * 2 + 1];
                }
            }
            has_audio = true;
        }

        // Accumulate all clips for this track into de-interleaved track buffers
        for clip in clips_guard.iter() {
            if clip.track_id != track.id {
                continue;
            }

            let clip_frames = clip.duration_frames();
            // Compute overlap between this clip and the current output buffer
            let clip_start = clip.start_sample;
            let clip_end = clip_start + clip_frames;
            let buf_start = playhead;
            let buf_end = playhead + frames as u64;

            if buf_end <= clip_start || buf_start >= clip_end {
                continue; // No overlap
            }

            let overlap_start = buf_start.max(clip_start);
            let overlap_end = buf_end.min(clip_end);

            for timeline_frame in overlap_start..overlap_end {
                let frame_offset = (timeline_frame - buf_start) as usize;
                let clip_frame =
                    (timeline_frame - clip_start) as usize + clip.trim_start_frames as usize;
                let clip_idx = clip_frame * 2;
                if clip_idx + 1 < clip.data.len() {
                    track_buf_l[frame_offset] += clip.data[clip_idx];
                    track_buf_r[frame_offset] += clip.data[clip_idx + 1];
                    has_audio = true;
                }
            }
        }

        // Process through plugin chain (even if no audio -- plugins may generate tails)
        if !track.plugin_ids.is_empty() {
            for &plugin_id in &track.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    if let Some(mut inst) = mutex.try_lock() {
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

        if !has_audio {
            continue;
        }

        // Apply track volume + pan and sum to master output
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

        sum_to_output(data, channels, frames, track_buf_l, track_buf_r, gain_l, gain_r);
    }

    drop(plugins_guard);

    // Metronome click synthesis
    if let Some(tm) = tempo_map.try_read() {
        if tm.metronome_enabled {
            let spb = tm.samples_per_beat(sample_rate);
            let numerator = tm.numerator as u64;
            let click_duration_samples = (sample_rate as f32 * CLICK_DURATION_SECS) as u64;

            for frame_offset in 0..output_frames {
                let timeline_frame = playhead + frame_offset as u64;
                // Use round() to avoid drift: find the nearest beat boundary
                let beat_index = (timeline_frame as f64 / spb).floor();
                let beat_start = (beat_index * spb).round() as u64;
                let beat_pos = timeline_frame.saturating_sub(beat_start);

                if beat_pos < click_duration_samples {
                    let t = beat_pos as f32 / sample_rate as f32;
                    let beat_in_bar = (beat_index as u64) % numerator;
                    let freq = if beat_in_bar == 0 { CLICK_FREQ_DOWNBEAT } else { CLICK_FREQ_UPBEAT };
                    let amplitude = CLICK_AMPLITUDE * (-t * CLICK_DECAY_RATE).exp();
                    let click = amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();

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
    }

    // Apply master volume, hard clip, and compute master peak levels
    apply_master_volume_and_peaks(data, channels, shared);

    // Advance playhead
    let new_playhead = playhead + output_frames as u64;
    shared.playhead.store(new_playhead, Ordering::Relaxed);
}
