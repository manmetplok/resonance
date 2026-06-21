//! Tick-space grid geometry: note-value → tick conversion and a
//! time-signature-aware bar ruler used to anchor grid lines.
//!
//! Everything here works purely in MIDI ticks derived from
//! [`TICKS_PER_QUARTER_NOTE`] and the tempo map's signature track, so it
//! is independent of sample rate and the (sample-keyed) bar table.

use crate::types::{SignaturePoint, TempoMap, TICKS_PER_QUARTER_NOTE};

/// Ticks in a whole note (4 quarter notes).
const TICKS_PER_WHOLE: u64 = 4 * TICKS_PER_QUARTER_NOTE;

/// The base note value of a quantize grid (the denominator side of the
/// musical fraction, e.g. `Eighth` == 1/8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridValue {
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
}

impl GridValue {
    /// Straight (un-modified) length of this value in ticks.
    fn base_ticks(self) -> u64 {
        match self {
            GridValue::Quarter => TICKS_PER_WHOLE / 4,
            GridValue::Eighth => TICKS_PER_WHOLE / 8,
            GridValue::Sixteenth => TICKS_PER_WHOLE / 16,
            GridValue::ThirtySecond => TICKS_PER_WHOLE / 32,
        }
    }
}

/// A grid modifier applied to a [`GridValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridModifier {
    /// Plain value.
    Straight,
    /// Triplet: two thirds of the straight length (three in the space of two).
    Triplet,
    /// Dotted: one and a half times the straight length.
    Dotted,
}

/// A musical grid division (note value + modifier), e.g. dotted eighth
/// or sixteenth-note triplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Division {
    pub value: GridValue,
    pub modifier: GridModifier,
}

impl Division {
    /// A straight division of the given value.
    pub fn straight(value: GridValue) -> Self {
        Division {
            value,
            modifier: GridModifier::Straight,
        }
    }

    /// A triplet division of the given value.
    pub fn triplet(value: GridValue) -> Self {
        Division {
            value,
            modifier: GridModifier::Triplet,
        }
    }

    /// A dotted division of the given value.
    pub fn dotted(value: GridValue) -> Self {
        Division {
            value,
            modifier: GridModifier::Dotted,
        }
    }

    /// Length of one grid step in ticks. Always >= 1.
    pub fn ticks(self) -> u64 {
        let base = self.value.base_ticks();
        let t = match self.modifier {
            GridModifier::Straight => base,
            // 2/3 of base; base is always divisible by 3 for the values
            // we support (160, 80, 40, ... and 320 for quarter).
            GridModifier::Triplet => base * 2 / 3,
            GridModifier::Dotted => base * 3 / 2,
        };
        t.max(1)
    }
}

/// Length of a bar (in ticks) for a `numerator/denominator` time
/// signature. A bar holds `numerator` notes of value `1/denominator`.
fn bar_len_ticks(numerator: u8, denominator: u8) -> u64 {
    if numerator == 0 || denominator == 0 {
        return TICKS_PER_WHOLE; // degenerate signature → one whole note
    }
    (numerator as u64 * TICKS_PER_WHOLE / denominator as u64).max(1)
}

/// One run of consecutive bars sharing a time signature.
#[derive(Debug, Clone, Copy)]
struct Segment {
    start_tick: u64,
    bar_len: u64,
}

/// A purely tick-based, time-signature-aware map of bar boundaries.
///
/// Built from a [`TempoMap`]'s signature track (tempo/BPM is irrelevant
/// in tick space). Used to anchor quantize grid lines so odd meters and
/// mid-project signature changes are honoured.
#[derive(Debug, Clone)]
pub struct BarRuler {
    /// Segments in ascending `start_tick` order. The last segment
    /// extends to infinity.
    segments: Vec<Segment>,
}

impl BarRuler {
    /// Build a ruler from a tempo map's signature points.
    pub fn new(tempo: &TempoMap) -> Self {
        // Signature points are expected sorted by bar; sort defensively
        // without mutating the caller's data.
        let mut sigs: Vec<&SignaturePoint> = tempo.signature_points.iter().collect();
        sigs.sort_by_key(|s| s.bar);

        let (mut cur_num, mut cur_den, mut idx) = if sigs.first().map(|s| s.bar) == Some(0) {
            (sigs[0].numerator, sigs[0].denominator, 1)
        } else {
            (tempo.numerator, tempo.denominator, 0)
        };

        let mut segments = Vec::new();
        let mut cur_bar: u32 = 0;
        let mut cur_tick: u64 = 0;
        loop {
            let bar_len = bar_len_ticks(cur_num, cur_den);
            segments.push(Segment {
                start_tick: cur_tick,
                bar_len,
            });
            match sigs.get(idx) {
                Some(next) => {
                    let bars = (next.bar.saturating_sub(cur_bar)) as u64;
                    cur_tick += bars * bar_len;
                    cur_bar = next.bar;
                    cur_num = next.numerator;
                    cur_den = next.denominator;
                    idx += 1;
                }
                None => break,
            }
        }

        BarRuler { segments }
    }

    /// Return `(bar_start_tick, bar_len_ticks)` for the bar containing
    /// `abs_tick`.
    pub fn bar_at(&self, abs_tick: u64) -> (u64, u64) {
        // Last segment whose start_tick <= abs_tick.
        let seg = self
            .segments
            .iter()
            .rev()
            .find(|s| s.start_tick <= abs_tick)
            .copied()
            .unwrap_or(self.segments[0]);
        let local = abs_tick - seg.start_tick;
        let bar_in_seg = local / seg.bar_len;
        let bar_start = seg.start_tick + bar_in_seg * seg.bar_len;
        (bar_start, seg.bar_len)
    }
}

/// Swing delay (ticks) applied to odd grid steps for a step size `g`.
///
/// `swing` in `0.0..=1.0`. At `swing == 2/3` the delay is `g/3`, i.e. the
/// off-beat lands exactly on the triplet — the classic swing feel; at
/// `swing == 1.0` the off-beat sits three-quarters of the way through the
/// beat pair.
fn swing_delay(g: u64, swing: f32) -> u64 {
    let s = swing.clamp(0.0, 1.0) as f64;
    (s * g as f64 / 2.0).round() as u64
}

/// Snap `abs_tick` to the nearest (optionally swung) grid line for the
/// given step size `g`, anchored to the bar containing it.
pub fn snap_to_grid(abs_tick: u64, ruler: &BarRuler, g: u64, swing: f32) -> u64 {
    let (bar_start, bar_len) = ruler.bar_at(abs_tick);
    let local = abs_tick - bar_start;
    let delay = swing_delay(g, swing);

    // Position of grid step `k` within the bar, swung on odd steps and
    // never past the bar's end (the next downbeat is itself a grid line).
    let step_pos = |k: u64| -> u64 {
        let mut pos = k * g;
        if k % 2 == 1 {
            pos += delay;
        }
        pos.min(bar_len)
    };

    let k_floor = local / g;
    let lo = step_pos(k_floor);
    let hi = step_pos(k_floor + 1);
    let dist = |a: u64, b: u64| -> u64 { a.max(b) - a.min(b) };
    let pick = if dist(local, lo) <= dist(local, hi) {
        lo
    } else {
        hi
    };
    bar_start + pick
}
