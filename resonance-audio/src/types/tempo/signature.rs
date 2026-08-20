//! Time-signature geometry — the one definition of how long a bar is.
//!
//! A bar in `numerator`/`denominator` holds `numerator` notes of value
//! `1/denominator`, so its length depends on *both* numbers: a 7/8 bar is
//! seven eighth-notes, not seven quarter-notes. Everything that needs a
//! bar length — the tempo map's bar table, the quantize grid ruler, MIDI
//! export, the timeline and Compose rulers — goes through the functions
//! here, so the answer cannot drift apart between them again
//! (ba todo #1389, where the bar table said 7/4 and the quantiser said
//! 7/8 and the two never met in a test).
//!
//! Tick space is quarter-note-based ([`TICKS_PER_QUARTER_NOTE`]) and so is
//! BPM, so [`bar_len_quarters`] is the bridge to sample space: a bar lasts
//! `samples_per_quarter * bar_len_quarters(..)` samples.

use super::TICKS_PER_QUARTER_NOTE;

/// Ticks in a whole note (four quarter notes).
pub const TICKS_PER_WHOLE_NOTE: u64 = 4 * TICKS_PER_QUARTER_NOTE;

/// Length of one beat — a single note of value `1/denominator` — in ticks.
///
/// Always >= 1 so callers can divide by it. A denominator of 0 is a
/// corrupt signature and degrades to a whole note.
///
/// Real signatures use power-of-two denominators, which divide
/// [`TICKS_PER_WHOLE_NOTE`] exactly. A denominator that does not (7, 9, …)
/// is rounded down here so that a bar stays an exact whole number of
/// beats — grid steps land on the beat rather than accumulating drift.
pub fn beat_len_ticks(denominator: u8) -> u64 {
    if denominator == 0 {
        return TICKS_PER_WHOLE_NOTE;
    }
    (TICKS_PER_WHOLE_NOTE / denominator as u64).max(1)
}

/// Length of a `numerator`/`denominator` bar in ticks: `numerator` beats
/// of [`beat_len_ticks`].
///
/// A degenerate signature (either side 0) falls back to one whole note.
pub fn bar_len_ticks(numerator: u8, denominator: u8) -> u64 {
    if numerator == 0 || denominator == 0 {
        return TICKS_PER_WHOLE_NOTE; // degenerate signature → one whole note
    }
    (numerator as u64 * beat_len_ticks(denominator)).max(1)
}

/// Length of a `numerator`/`denominator` bar expressed in quarter notes —
/// the unit BPM counts in. Multiply by samples-per-quarter for the bar's
/// length in samples. 4/4 gives 4.0; 7/8 gives 3.5.
pub fn bar_len_quarters(numerator: u8, denominator: u8) -> f64 {
    bar_len_ticks(numerator, denominator) as f64 / TICKS_PER_QUARTER_NOTE as f64
}

/// Convert a tick length to quarter notes — the sample-space bridge for a
/// bar whose tick length is already known (a `BarEntry`'s `ticks_in_bar`).
pub fn ticks_to_quarters(ticks: u64) -> f64 {
    ticks as f64 / TICKS_PER_QUARTER_NOTE as f64
}
