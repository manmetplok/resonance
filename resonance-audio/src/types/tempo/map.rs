//! The `TempoMap` type and its construction / lookup methods.

use serde::{Deserialize, Serialize};

use super::conversion::{arrival_bpm_at_bar, avg_bpm_for_bar, bpm_at_bar};
use super::TICKS_PER_QUARTER_NOTE;

/// A tempo change point on the tempo track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoPoint {
    /// 0-based bar number where this tempo takes effect.
    pub bar: u32,
    pub bpm: f32,
}

/// A time signature change point on the signature track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignaturePoint {
    /// 0-based bar number where this signature takes effect.
    pub bar: u32,
    pub numerator: u8,
    pub denominator: u8,
}

/// Precomputed entry for O(log n) BPM lookup by sample position.
#[derive(Debug, Clone)]
pub(super) struct BarEntry {
    /// Sample position at the start of this bar.
    pub sample: u64,
    /// Departure BPM at this bar (after any step change).
    pub bpm: f32,
    /// Arrival BPM at this bar (ramp target from the previous bar).
    /// Equals `bpm` when there is no step change at this bar.
    pub arrival_bpm: f32,
    /// Cumulative tick count at the start of this bar.
    pub tick: u64,
    /// Number of ticks in this bar (depends on time signature).
    pub ticks_in_bar: u32,
    /// Time signature numerator active at this bar.
    pub numerator: u8,
    /// Time signature denominator active at this bar.
    pub denominator: u8,
}

/// Tempo and time signature state.
#[derive(Debug, Clone)]
pub struct TempoMap {
    pub bpm: f32,
    pub numerator: u8,
    pub denominator: u8,
    pub metronome_enabled: bool,
    /// Full tempo event list (sorted by bar).
    pub tempo_points: Vec<TempoPoint>,
    pub signature_points: Vec<SignaturePoint>,
    /// Precomputed bar→sample table for fast BPM lookup in the mixer.
    pub(super) bar_table: Vec<BarEntry>,
    /// Sample rate used to build the bar table.
    pub(super) table_sample_rate: u32,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            numerator: 4,
            denominator: 4,
            metronome_enabled: false,
            tempo_points: Vec::new(),
            signature_points: Vec::new(),
            bar_table: Vec::new(),
            table_sample_rate: 0,
        }
    }
}

impl TempoMap {
    /// Rebuild the precomputed bar table. Call after changing tempo or
    /// signature events, on the engine control thread (not audio).
    pub fn rebuild_bar_table(&mut self, sample_rate: u32) {
        self.table_sample_rate = sample_rate;
        self.bar_table.clear();
        if sample_rate == 0 {
            return;
        }
        let sr = sample_rate as f64;
        let mut sample_pos: f64 = 0.0;
        let mut tick_pos: u64 = 0;
        let mut cur_num = self
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(self.numerator);
        let mut si: usize = if self.signature_points.first().map(|e| e.bar) == Some(0) {
            1
        } else {
            0
        };

        // Build table from bar 0 to the last event + generous margin.
        let last_bar = self
            .tempo_points
            .iter()
            .map(|e| e.bar)
            .chain(self.signature_points.iter().map(|e| e.bar))
            .max()
            .unwrap_or(0);
        let table_end = (last_bar + 200).min(10_000);
        let mut cur_den = self
            .signature_points
            .first()
            .map(|e| e.denominator)
            .unwrap_or(self.denominator);
        for b in 0u32..table_end {
            while let Some(e) = self.signature_points.get(si) {
                if e.bar == b {
                    cur_num = e.numerator;
                    cur_den = e.denominator;
                    si += 1;
                } else {
                    break;
                }
            }
            let bpm = bpm_at_bar(b as f64, &self.tempo_points) as f32;
            let arr = arrival_bpm_at_bar(b, &self.tempo_points) as f32;
            let ticks_in_bar = cur_num as u32 * TICKS_PER_QUARTER_NOTE as u32;
            self.bar_table.push(BarEntry {
                sample: sample_pos.round() as u64,
                bpm,
                arrival_bpm: arr,
                tick: tick_pos,
                ticks_in_bar,
                numerator: cur_num,
                denominator: cur_den,
            });
            tick_pos += ticks_in_bar as u64;
            let avg = avg_bpm_for_bar(b, &self.tempo_points);
            let samples_per_beat = sr * 60.0 / avg;
            sample_pos += samples_per_beat * cur_num as f64;
        }
    }

    /// Return the interpolated BPM at a sample position. Uses the
    /// precomputed bar table for O(log n) lookup — safe for the
    /// real-time audio callback.
    pub fn bpm_at(&self, sample_pos: u64, _sample_rate: u32) -> f32 {
        if self.bar_table.is_empty() {
            return self.bpm;
        }
        // Binary search for the last bar entry <= sample_pos.
        let idx = match self
            .bar_table
            .binary_search_by_key(&sample_pos, |e| e.sample)
        {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let entry = &self.bar_table[idx];
        // Interpolate within the bar if there's a next entry.
        if let Some(next) = self.bar_table.get(idx + 1) {
            let span = (next.sample - entry.sample) as f64;
            if span > 0.0 {
                let t = (sample_pos - entry.sample) as f64 / span;
                return entry.bpm + (next.arrival_bpm - entry.bpm) * t as f32;
            }
        }
        entry.bpm
    }

    /// Update the `bpm` field from the bar table at the given playhead.
    /// Call once per audio block to keep the stable BPM in sync with
    /// the tempo map. This is O(log n) — safe for real-time.
    pub fn sync_bpm_at(&mut self, sample_pos: u64, sample_rate: u32) {
        if !self.bar_table.is_empty() {
            self.bpm = self.bpm_at(sample_pos, sample_rate);
        }
    }

    /// Whether [`sync_bpm_at`](Self::sync_bpm_at) would actually move
    /// the stable `bpm`. The engine loop checks this under a read lock
    /// before escalating to a write lock — for static-tempo projects
    /// (the common case) the answer is always `false`, which keeps the
    /// audio thread's `tempo_map.try_read()` from contending with us
    /// every engine tick.
    pub fn sync_bpm_would_change(&self, sample_pos: u64, sample_rate: u32) -> bool {
        if self.bar_table.is_empty() {
            return false;
        }
        (self.bpm_at(sample_pos, sample_rate) - self.bpm).abs() > 1e-4
    }
}
