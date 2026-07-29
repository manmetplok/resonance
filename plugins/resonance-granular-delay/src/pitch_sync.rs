//! Pitch-synchronous (Voice/Mono) grain scheduling — PSOLA-style
//! placement for monophonic material (ba todo #1082, doc #252 §4).
//!
//! The resonance-dsp [`PitchTracker`] runs on the dry input as it is
//! written into the delay buffer; its period markers are translated to
//! write-stream positions and stored in a pre-allocated marker ring
//! alongside the buffer. While the tracker is voiced, grain onsets snap
//! to the pitch mark nearest the delay tap, grain length is two tracked
//! periods with a Hann window (texture forced), and transposition is
//! realized by changing *output onset spacing* (period / α) instead of
//! per-grain resampling — classic TD-PSOLA: formants are preserved and
//! the AM/comb beating of fixed-rate granulation on pitched material
//! disappears. Unvoiced or unlocked input falls back to the async
//! scheduler transparently (the caller crossfades the buses).
//!
//! Everything is pre-allocated in [`PitchSyncGranulator::new`]; `feed`
//! and `render` perform no allocation and take no locks. Onset
//! scheduling runs on absolute sample counters, so rendering is
//! invariant to how the host slices blocks.

use resonance_dsp::{read_hermite_wrapped, PitchTracker};

/// Fixed voice-pool size. Overlap is `2 · α` (two-period grains at
/// `period / α` spacing), at most 8 for α = 4 (+24 st); 16 leaves
/// headroom for drain transients.
pub const MAX_VOICES: usize = 16;

/// Marker-ring capacity. Worst case the delay tap sits
/// `MAX_DELAY_SECONDS` behind the head with the tracker at its 800 Hz
/// upper bound: 4 s × 800 markers/s = 3200; 4096 covers it.
const MARKER_CAPACITY: usize = 4096;

/// Onset log length (test/metering aid).
const ONSET_LOG: usize = 64;

/// Safety margin (samples) between any voice read span and the write
/// head (covers the Hermite reader's ±2-sample support, same rationale
/// as the grain engine's guard).
const HEAD_MARGIN: f64 = 16.0;

/// Floor on the synthesis onset spacing, samples.
const MIN_SPACING: f64 = 8.0;

/// A marker further than this many periods from the read target is not
/// usable — the caller's engage gate falls back to async instead.
pub const MARKER_RANGE_PERIODS: f64 = 2.0;

/// How onsets are placed for a render call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// Voiced: snap each onset's read centre to the nearest pitch mark.
    Marker,
    /// Draining crossfade: keep spawning at the nominal tap position
    /// with the last known period, so the fallback blend never gaps.
    Nominal,
    /// No new onsets; live voices finish.
    None,
}

/// One pooled PSOLA voice: a two-period Hann-windowed segment read at
/// unity rate from the caller's circular buffer.
#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    /// Fractional read position (absolute write-stream samples; wraps
    /// through `read_hermite_wrapped`'s power-of-two mask).
    read_pos: f64,
    /// Duration in output samples (two periods).
    dur: f64,
    /// Envelope phase in output samples.
    env: f64,
    gain: f32,
    /// Intra-block onset offset for a voice spawned this block.
    start: u32,
}

impl Voice {
    const INACTIVE: Voice = Voice {
        active: false,
        read_pos: 0.0,
        dur: 0.0,
        env: 0.0,
        gain: 0.0,
        start: 0,
    };
}

/// Read-only view of one sounding PSOLA voice (metering aid, ba todo
/// #1135), mirroring `resonance_dsp::GrainView` for the editor's cloud.
#[derive(Debug, Clone, Copy)]
pub struct VoiceView {
    /// Fractional read position, absolute write-stream samples.
    pub read_pos: f64,
    /// Total duration in output samples (two tracked periods).
    pub dur_samples: f64,
    /// Current Hann-enveloped level (window × the 1/α overlap gain).
    pub level: f32,
}

/// Streaming PSOLA granulator: tracker, marker ring and voice pool.
pub struct PitchSyncGranulator {
    tracker: PitchTracker,
    /// Write-stream position corresponding to tracker sample 0; markers
    /// are stored as `stream_base + marker`.
    stream_base: u64,
    /// Full-rate samples fed since the last (re)base.
    fed: u64,
    /// Circular ring of absolute marker positions (ascending).
    markers: Vec<f64>,
    /// Total markers pushed; the ring holds the newest
    /// `min(total, MARKER_CAPACITY)`.
    marker_total: u64,
    /// Cached logical marker index for the nearest-marker search (the
    /// tap advances monotonically, so lookups are amortized O(1)).
    search_cache: u64,
    /// Last known fundamental period, full-rate samples. Held across
    /// unvoiced spans and freeze (a frozen head keeps spawning from the
    /// last known period).
    period: f64,
    /// Absolute output-stream time of the next synthesis onset.
    next_syn: f64,
    /// Absolute output samples rendered.
    out_pos: u64,
    voices: [Voice; MAX_VOICES],
    /// Total onsets spawned (metering/test aid).
    onsets: u64,
    /// Ring of the most recent onset times, absolute output samples.
    onset_log: [f64; ONSET_LOG],
}

