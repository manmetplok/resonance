//! Bar / beat / subdivision math (time-signature aware) on `TempoMap`.

use super::conversion::{sample_frac_to_tick_frac, tick_frac_to_sample_frac};
use super::map::TempoMap;
use super::TICKS_PER_QUARTER_NOTE;

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

    /// Number of beats in bar `bar_idx`.
    pub fn beats_in_bar(&self, bar_idx: usize) -> u32 {
        self.bar_table
            .get(bar_idx)
            .map(|e| e.ticks_in_bar / TICKS_PER_QUARTER_NOTE as u32)
            .unwrap_or(self.numerator as u32)
    }

    /// Sample position of beat `beat` (0-based) in bar `bar_idx`.
    /// Uses logarithmic interpolation for correct intra-bar tempo.
    pub fn beat_sample_in_bar(&self, bar_idx: usize, beat: u32, sample_rate: u32) -> Option<u64> {
        let entry = self.bar_table.get(bar_idx)?;
        let num_beats = entry.ticks_in_bar as f64 / TICKS_PER_QUARTER_NOTE as f64;
        if beat as f64 >= num_beats {
            return None;
        }
        let tick_frac = beat as f64 / num_beats;
        if let Some(ne) = self.bar_table.get(bar_idx + 1) {
            let bar_samples = (ne.sample - entry.sample) as f64;
            let sf = tick_frac_to_sample_frac(tick_frac, entry.bpm as f64, ne.arrival_bpm as f64);
            Some(entry.sample + (sf * bar_samples) as u64)
        } else {
            let spb = sample_rate as f64 * 60.0 / entry.bpm as f64;
            Some(entry.sample + (beat as f64 * spb) as u64)
        }
    }

    /// Samples per beat at the given sample rate (uses `bpm` field).
    pub fn samples_per_beat(&self, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / self.bpm as f64
    }

    /// Samples per beat at a specific sample position (uses tempo events).
    pub fn samples_per_beat_at(&self, sample_pos: u64, sample_rate: u32) -> f64 {
        let bpm = self.bpm_at(sample_pos, sample_rate);
        sample_rate as f64 * 60.0 / bpm as f64
    }

    /// Samples per bar at the given sample rate.
    pub fn samples_per_bar(&self, sample_rate: u32) -> f64 {
        self.samples_per_beat(sample_rate) * self.numerator as f64
    }

    /// Convert a sample position to (bar, beat, fractional_beat).
    /// Bar and beat are 1-based. Uses the bar table when available
    /// so the position accounts for tempo changes.
    pub fn position_to_bars(&self, sample_pos: u64, sample_rate: u32) -> (u32, u8, f64) {
        if self.bar_table.is_empty() {
            let spb = self.samples_per_beat(sample_rate);
            let total_beats = sample_pos as f64 / spb;
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
        let num_beats = entry.ticks_in_bar as f64 / TICKS_PER_QUARTER_NOTE as f64;
        if let Some(next) = self.bar_table.get(idx + 1) {
            let bar_samples = (next.sample - entry.sample) as f64;
            let sample_frac = if bar_samples > 0.0 {
                (sample_pos - entry.sample) as f64 / bar_samples
            } else {
                0.0
            };
            let tick_frac =
                sample_frac_to_tick_frac(sample_frac, entry.bpm as f64, next.arrival_bpm as f64);
            let beat_frac = tick_frac * num_beats;
            let beat = beat_frac.floor() as u8 + 1;
            (bar, beat, beat_frac.fract())
        } else {
            let spb = sample_rate as f64 * 60.0 / entry.bpm as f64;
            let beat_frac = (sample_pos - entry.sample) as f64 / spb;
            let beat = beat_frac.floor() as u8 + 1;
            (bar, beat, beat_frac.fract())
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
            let spb = self.table_sample_rate as f64 * 60.0 / last.bpm as f64;
            let num = last.numerator as f64;
            return last.sample + (bars_past as f64 * spb * num) as u64;
        }
        // No bar table at all: flat BPM.
        let spb = self.table_sample_rate as f64 * 60.0 / self.bpm as f64;
        (bar as f64 * spb * self.numerator as f64) as u64
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
            let spb = sample_rate as f64 * 60.0 / self.bpm as f64;
            let bar_samples = spb * self.numerator as f64;
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
            let spb = sample_rate as f64 * 60.0 / entry.bpm as f64;
            let bar_samples = spb * entry.numerator as f64;
            if bar_samples <= 0.0 {
                return (bar, 0.0);
            }
            let samples_past = (sample_pos - entry.sample) as f64;
            let extra_bars = (samples_past / bar_samples).floor() as u32;
            let frac = (samples_past - extra_bars as f64 * bar_samples) / bar_samples;
            (bar + extra_bars, frac)
        }
    }

    /// Return the time signature numerator active at a given 0-based bar.
    pub fn numerator_at_bar(&self, bar: u32) -> u8 {
        if let Some(entry) = self.bar_table.get(bar as usize) {
            return entry.numerator;
        }
        self.bar_table
            .last()
            .map(|e| e.numerator)
            .unwrap_or(self.numerator)
    }
}
