//! Metronome click synthesis. Two modes:
//! - count-in clicks ([`render_count_in_clicks`]): aligned to a
//!   count-in-local elapsed counter so the last click lands exactly
//!   one beat before the punch-in line.
//! - timeline clicks ([`render_metronome_clicks`]): aligned to the
//!   tempo map's bar/beat table (or flat BPM as a fallback) and
//!   layered onto the rendered output buffer post-master-FX,
//!   pre-master-volume.
//!
//! Both modes share the same envelope: a short sine burst with
//! exponential decay, accented on downbeats.

const CLICK_DURATION_SECS: f32 = 0.02;
const CLICK_FREQ_DOWNBEAT: f32 = 1500.0;
const CLICK_FREQ_UPBEAT: f32 = 1000.0;
const CLICK_AMPLITUDE: f32 = 0.3;
const CLICK_DECAY_RATE: f32 = 200.0;

use crate::limits::MAX_METRONOME_BEATS_PER_BUFFER;
use crate::types::TempoMap;

/// One sine-burst click sample at envelope position `t` (seconds since
/// click-onset). `is_downbeat` selects the higher-pitched accent tone.
#[inline]
fn click_sample(t: f32, is_downbeat: bool) -> f32 {
    let freq = if is_downbeat {
        CLICK_FREQ_DOWNBEAT
    } else {
        CLICK_FREQ_UPBEAT
    };
    let amplitude = CLICK_AMPLITUDE * (-t * CLICK_DECAY_RATE).exp();
    amplitude * (2.0 * std::f32::consts::PI * freq * t).sin()
}

/// Render count-in metronome clicks into the start of `data`.
///
/// `elapsed_at_start` is the count-in-local frame index of the first
/// frame in this buffer. `click_frames` is how many of the buffer's
/// frames remain inside the count-in window (the count-in stops once
/// the remaining-frames counter hits zero, even if more output frames
/// follow).
pub(super) fn render_count_in_clicks(
    data: &mut [f32],
    channels: usize,
    sample_rate: u32,
    tempo_map: &TempoMap,
    elapsed_at_start: u64,
    click_frames: usize,
) {
    let spb = tempo_map.samples_per_beat(sample_rate);
    let numerator = tempo_map.numerator as u64;
    let click_duration_samples = (sample_rate as f32 * CLICK_DURATION_SECS) as u64;
    for frame_offset in 0..click_frames {
        let elapsed = elapsed_at_start + frame_offset as u64;
        let beat_index = (elapsed as f64 / spb).floor();
        let beat_start = (beat_index * spb).round() as u64;
        let beat_pos = elapsed.saturating_sub(beat_start);
        if beat_pos < click_duration_samples {
            let t = beat_pos as f32 / sample_rate as f32;
            let beat_in_bar = (beat_index as u64) % numerator;
            let click = click_sample(t, beat_in_bar == 0);
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

/// Layer metronome clicks onto a fully-rendered output buffer. Walks
/// the bar/beat table (or, if the table is empty, the flat-BPM grid)
/// to gather every beat boundary that overlaps the buffer's timeline
/// ranges, then renders the click envelope at each.
///
/// `seam_split` carries the loop-seam information so the function can
/// map output-frame indices to two distinct timeline ranges (pre-wrap
/// and post-wrap). When `seam_split` is `None`, the buffer maps
/// linearly from `playhead`.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_metronome_clicks(
    data: &mut [f32],
    channels: usize,
    sample_rate: u32,
    tempo_map: &TempoMap,
    flat_bpm: f64,
    flat_numerator: u16,
    output_frames: usize,
    playhead: u64,
    seam_split: Option<(usize, usize, u64)>,
) {
    let click_duration_samples = (sample_rate as f32 * CLICK_DURATION_SECS) as u64;

    // Collect beat boundaries near the buffer into a small stack
    // array: (beat_sample, is_downbeat). At most ~8 beats can
    // overlap one audio buffer even at extreme tempos.
    let mut beats = [(0u64, false); MAX_METRONOME_BEATS_PER_BUFFER];
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
            ranges = [(playhead, playhead + output_frames as u64), (0, 0)];
            n_ranges = 1;
        }
    }

    if tempo_map.bar_count() > 0 {
        for &(r_start, r_end) in ranges.iter().take(n_ranges) {
            let search_start = r_start.saturating_sub(click_duration_samples);
            let Some(start_bar) = tempo_map.bar_index_at(search_start) else {
                continue;
            };
            let mut bar_idx = start_bar;
            'bar: loop {
                let num_beats = tempo_map.beats_in_bar(bar_idx);
                for beat in 0..num_beats {
                    let Some(bs) = tempo_map.beat_sample_in_bar(bar_idx, beat, sample_rate) else {
                        break 'bar;
                    };
                    if bs >= r_end {
                        break 'bar;
                    }
                    if bs + click_duration_samples > r_start
                        && n_beats < MAX_METRONOME_BEATS_PER_BUFFER
                    {
                        beats[n_beats] = (bs, beat == 0);
                        n_beats += 1;
                    }
                }
                bar_idx += 1;
                if bar_idx >= tempo_map.bar_count() {
                    break;
                }
            }
        }
    } else {
        // No bar table — flat BPM beat positions.
        let spb = sample_rate as f64 * 60.0 / flat_bpm;
        let numerator = flat_numerator as u64;
        for &(r_start, r_end) in ranges.iter().take(n_ranges) {
            let search_start = r_start.saturating_sub(click_duration_samples);
            let first_beat = (search_start as f64 / spb).floor() as u64;
            let mut bi = first_beat;
            loop {
                let bs = (bi as f64 * spb).round() as u64;
                if bs >= r_end {
                    break;
                }
                if bs + click_duration_samples > r_start
                    && n_beats < MAX_METRONOME_BEATS_PER_BUFFER
                {
                    beats[n_beats] = (bs, bi.is_multiple_of(numerator));
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
        for &(beat_sample, is_downbeat) in beats.iter().take(n_beats) {
            let beat_pos = timeline_frame.saturating_sub(beat_sample);
            if beat_pos < click_duration_samples && timeline_frame >= beat_sample {
                let t = beat_pos as f32 / sample_rate as f32;
                let click = click_sample(t, is_downbeat);
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
