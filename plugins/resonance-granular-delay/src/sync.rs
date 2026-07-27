//! Tempo-sync resolution for the delay time, following the
//! `resonance-delay` division-table convention.

use resonance_plugin::TempoInfo;

pub const DIVISION_LABELS: &[&str] = &[
    "1/1", "1/2", "1/2D", "1/2T", "1/4", "1/4D", "1/4T", "1/8", "1/8D", "1/8T", "1/16", "1/16T",
];

const DIVISION_BEATS: &[f32] = &[
    4.0,       // 1/1
    2.0,       // 1/2
    3.0,       // 1/2D  (dotted)
    4.0 / 3.0, // 1/2T  (triplet)
    1.0,       // 1/4
    1.5,       // 1/4D
    2.0 / 3.0, // 1/4T
    0.5,       // 1/8
    0.75,      // 1/8D
    1.0 / 3.0, // 1/8T
    0.25,      // 1/16
    1.0 / 6.0, // 1/16T
];

/// Resolve sync/division/time/tempo into the nominal grain read position
/// in *seconds* behind the write head. Free-running (or sync without
/// host transport) falls back to `time_ms`.
pub fn delay_seconds(
    sync: bool,
    division: usize,
    time_ms: f32,
    tempo: Option<TempoInfo>,
    max_delay_seconds: f32,
) -> f32 {
    let raw = if sync {
        if let Some(t) = tempo {
            let bpm = t.bpm.max(20.0);
            let seconds_per_beat = 60.0 / bpm;
            let div = division.min(DIVISION_BEATS.len() - 1);
            seconds_per_beat * DIVISION_BEATS[div]
        } else {
            time_ms * 0.001
        }
    } else {
        time_ms * 0.001
    };
    raw.clamp(0.001, max_delay_seconds)
}
