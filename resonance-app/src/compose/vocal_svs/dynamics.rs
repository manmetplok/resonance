//! The dynamics gain envelope: a per-sample gain applied to the rendered
//! mono take after the vocoder (vocal-mcp-control-and-quality.md §9).
//!
//! The envelope is built from [`GainSpan`]s — stretches of the take held at
//! one gain in dB, 0 dB everywhere else — and every step between spans is
//! smoothed by a centred moving average [`DYNAMICS_SMOOTH_SEC`] wide, so a
//! change of gain at a note boundary is a 30 ms ramp rather than a click.
//!
//! Note velocity feeds it today ([`velocity_gain_spans`]): each note gets
//! up to ±[`VELOCITY_GAIN_MAX_DB`] at `velocity_dynamics` 1, on the same
//! `(vel − 0.78) / 0.22` shape the tension curve uses. This is what makes a
//! bank without an energy input (TIGER) follow velocity at all. The
//! function takes spans rather than notes so a per-frame source (the
//! expression Dynamics curve on a bank without energy) can feed it too:
//! one span per frame, smoothed the same way.

use resonance_audio::types::MidiNote;

/// One stretch of the take held at `gain_db`. Times are seconds from the
/// start of the buffer the envelope is applied to; `end_sec` may be
/// `f64::INFINITY` for "to the end".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainSpan {
    pub start_sec: f64,
    pub end_sec: f64,
    pub gain_db: f32,
}

/// Width of the ramp between two spans' gains.
pub const DYNAMICS_SMOOTH_SEC: f64 = 0.030;

/// Gain of a note at full velocity (or its negative at the softest) with
/// `velocity_dynamics` 1.
pub const VELOCITY_GAIN_MAX_DB: f32 = 6.0;

/// The velocity both dynamics paths treat as neutral: `derive_vocal`'s
/// ordinary syllable. The tension curve shares it.
pub const NEUTRAL_VELOCITY: f32 = 0.78;

/// `velocity` mapped to `-1..=1` around [`NEUTRAL_VELOCITY`]: 1 at
/// velocity 1, -1 at 0.56 and below.
pub fn velocity_modulation(velocity: f32) -> f32 {
    ((velocity - NEUTRAL_VELOCITY) / (1.0 - NEUTRAL_VELOCITY)).clamp(-1.0, 1.0)
}

/// A note's post-vocoder gain in dB at `amount` (`velocity_dynamics`).
pub fn velocity_gain_db(velocity: f32, amount: f32) -> f32 {
    amount.clamp(0.0, 1.0) * VELOCITY_GAIN_MAX_DB * velocity_modulation(velocity)
}

/// One span per note: from its onset to the next note's onset (the first
/// from the start of the buffer, the last to its end), at
/// [`velocity_gain_db`]. `lead_sec` is where the first note's onset sits
/// in the buffer (the leading silence pad). Onsets are relative to the
/// first note, at `bpm`.
pub fn velocity_gain_spans(
    notes: &[MidiNote],
    ticks_per_quarter: u32,
    bpm: f32,
    amount: f32,
    lead_sec: f64,
) -> Vec<GainSpan> {
    velocity_gain_spans_with_breaths(notes, &[], ticks_per_quarter, bpm, amount, lead_sec)
}

/// [`velocity_gain_spans`] where note `i`'s span starts
/// `breath_lead_sec[i]` before its onset: a written breath in front of a
/// note is sung at the end of the previous note's slot, and takes the
/// gain of the note it leads into, not the one it follows. A missing or
/// zero entry is no breath; a lead never reaches back past the previous
/// note's onset.
pub fn velocity_gain_spans_with_breaths(
    notes: &[MidiNote],
    breath_lead_sec: &[f64],
    ticks_per_quarter: u32,
    bpm: f32,
    amount: f32,
    lead_sec: f64,
) -> Vec<GainSpan> {
    let Some(first) = notes.first() else {
        return Vec::new();
    };
    let spt = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter.max(1) as f64);
    let onset = |n: &MidiNote| lead_sec + n.start_tick.saturating_sub(first.start_tick) as f64 * spt;
    // Where note `i`'s span starts: its onset, pulled back over its breath.
    let start = |i: usize| -> f64 {
        let at = onset(&notes[i]);
        let breath = breath_lead_sec.get(i).copied().unwrap_or(0.0).max(0.0);
        (at - breath).max(onset(&notes[i - 1]))
    };
    (0..notes.len())
        .map(|i| GainSpan {
            start_sec: if i == 0 { 0.0 } else { start(i) },
            end_sec: if i + 1 < notes.len() {
                start(i + 1)
            } else {
                f64::INFINITY
            },
            gain_db: velocity_gain_db(notes[i].velocity, amount),
        })
        .collect()
}

/// The per-sample linear gain for a buffer of `len` samples at
/// `sample_rate`: each span's gain over its stretch, 0 dB elsewhere, the
/// dB steps smoothed by a centred moving average `smooth_sec` wide. Later
/// spans win where spans overlap. `None` when every span is 0 dB — the
/// take is left bit-identical.
pub fn dynamics_gain_envelope(
    spans: &[GainSpan],
    len: usize,
    sample_rate: u32,
    smooth_sec: f64,
) -> Option<Vec<f32>> {
    if len == 0 || spans.iter().all(|s| s.gain_db == 0.0) {
        return None;
    }
    let sr = sample_rate as f64;
    let index = |t: f64| -> usize {
        if t.is_infinite() {
            len
        } else {
            ((t * sr).round().max(0.0) as usize).min(len)
        }
    };
    let mut db = vec![0.0f64; len];
    for s in spans {
        let (a, b) = (index(s.start_sec), index(s.end_sec));
        for v in &mut db[a..b.max(a)] {
            *v = s.gain_db as f64;
        }
    }
    // Centred moving average over the dB steps: a step becomes a linear
    // ramp `smooth_sec` wide. Edges average over the part of the window
    // that exists.
    let half = ((smooth_sec * sr) / 2.0).round() as usize;
    let smoothed = if half == 0 {
        db
    } else {
        let mut prefix = Vec::with_capacity(len + 1);
        prefix.push(0.0f64);
        let mut acc = 0.0;
        for v in &db {
            acc += v;
            prefix.push(acc);
        }
        (0..len)
            .map(|i| {
                let lo = i.saturating_sub(half);
                let hi = (i + half + 1).min(len);
                (prefix[hi] - prefix[lo]) / (hi - lo) as f64
            })
            .collect()
    };
    Some(
        smoothed
            .into_iter()
            .map(|d| 10f64.powf(d / 20.0) as f32)
            .collect(),
    )
}

/// Apply [`dynamics_gain_envelope`] to `mono` in place. Returns whether
/// anything changed (all-0 dB spans leave the buffer untouched).
pub fn apply_dynamics_gain(mono: &mut [f32], spans: &[GainSpan], sample_rate: u32) -> bool {
    let Some(env) = dynamics_gain_envelope(spans, mono.len(), sample_rate, DYNAMICS_SMOOTH_SEC)
    else {
        return false;
    };
    for (s, g) in mono.iter_mut().zip(env) {
        *s *= g;
    }
    true
}