impl PitchSyncGranulator {
    /// Pre-allocates the tracker, marker ring and voice pool.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not finite or below 8 kHz (the
    /// tracker front end's requirement).
    pub fn new(sample_rate: f32) -> Self {
        Self {
            tracker: PitchTracker::new(sample_rate),
            stream_base: 0,
            fed: 0,
            markers: vec![0.0; MARKER_CAPACITY],
            marker_total: 0,
            search_cache: 0,
            period: 0.0,
            next_syn: 0.0,
            out_pos: 0,
            voices: [Voice::INACTIVE; MAX_VOICES],
            onsets: 0,
            onset_log: [0.0; ONSET_LOG],
        }
    }

    /// Full reset (tracker, markers, voices, counters); allocation-free.
    pub fn reset(&mut self) {
        self.tracker.reset();
        self.stream_base = 0;
        self.fed = 0;
        self.marker_total = 0;
        self.search_cache = 0;
        self.period = 0.0;
        self.next_syn = 0.0;
        self.out_pos = 0;
        self.voices = [Voice::INACTIVE; MAX_VOICES];
        self.onsets = 0;
    }

    /// Rebase the tracker onto `stream_pos` (the caller's write-head
    /// position) if the fed stream is not already contiguous with it —
    /// e.g. when the mode engages mid-run. Existing markers stay valid
    /// (they are stored in absolute stream coordinates); the tracker
    /// itself re-locks within a few analysis hops.
    pub fn sync_to(&mut self, stream_pos: u64) {
        if self.stream_base + self.fed != stream_pos {
            self.tracker.reset();
            self.fed = 0;
            self.stream_base = stream_pos;
        }
    }

    /// Feed the dry input samples just written to the buffer, pushing
    /// emitted period markers (translated to stream positions) into the
    /// marker ring and updating the held period. Allocation-free.
    pub fn feed(&mut self, block: &[f32]) {
        let est = self.tracker.feed(block);
        self.fed += block.len() as u64;
        let base = self.stream_base as f64;
        for &m in self.tracker.markers() {
            let idx = (self.marker_total % MARKER_CAPACITY as u64) as usize;
            self.markers[idx] = base + m;
            self.marker_total += 1;
        }
        if est.voiced {
            self.period = f64::from(est.period_samples);
        }
    }

    /// True while the tracker reports a reliable pitch and a period has
    /// been latched.
    pub fn voiced(&self) -> bool {
        self.tracker.latest().voiced && self.period > 0.0
    }

    /// Last known fundamental period in full-rate samples (0 before the
    /// first voiced lock; held across unvoiced spans and freeze).
    pub fn period_samples(&self) -> f64 {
        self.period
    }

    /// Whether a stored pitch mark lies within
    /// [`MARKER_RANGE_PERIODS`] periods of `target` — the caller's
    /// engage gate for marker-snapped spawning.
    pub fn has_marker_near(&mut self, target: f64) -> bool {
        let p = self.period;
        p > 0.0
            && self
                .nearest_marker(target)
                .is_some_and(|m| (m - target).abs() <= MARKER_RANGE_PERIODS * p)
    }

