//! Tempo sync for pre-delay and decay (reverb-algorithms.md §4.5).
//!
//! Both are pure functions of the choice and the host's tempo: `None`
//! means "not synced" (the choice is `Off`, or the host gave no usable
//! tempo), and the caller falls back to the ms/s knob.
//!
//! The note values are musical: a beat is a quarter note, so `1/16` is a
//! quarter of a beat (125 ms at 120 BPM) and `1/64` a sixteenth of one.
//! That puts the vocal pre-delay range (20–40 ms) on `1/64` at common
//! tempos, the convention the skills' tables use.

use resonance_plugin::TempoInfo;

use crate::dsp::MAX_PREDELAY_SECONDS;

/// Labels of `predelay_sync`.
pub const PREDELAY_SYNC_LABELS: &[&str] = &["Off", "1/128", "1/64", "1/32", "1/16", "1/8"];

/// Labels of `decay_sync`.
pub const DECAY_SYNC_LABELS: &[&str] = &["Off", "1/4", "1/2", "1 bar", "2 bars", "4 bars"];

/// Length of each `predelay_sync` choice in beats (index 0, `Off`, is
/// unused).
const PREDELAY_BEATS: [f32; 6] = [0.0, 4.0 / 128.0, 4.0 / 64.0, 4.0 / 32.0, 4.0 / 16.0, 4.0 / 8.0];

/// The decay range the `decay` parameter allows; a synced T60 is held
/// inside it.
const DECAY_MIN_S: f32 = 0.1;
const DECAY_MAX_S: f32 = 30.0;

/// The host's tempo, if it gave one that means something.
fn bpm(tempo: Option<TempoInfo>) -> Option<f32> {
    tempo
        .map(|t| t.bpm)
        .filter(|b| b.is_finite() && *b > 0.0)
}

/// The synced pre-delay in ms for choice `index` at the host tempo, held
/// at the longest pre-delay the DSP has room for (a `1/8` below 30 BPM
/// would exceed it), so the editor shows the value actually in effect.
pub fn predelay_ms(index: i32, tempo: Option<TempoInfo>) -> Option<f32> {
    let beats = *PREDELAY_BEATS.get(usize::try_from(index).ok()?)?;
    if beats <= 0.0 {
        return None;
    }
    Some((60_000.0 / bpm(tempo)? * beats).min(MAX_PREDELAY_SECONDS * 1000.0))
}

/// The synced decay (T60) in seconds for choice `index` at the host
/// tempo: `1/4` is one beat, `1/2` two, a bar is the host meter's length.
pub fn decay_s(index: i32, tempo: Option<TempoInfo>) -> Option<f32> {
    let bpm = bpm(tempo)?;
    let bar = tempo?.beats_per_bar();
    let beats = match index {
        1 => 1.0,
        2 => 2.0,
        3 => bar,
        4 => 2.0 * bar,
        5 => 4.0 * bar,
        _ => return None,
    };
    Some((beats * 60.0 / bpm).clamp(DECAY_MIN_S, DECAY_MAX_S))
}
