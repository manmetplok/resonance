//! Snap-to-grid helpers used by the timeline canvas, the clip-drag
//! handlers, and the transport seek path. Pure functions: take the
//! current zoom + tempo map, return the snapped sample position.
//!
//! Two zoom-driven snap resolutions:
//! - bar pixel width >= 40 → snap to beats
//! - bar pixel width >= 20 → snap to bars
//! - lower zoom → snap to multi-bar increments

use resonance_audio::types::{bar_len_quarters, bpm_at_bar, TempoMap};

/// Snap a sample position to the nearest bar or beat boundary of the
/// project's `TempoMap` (the grid the ruler draws). At high zoom (bar
/// wider than 40 px) snaps to beats; lower zoom snaps to bars.
pub fn snap_sample_to_grid_tempo(
    sample: u64,
    bpm: f32,
    time_sig_num: u8,
    sample_rate: u32,
    zoom: f32,
    tempo_map: &TempoMap,
) -> u64 {
    if bpm <= 0.0 || time_sig_num == 0 || zoom <= 0.0 {
        return sample;
    }
    // A single tempo *and* a single signature make a uniform grid: use
    // the flat path. Any signature change needs the map's bar table —
    // the transport numerator follows the playhead, so it can't stand in
    // for the meter at `sample` (review FU-V1a). The map's own points win
    // over the transport values, which are only the fallback for an
    // empty map.
    if tempo_map.tempo_points.len() <= 1 && tempo_map.signature_points.len() <= 1 {
        let bpm = tempo_map.tempo_points.first().map_or(bpm, |p| p.bpm);
        let (num, den) = tempo_map
            .signature_points
            .first()
            .map_or((time_sig_num, tempo_map.denominator_at_bar(0)), |p| {
                (p.numerator, p.denominator)
            });
        if bpm <= 0.0 || num == 0 {
            return sample;
        }
        // BPM counts quarter notes, so the bar's length comes from the
        // signature's length in quarters, not its numerator; the beat
        // step then follows from the bar (ba todo #1389).
        let samples_per_quarter = sample_rate as f64 * 60.0 / bpm as f64;
        let samples_per_bar = samples_per_quarter * bar_len_quarters(num, den);
        let samples_per_beat = samples_per_bar / num as f64;
        let bar_pixel_width = (samples_per_bar / sample_rate as f64) as f32 * zoom;
        let step = if bar_pixel_width >= 40.0 {
            samples_per_beat
        } else if bar_pixel_width >= 20.0 {
            samples_per_bar
        } else {
            samples_per_bar * (20.0 / bar_pixel_width).ceil() as f64
        };
        if step <= 0.0 {
            return sample;
        }
        return ((sample as f64 / step).round() * step).round() as u64;
    }

    // Tempo map is active: find which bar we're in and snap to the
    // nearest bar or beat boundary.
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);

    // Determine snap resolution from the local bar pixel width.
    let local_bpm = bpm_at_bar(bar as f64, &tempo_map.tempo_points);
    let cur_num = tempo_map.numerator_at_bar(bar);
    let spq = sample_rate as f64 * 60.0 / local_bpm;
    let bar_samples = spq * tempo_map.bar_len_quarters_at(bar);
    let bar_px = (bar_samples / sample_rate as f64) as f32 * zoom;

    let snap_to_beats = bar_px >= 40.0;

    if snap_to_beats {
        // Snap to the nearest beat within the bar.
        let beat_frac = frac * cur_num as f64;
        let nearest_beat = beat_frac.round() as u32;
        if nearest_beat >= cur_num as u32 {
            // Snaps to start of next bar
            tempo_map.bar_to_sample(bar + 1)
        } else {
            // Snaps to beat within this bar
            let bar_start = tempo_map.bar_to_sample(bar);
            let bar_end = tempo_map.bar_to_sample(bar + 1);
            let total = (bar_end - bar_start) as f64;
            let beat_frac_pos = nearest_beat as f64 / cur_num as f64;
            bar_start + (beat_frac_pos * total) as u64
        }
    } else {
        // Snap to the nearest bar.
        let nearest_bar = if frac >= 0.5 { bar + 1 } else { bar };
        tempo_map.bar_to_sample(nearest_bar)
    }
}