    /// Currently sounding voices.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Read-only views of the currently sounding voices (allocation-free
    /// metering aid, ba todo #1135): read position, duration and the
    /// current Hann-enveloped level, for the editor's grain-cloud view.
    pub fn active_voice_views(&self) -> impl Iterator<Item = VoiceView> + '_ {
        self.voices.iter().filter(|v| v.active).map(|v| VoiceView {
            read_pos: v.read_pos,
            dur_samples: v.dur,
            level: (0.5 - 0.5 * ((std::f64::consts::TAU * v.env / v.dur) as f32).cos()) * v.gain,
        })
    }

    /// Total onsets spawned (metering/test aid).
    pub fn onsets(&self) -> u64 {
        self.onsets
    }

    /// The most recent onset times in absolute output samples, oldest
    /// first (up to 64). Test/metering aid — allocates, so keep it off
    /// the audio path.
    pub fn recent_onsets(&self) -> Vec<f64> {
        let len = self.onsets.min(ONSET_LOG as u64);
        (0..len)
            .map(|i| self.onset_log[((self.onsets - len + i) % ONSET_LOG as u64) as usize])
            .collect()
    }

    /// Render one block, accumulating into `out_l` / `out_r` (equal
    /// lengths). `write_pos` is the caller's write head at the block
    /// start, `head_advance` its per-sample motion (0 while frozen),
    /// `delay_samples` the effective tap distance and `alpha` the
    /// transposition factor `2^(semitones/12)` — realized purely as
    /// onset-spacing `period / alpha`, voices always read at unity
    /// rate. No allocation, no locks.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        buf_l: &[f32],
        buf_r: &[f32],
        write_pos: u64,
        head_advance: f64,
        delay_samples: f64,
        alpha: f64,
        spawn: SpawnMode,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let n = out_l.len().min(out_r.len());
        let out0 = self.out_pos as f64;

        if spawn != SpawnMode::None && self.period > 0.0 {
            let spacing = (self.period / alpha).max(MIN_SPACING);
            if self.next_syn < out0 {
                // Re-engaging after a gap: resume now instead of
                // burst-spawning the missed lattice.
                self.next_syn = out0;
            }
            while self.next_syn < out0 + n as f64 {
                let o = self.next_syn - out0;
                self.spawn_voice(
                    buf_l.len(),
                    write_pos,
                    head_advance,
                    delay_samples,
                    o,
                    alpha,
                    spawn,
                );
                self.next_syn += spacing;
            }
        }

        for v in self.voices.iter_mut() {
            if !v.active {
                continue;
            }
            let start = (v.start as usize).min(n);
            v.start = 0;
            for k in start..n {
                // Two-period Hann (texture forced, doc #252 §4).
                let w = 0.5 - 0.5 * ((std::f64::consts::TAU * v.env / v.dur) as f32).cos();
                out_l[k] += read_hermite_wrapped(buf_l, v.read_pos) * w * v.gain;
                out_r[k] += read_hermite_wrapped(buf_r, v.read_pos) * w * v.gain;
                v.read_pos += 1.0;
                v.env += 1.0;
                if v.env >= v.dur {
                    v.active = false;
                    break;
                }
            }
        }
        self.out_pos += n as u64;
    }

    /// Place one voice at fractional block offset `o`.
    #[allow(clippy::too_many_arguments)]
    fn spawn_voice(
        &mut self,
        buf_len: usize,
        write_pos: u64,
        head_advance: f64,
        delay_samples: f64,
        o: f64,
        alpha: f64,
        spawn: SpawnMode,
    ) {
        let p = self.period;
        let head_at_onset = write_pos as f64 + o * head_advance;
        let target = head_at_onset - delay_samples;
        let mut m = match spawn {
            SpawnMode::Marker => match self.nearest_marker(target) {
                Some(m) if (m - target).abs() <= MARKER_RANGE_PERIODS * p => m,
                _ => return, // no usable pitch mark near the tap
            },
            SpawnMode::Nominal => target,
            SpawnMode::None => return,
        };

        // Write-head collision guard for a unity-rate read of 2p
        // samples starting at m − p while the head moves at
        // `head_advance`: the minimum head−reader gap over the voice's
        // life requires m ≤ head + p·(2·advance − 1) − margin. Shift
        // back by whole periods — phase-preserving — to satisfy it.
        let limit = head_at_onset + p * (2.0 * head_advance - 1.0) - HEAD_MARGIN;
        if m > limit {
            m -= ((m - limit) / p).ceil() * p;
        }
        // History guard: never read before the retained ring span.
        if m - p < head_at_onset - buf_len as f64 + HEAD_MARGIN {
            return;
        }

        let Some(slot) = self.voices.iter().position(|v| !v.active) else {
            return; // pool exhausted: drop the onset (overlap-bounded)
        };
        self.voices[slot] = Voice {
            active: true,
            read_pos: m - p,
            dur: (2.0 * p).max(4.0),
            env: 0.0,
            // Coherent PSOLA overlap-add at spacing p/α sums the Hann
            // train to α; compensate so the level is transposition-
            // independent.
            gain: (1.0 / alpha) as f32,
            start: o as u32,
        };
        self.onset_log[(self.onsets % ONSET_LOG as u64) as usize] = self.out_pos as f64 + o;
        self.onsets += 1;
    }

    /// Nearest stored marker to `target`, using (and updating) the
    /// monotonic search cache. Amortized O(1) per onset.
    fn nearest_marker(&mut self, target: f64) -> Option<f64> {
        let len = self.marker_total.min(MARKER_CAPACITY as u64);
        if len == 0 {
            return None;
        }
        let lo = self.marker_total - len;
        let hi = self.marker_total; // exclusive
        let markers = &self.markers;
        let at = |i: u64| markers[(i % MARKER_CAPACITY as u64) as usize];
        let mut i = self.search_cache.clamp(lo, hi - 1);
        while i + 1 < hi && at(i) < target {
            i += 1;
        }
        while i > lo && at(i) > target {
            i -= 1;
        }
        // `i` is the newest marker ≤ target (or the oldest overall);
        // the next one up may be closer.
        let mut best = at(i);
        if i + 1 < hi {
            let up = at(i + 1);
            if (up - target).abs() < (best - target).abs() {
                best = up;
            }
        }
        self.search_cache = i;
        Some(best)
    }
}
