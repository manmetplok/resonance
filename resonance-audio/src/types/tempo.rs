//! Tempo map and plugin/device info types.

/// Ticks per quarter note for MIDI timing (standard PPQ).
pub const TICKS_PER_QUARTER_NOTE: u64 = 480;

/// Describes an available audio input source (PipeWire/PulseAudio source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// PipeWire source name (e.g. "alsa_input.usb-...").
    pub name: String,
    /// Human-readable description (e.g. "USB Microphone Analog Stereo").
    pub description: String,
    /// Number of input channels exposed by this device. 0 means the
    /// channel count couldn't be determined at enumeration time.
    pub channels: u16,
}

impl std::fmt::Display for InputDeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}

/// Describes a plugin available in a .clap bundle (used during loading).
#[derive(Debug, Clone)]
pub struct PluginDescInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// True if the plugin declared the `instrument` feature in its CLAP descriptor.
    pub is_instrument: bool,
}

/// A plugin parameter descriptor with current value.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub current_value: f64,
}

/// A scanned plugin available for use, with its file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedPlugin {
    pub clap_file_path: String,
    pub clap_plugin_id: String,
    pub name: String,
    pub vendor: String,
    /// True if the plugin declared the `instrument` feature in its CLAP descriptor.
    pub is_instrument: bool,
}

impl std::fmt::Display for ScannedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.vendor.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{} ({})", self.name, self.vendor)
        }
    }
}

/// A tempo change point on the tempo track.
#[derive(Debug, Clone)]
pub struct TempoPoint {
    /// 0-based bar number where this tempo takes effect.
    pub bar: u32,
    pub bpm: f32,
}

/// A time signature change point on the signature track.
#[derive(Debug, Clone)]
pub struct SignaturePoint {
    /// 0-based bar number where this signature takes effect.
    pub bar: u32,
    pub numerator: u8,
    pub denominator: u8,
}

/// Return the interpolated BPM at a fractional bar position.
/// Between events at different bars the BPM ramps linearly.
/// When multiple events share the same bar (step change) the last
/// value at that bar wins.
pub fn bpm_at_bar(bar: f64, tempo_points: &[TempoPoint]) -> f64 {
    if tempo_points.is_empty() {
        return 120.0;
    }
    let mut prev_bpm = tempo_points[0].bpm as f64;
    let mut prev_bar = tempo_points[0].bar as f64;
    let mut next: Option<&TempoPoint> = None;

    for e in tempo_points {
        if (e.bar as f64) <= bar {
            prev_bpm = e.bpm as f64;
            prev_bar = e.bar as f64;
        } else {
            next = Some(e);
            break;
        }
    }

    if let Some(ne) = next {
        if prev_bar < bar {
            let t = (bar - prev_bar) / (ne.bar as f64 - prev_bar);
            return prev_bpm + (ne.bpm as f64 - prev_bpm) * t;
        }
    }

    prev_bpm
}

/// Return the arrival BPM at a bar — the ramp target approaching this
/// bar from the left. When multiple events share the same bar (step
/// change), this returns the FIRST event's value; `bpm_at_bar` returns
/// the LAST (departure) value.
pub fn arrival_bpm_at_bar(bar: u32, tempo_points: &[TempoPoint]) -> f64 {
    if tempo_points.is_empty() {
        return 120.0;
    }
    // Return the first event at exactly this bar if one exists.
    for e in tempo_points {
        if e.bar == bar {
            return e.bpm as f64;
        }
        if e.bar > bar {
            break;
        }
    }
    // No event at this bar — arrival equals the interpolated value.
    bpm_at_bar(bar as f64, tempo_points)
}

/// Average BPM across a bar (departure at start, arrival at end) / 2.
/// Uses arrival BPM for the end so that step changes at bar boundaries
/// don't erase the ramp target.
pub fn avg_bpm_for_bar(bar: u32, tempo_points: &[TempoPoint]) -> f64 {
    let bpm_start = bpm_at_bar(bar as f64, tempo_points);
    let bpm_end = arrival_bpm_at_bar(bar + 1, tempo_points);
    (bpm_start + bpm_end) / 2.0
}

/// Map a tick fraction (0..1) within a bar to a sample fraction (0..1),
/// accounting for linear BPM interpolation within the bar.
/// When BPM ramps from `bpm_s` to `bpm_e`, the tick→sample mapping is
/// logarithmic: `g(f) = ln(1 + (r-1)*f) / ln(r)` where `r = bpm_e/bpm_s`.
pub fn tick_frac_to_sample_frac(tick_frac: f64, bpm_start: f64, bpm_end: f64) -> f64 {
    let r = bpm_end / bpm_start;
    if (r - 1.0).abs() < 1e-6 {
        return tick_frac;
    }
    (1.0 + (r - 1.0) * tick_frac).ln() / r.ln()
}

