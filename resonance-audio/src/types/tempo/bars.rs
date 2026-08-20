//! Bar / beat / subdivision math (time-signature aware) on `TempoMap`.

use super::conversion::{sample_frac_to_tick_frac, tick_frac_to_sample_frac};
use super::map::TempoMap;
use super::signature::{bar_len_quarters, bar_len_ticks, beat_len_ticks, ticks_to_quarters};
use super::TICKS_PER_QUARTER_NOTE;

/// Tolerance used when snapping a derived beat position back onto the beat
/// grid, expressed in **samples**.
///
/// A bar start is never an exact real number of samples: `rebuild_bar_table`
/// rounds each entry to the nearest sample (±0.5), `bar_to_sample`'s
/// past-the-horizon extrapolation *truncates* (up to −1), and a caller
/// deriving a beat inside that bar rounds once more (±0.5). So a position
/// that genuinely is a downbeat can arrive up to ~2 samples short of the
/// arithmetic boundary, and flooring it lands in the previous bar with a beat
/// fraction of 0.9999 — the bug this constant exists to fix.
///
/// Two samples is 42 µs at 48 kHz, orders of magnitude below anything
/// audible or musically meaningful (a 1 ms offset is already 48 samples), so
/// the snap cannot quantise a genuinely off-grid position.
const SNAP_TOLERANCE_SAMPLES: f64 = 2.0;

/// Collapse floating-point residue onto the whole-beat grid.
///
/// `eps` is [`SNAP_TOLERANCE_SAMPLES`] converted to beats by the caller,
/// which is the only place that knows the local samples-per-beat. Snapping to
/// the *nearest* whole beat keeps the correction symmetric — a position a
/// hair before and a hair after a boundary both resolve to it — while a
/// genuinely off-grid position keeps its true fraction (half a beat in still
/// reads 0.5).
fn snap_to_beat_grid(beats: f64, eps: f64) -> f64 {
    if !beats.is_finite() || !(eps > 0.0) {
        return beats;
    }
    let nearest = beats.round();
    if (beats - nearest).abs() <= eps {
        nearest
    } else {
        beats
    }
}

impl TempoMap {
    /// Number of bars in the precomputed bar table.
    pub fn bar_count(&self) -> usize {
        self.bar_table.len()
    }

    /// Find the bar table index containing the given sample position.
    pub fn bar_index_at(&self, sample_pos: u64) -> Option<usize> {
        if self.bar_table.is_empty() {
            return None;
        }
        Some(
            match self
                .bar_table
                .binary_search_by_key(&sample_pos, |e| e.sample)
            {
                Ok(i) => i,
                Err(0) => 0,
                Err(i) => i - 1,
            },
        )
    }

    /// Number of beats in bar `bar_idx` — the signature's numerator, one
    /// beat per note of value `1/denominator`. A 7/8 bar has seven beats,
    /// each an eighth note; it does *not* have 3.5 quarter-note beats.
    pub fn beats_in_bar(&self, bar_idx: usize) -> u32 {
        self.bar_table
            .get(bar_idx)
            .map(|e| e.numerator as u32)
            .unwrap_or(self.numerator as u32)
    }

    /// Sample position of beat `beat` (0-based) in bar `bar_idx`.
    /// Uses logarithmic interpolation for correct intra-bar tempo.
    pub fn beat_sample_in_bar(&self, bar_idx: usize, beat: u32, sample_rate: u32) -> Option<u64> {
        let entry = self.bar_table.get(bar_idx)?;
        let num_beats = entry.numerator as f64;
        if beat as f64 >= num_beats {
            return None;
        }
        let tick_frac = beat as f64 / num_beats;
        if let Some(ne) = self.bar_table.get(bar_idx + 1) {
            let bar_samples = (ne.sample - entry.sample) as f64;
            let sf = tick_frac_to_sample_frac(tick_frac, entry.bpm as f64, ne.arrival_bpm as f64);
            Some(entry.sample + (sf * bar_samples) as u64)
        } else {
            let spb =
                self.samples_per_signature_beat(entry.bpm as f64, entry.denominator, sample_rate);
            Some(entry.sample + (beat as f64 * spb) as u64)
        }
    }