/// Map a sample fraction (0..1) within a bar to a tick fraction (0..1),
/// accounting for linear BPM interpolation within the bar.
/// Inverse of `tick_frac_to_sample_frac`.
pub fn sample_frac_to_tick_frac(sample_frac: f64, bpm_start: f64, bpm_end: f64) -> f64 {
    let r = bpm_end / bpm_start;
    if (r - 1.0).abs() < 1e-6 {
        return sample_frac;
    }
    (r.powf(sample_frac) - 1.0) / (r - 1.0)
}

/// Precomputed entry for O(log n) BPM lookup by sample position.
#[derive(Debug, Clone)]
struct BarEntry {
    /// Sample position at the start of this bar.
    sample: u64,
    /// Departure BPM at this bar (after any step change).
    bpm: f32,
    /// Arrival BPM at this bar (ramp target from the previous bar).
    /// Equals `bpm` when there is no step change at this bar.
    arrival_bpm: f32,
    /// Cumulative tick count at the start of this bar.
    tick: u64,
    /// Number of ticks in this bar (depends on time signature).
    ticks_in_bar: u32,
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
    bar_table: Vec<BarEntry>,
    /// Sample rate used to build the bar table.
    table_sample_rate: u32,
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
        if self.tempo_points.len() <= 1 {
            return;
        }
        let sr = sample_rate as f64;
        let mut sample_pos: f64 = 0.0;
        let mut tick_pos: u64 = 0;
        let mut cur_num = self.signature_points.first()
            .map(|e| e.numerator).unwrap_or(self.numerator);
        let mut si: usize = if self.signature_points.first()
            .map(|e| e.bar) == Some(0) { 1 } else { 0 };

        // Build table from bar 0 to the last event + generous margin.
        let last_bar = self.tempo_points.iter()
            .map(|e| e.bar)
            .chain(self.signature_points.iter().map(|e| e.bar))
            .max()
            .unwrap_or(0);
        let table_end = (last_bar + 200).min(10_000);
        for b in 0u32..table_end {
            while let Some(e) = self.signature_points.get(si) {
                if e.bar == b { cur_num = e.numerator; si += 1; } else { break; }
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
        let idx = match self.bar_table.binary_search_by_key(&sample_pos, |e| e.sample) {
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

    /// Number of bars in the precomputed bar table.
    pub fn bar_count(&self) -> usize {
        self.bar_table.len()
    }

    /// Find the bar table index containing the given sample position.
    pub fn bar_index_at(&self, sample_pos: u64) -> Option<usize> {
        if self.bar_table.is_empty() {
            return None;
        }
        Some(match self.bar_table.binary_search_by_key(&sample_pos, |e| e.sample) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        })
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
        let idx = match self.bar_table.binary_search_by_key(&sample_pos, |e| e.sample) {
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
            let tick_frac = sample_frac_to_tick_frac(
                sample_frac, entry.bpm as f64, next.arrival_bpm as f64,
            );
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

    /// Convert a tick offset from a clip's start sample to an absolute
    /// sample position, integrating tempo changes via the bar table.
    /// O(log n) — safe for the real-time audio callback.
    pub fn tick_to_abs_sample(&self, clip_start: u64, tick_offset: u64, sample_rate: u32) -> u64 {
        if tick_offset == 0 {
            return clip_start;
        }
        if self.bar_table.is_empty() {
            let spt = (sample_rate as f64 * 60.0 / self.bpm as f64)
                / TICKS_PER_QUARTER_NOTE as f64;
            return clip_start + (tick_offset as f64 * spt) as u64;
        }

        // Find the bar containing clip_start
        let start_idx = match self.bar_table.binary_search_by_key(&clip_start, |e| e.sample) {
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
            let tick_frac = sample_frac_to_tick_frac(
                sample_frac, se.bpm as f64, ne.arrival_bpm as f64,
            );
            se.tick as f64 + tick_frac * se.ticks_in_bar as f64
        } else {
            let spt = (sample_rate as f64 * 60.0 / se.bpm as f64)
                / TICKS_PER_QUARTER_NOTE as f64;
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
            let sample_frac = tick_frac_to_sample_frac(
                tick_frac, te.bpm as f64, ne.arrival_bpm as f64,
            );
            te.sample + (sample_frac * bar_samples) as u64
        } else {
            let spt = (sample_rate as f64 * 60.0 / te.bpm as f64)
                / TICKS_PER_QUARTER_NOTE as f64;
            te.sample + (ticks_into_bar * spt) as u64
        }
    }

    /// Format a sample position as "bar.beat".
    pub fn format_position(&self, sample_pos: u64, sample_rate: u32) -> String {
        let (bar, beat, _) = self.position_to_bars(sample_pos, sample_rate);
        format!("{}.{}", bar, beat)
    }

    /// Format a sample position as "mm:ss.mmm" wall-clock time.
    pub fn format_time(&self, sample_pos: u64, sample_rate: u32) -> String {
        let total_secs = sample_pos as f64 / sample_rate as f64;
        let minutes = (total_secs / 60.0).floor() as u32;
        let seconds = total_secs - (minutes as f64 * 60.0);
        format!("{:02}:{:06.3}", minutes, seconds)
    }
}