    /// Samples in one beat of the signature — one note of value
    /// `1/denominator`. BPM counts quarter notes, so an eighth-note beat
    /// is half a BPM beat.
    fn samples_per_signature_beat(&self, bpm: f64, denominator: u8, sample_rate: u32) -> f64 {
        let samples_per_quarter = sample_rate as f64 * 60.0 / bpm;
        samples_per_quarter * ticks_to_quarters(beat_len_ticks(denominator))
    }

    /// Samples per beat at the given sample rate (uses `bpm` field).
    pub fn samples_per_beat(&self, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / self.bpm as f64
    }

    /// Samples per bar at the given sample rate, from the project's
    /// default signature. Uses the bar's length in *quarter notes* —
    /// [`samples_per_beat`](Self::samples_per_beat) is a quarter note, so
    /// a 7/8 bar is 3.5 of them, not 7.
    pub fn samples_per_bar(&self, sample_rate: u32) -> f64 {
        self.samples_per_beat(sample_rate) * bar_len_quarters(self.numerator, self.denominator)
    }

    /// Convert a sample position to (bar, beat, fractional_beat).
    /// Bar and beat are 1-based. Uses the bar table when available
    /// so the position accounts for tempo changes.
    pub fn position_to_bars(&self, sample_pos: u64, sample_rate: u32) -> (u32, u8, f64) {
        if self.bar_table.is_empty() {
            let spb =
                self.samples_per_signature_beat(self.bpm as f64, self.denominator, sample_rate);
            let total_beats =
                snap_to_beat_grid(sample_pos as f64 / spb, SNAP_TOLERANCE_SAMPLES / spb);
            let bar = (total_beats / self.numerator as f64).floor() as u32 + 1;
            let beat_in_bar = (total_beats % self.numerator as f64).floor() as u8 + 1;
            let frac = total_beats.fract();
            return (bar, beat_in_bar, frac);
        }
        let idx = match self
            .bar_table
            .binary_search_by_key(&sample_pos, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let entry = &self.bar_table[idx];
        let bar = idx as u32 + 1; // 1-based
        let num_beats = entry.numerator as f64;
        if let Some(next) = self.bar_table.get(idx + 1) {
            let bar_samples = (next.sample - entry.sample) as f64;
            let sample_frac = if bar_samples > 0.0 {
                (sample_pos - entry.sample) as f64 / bar_samples
            } else {
                0.0
            };
            let tick_frac =
                sample_frac_to_tick_frac(sample_frac, entry.bpm as f64, next.arrival_bpm as f64);
            // This bar spans `bar_samples` samples and `num_beats` beats, so
            // one sample is `num_beats / bar_samples` beats.
            let eps = if bar_samples > 0.0 {
                SNAP_TOLERANCE_SAMPLES * num_beats / bar_samples
            } else {
                0.0
            };
            let beat_frac = snap_to_beat_grid(tick_frac * num_beats, eps);
            if beat_frac >= num_beats {
                // Snapped up onto the next bar's downbeat.
                return (bar + 1, 1, 0.0);
            }
            let beat = beat_frac.floor() as u8 + 1;
            (bar, beat, beat_frac.fract())
        } else {
            // Last tabulated bar, or past it. `rebuild_bar_table` stops at a
            // fixed horizon (last event + 200 bars), so a position beyond the
            // table must roll the surplus beats up into whole bars — counting
            // beats from the last entry forever would report that entry's bar
            // with an unbounded beat. Extrapolates at the last entry's tempo
            // and meter, matching `bar_to_sample`'s inverse.
            let spb =
                self.samples_per_signature_beat(entry.bpm as f64, entry.denominator, sample_rate);
            if !spb.is_finite() || spb <= 0.0 || num_beats <= 0.0 {
                return (bar, 1, 0.0);
            }
            // `bar_to_sample` truncates its extrapolation past the horizon, so
            // an exact downbeat can arrive short of the boundary and would
            // floor into the previous bar with a beat fraction of 0.9999.
            // One sample is `1/spb` beats.
            let beats_past = snap_to_beat_grid(
                (sample_pos - entry.sample) as f64 / spb,
                SNAP_TOLERANCE_SAMPLES / spb,
            );
            let mut bars_past = (beats_past / num_beats).floor();
            let mut beat_frac = beats_past - bars_past * num_beats;
            if beat_frac >= num_beats {
                bars_past += 1.0;
                beat_frac -= num_beats;
            }
            let bar = bar + bars_past as u32;
            (bar, beat_frac.floor() as u8 + 1, beat_frac.fract())
        }
    }

    /// Convert an absolute sample position to an absolute tick using
    /// the bar table. Inverse of [`Self::tick_to_abs_sample`] for a
    /// `clip_start` of 0. Used by the live MIDI recorder to
    /// timestamp incoming notes against the project tempo map.
    pub fn sample_to_abs_tick(&self, sample_pos: u64, sample_rate: u32) -> u64 {
        if self.bar_table.is_empty() {
            let spt =
                (sample_rate as f64 * 60.0 / self.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
            if spt <= 0.0 {
                return 0;
            }
            return (sample_pos as f64 / spt) as u64;
        }
        let idx = match self
            .bar_table
            .binary_search_by_key(&sample_pos, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let entry = &self.bar_table[idx];
        if let Some(next) = self.bar_table.get(idx + 1) {
            let bar_samples = (next.sample - entry.sample) as f64;
            let sample_frac = if bar_samples > 0.0 {
                (sample_pos - entry.sample) as f64 / bar_samples
            } else {
                0.0
            };
            let tick_frac =
                sample_frac_to_tick_frac(sample_frac, entry.bpm as f64, next.arrival_bpm as f64);
            entry.tick + (tick_frac * entry.ticks_in_bar as f64) as u64
        } else {
            // Past the last cached bar: extrapolate at the bar's BPM.
            let spt =
                (sample_rate as f64 * 60.0 / entry.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
            if spt <= 0.0 {
                return entry.tick;
            }
            entry.tick + ((sample_pos - entry.sample) as f64 / spt) as u64
        }
    }

    /// Convert a tick offset from a clip's start sample to an absolute
    /// sample position, integrating tempo changes via the bar table.
    /// O(log n) — safe for the real-time audio callback.
    pub fn tick_to_abs_sample(&self, clip_start: u64, tick_offset: u64, sample_rate: u32) -> u64 {
        if tick_offset == 0 {
            return clip_start;
        }
        if self.bar_table.is_empty() {
            let spt = (sample_rate as f64 * 60.0 / self.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
            return clip_start + (tick_offset as f64 * spt) as u64;
        }

        // Find the bar containing clip_start
        let start_idx = match self
            .bar_table
            .binary_search_by_key(&clip_start, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };

        // Compute the absolute tick position at clip_start using the
        // logarithmic sample↔tick mapping for correct intra-bar tempo.
        let se = &self.bar_table[start_idx];
        let clip_tick = if let Some(ne) = self.bar_table.get(start_idx + 1) {
            let bar_samples = (ne.sample - se.sample) as f64;
            let sample_frac = if bar_samples > 0.0 {
                (clip_start - se.sample) as f64 / bar_samples
            } else {
                0.0
            };
            let tick_frac =
                sample_frac_to_tick_frac(sample_frac, se.bpm as f64, ne.arrival_bpm as f64);
            se.tick as f64 + tick_frac * se.ticks_in_bar as f64
        } else {
            let spt = (sample_rate as f64 * 60.0 / se.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
            se.tick as f64 + (clip_start - se.sample) as f64 / spt
        };

        let target_tick = clip_tick + tick_offset as f64;

        // Binary search for the bar containing the target tick.
        let target_idx = match self.bar_table.binary_search_by(|e| {
            (e.tick as f64)
                .partial_cmp(&target_tick)
                .unwrap_or(std::cmp::Ordering::Less)
        }) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };

        let te = &self.bar_table[target_idx];
        let ticks_into_bar = target_tick - te.tick as f64;

        if let Some(ne) = self.bar_table.get(target_idx + 1) {
            let bar_samples = (ne.sample - te.sample) as f64;
            let tick_frac = if te.ticks_in_bar > 0 {
                ticks_into_bar / te.ticks_in_bar as f64
            } else {
                0.0
            };
            let sample_frac =
                tick_frac_to_sample_frac(tick_frac, te.bpm as f64, ne.arrival_bpm as f64);
            te.sample + (sample_frac * bar_samples) as u64
        } else {
            let spt = (sample_rate as f64 * 60.0 / te.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
            te.sample + (ticks_into_bar * spt) as u64
        }
    }

    /// Sample position at the start of a given 0-based bar number.
    /// Uses the precomputed bar table for O(1) lookup.
    pub fn bar_to_sample(&self, bar: u32) -> u64 {
        if let Some(entry) = self.bar_table.get(bar as usize) {
            return entry.sample;
        }
        // Past end of bar table: extrapolate from the last entry.
        if let Some(last) = self.bar_table.last() {
            let bars_past = bar as u64 - (self.bar_table.len() as u64 - 1);
            let spq = self.table_sample_rate as f64 * 60.0 / last.bpm as f64;
            let quarters = ticks_to_quarters(last.ticks_in_bar as u64);
            return last.sample + (bars_past as f64 * spq * quarters) as u64;
        }
        // No bar table at all: flat BPM.
        let spq = self.table_sample_rate as f64 * 60.0 / self.bpm as f64;
        (bar as f64 * spq * bar_len_quarters(self.numerator, self.denominator)) as u64
    }

    /// Return the interpolated (bpm, numerator, denominator) at a sample
    /// position. Uses the bar table for O(log n) lookup.
    pub fn tempo_at_sample(&self, sample_pos: u64, sample_rate: u32) -> (f32, u8, u8) {
        if self.bar_table.is_empty() {
            return (self.bpm, self.numerator, self.denominator);
        }
        let idx = match self
            .bar_table
            .binary_search_by_key(&sample_pos, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let entry = &self.bar_table[idx];
        let bpm = self.bpm_at(sample_pos, sample_rate);
        (bpm, entry.numerator, entry.denominator)
    }

    /// Convert a sample position to a (bar, fraction) pair where bar is
    /// 0-based and fraction is 0.0..1.0 within the bar.
    pub fn sample_to_bar(&self, sample_pos: u64, sample_rate: u32) -> (u32, f64) {
        if self.bar_table.is_empty() {
            let spq = sample_rate as f64 * 60.0 / self.bpm as f64;
            let bar_samples = spq * bar_len_quarters(self.numerator, self.denominator);
            if bar_samples <= 0.0 {
                return (0, 0.0);
            }
            let bar = (sample_pos as f64 / bar_samples).floor() as u32;
            let frac = (sample_pos as f64 - bar as f64 * bar_samples) / bar_samples;
            return (bar, frac);
        }
        let idx = match self
            .bar_table
            .binary_search_by_key(&sample_pos, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let entry = &self.bar_table[idx];
        let bar = idx as u32;
        if let Some(next) = self.bar_table.get(idx + 1) {
            let span = (next.sample - entry.sample) as f64;
            let frac = if span > 0.0 {
                (sample_pos - entry.sample) as f64 / span
            } else {
                0.0
            };
            (bar, frac)
        } else {
            // Past the last bar entry: extrapolate.
            let spq = sample_rate as f64 * 60.0 / entry.bpm as f64;
            let bar_samples = spq * ticks_to_quarters(entry.ticks_in_bar as u64);
            if bar_samples <= 0.0 {
                return (bar, 0.0);
            }
            let samples_past = (sample_pos - entry.sample) as f64;
            let extra_bars = (samples_past / bar_samples).floor() as u32;
            let frac = (samples_past - extra_bars as f64 * bar_samples) / bar_samples;
            (bar + extra_bars, frac)
        }
    }

    /// Recover the 0-based bar index whose start sample coincides with
    /// `sample_pos`, within `tolerance` samples, or `None` if the position
    /// is not on (or adjacent to) a bar boundary.
    ///
    /// This is the tempo-map-aware inverse of [`Self::bar_to_sample`] used
    /// to re-associate a loaded clip with the bar it was generated for.
    /// Unlike a `sample % samples_per_bar == 0` test against a truncated
    /// scalar, it accounts for tempo changes and absorbs the sub-sample
    /// rounding in the bar table, so a clip placed with `bar_to_sample(N)`
    /// round-trips back to bar `N` exactly.
    ///
    /// The candidate bar is found by [`Self::bar_index_at`] (the last bar
    /// entry at or before `sample_pos`); both it and the following bar are
    /// checked so a position rounded a hair *past* a boundary still matches.
    pub fn bar_at_sample_exact(&self, sample_pos: u64, tolerance: u64) -> Option<u32> {
        let idx = match self.bar_index_at(sample_pos) {
            Some(i) => i as u32,
            // No bar table (never rebuilt): fall back to the flat-BPM
            // conversion that `bar_to_sample` itself uses in this case.
            None => self.sample_to_bar(sample_pos, self.table_sample_rate).0,
        };
        for bar in [idx, idx + 1] {
            let bar_sample = self.bar_to_sample(bar);
            if bar_sample.abs_diff(sample_pos) <= tolerance {
                return Some(bar);
            }
        }
        None
    }

    /// Return the time signature numerator active at a given 0-based bar.
    ///
    /// This is the bar's *beat count*, not its length: do not multiply it
    /// by [`TICKS_PER_QUARTER_NOTE`] to get a bar's tick span, because a
    /// 7/8 beat is an eighth note. Use [`Self::bar_len_ticks_at`].
    pub fn numerator_at_bar(&self, bar: u32) -> u8 {
        if let Some(entry) = self.bar_table.get(bar as usize) {
            return entry.numerator;
        }
        self.bar_table
            .last()
            .map(|e| e.numerator)
            .unwrap_or(self.numerator)
    }

    /// Return the time signature denominator active at a given 0-based bar.
    pub fn denominator_at_bar(&self, bar: u32) -> u8 {
        if let Some(entry) = self.bar_table.get(bar as usize) {
            return entry.denominator;
        }
        self.bar_table
            .last()
            .map(|e| e.denominator)
            .unwrap_or(self.denominator)
    }

    /// Length of a given 0-based bar in ticks, honouring both halves of
    /// the signature active there. The single answer every caller that
    /// needs a bar's tick span should use — see
    /// [`bar_len_ticks`](super::signature::bar_len_ticks).
    pub fn bar_len_ticks_at(&self, bar: u32) -> u64 {
        if let Some(entry) = self.bar_table.get(bar as usize) {
            return entry.ticks_in_bar as u64;
        }
        bar_len_ticks(self.numerator_at_bar(bar), self.denominator_at_bar(bar))
    }

    /// Length of a given 0-based bar in quarter notes — the unit BPM
    /// counts in, so `samples_per_beat * bar_len_quarters_at(bar)` is the
    /// bar's length in samples.
    pub fn bar_len_quarters_at(&self, bar: u32) -> f64 {
        ticks_to_quarters(self.bar_len_ticks_at(bar))
    }
}
