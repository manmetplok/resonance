//! Early reflections: parallel multi-tap stereo delay producing 12 discrete
//! taps before the diffused tail arrives. Gives the reverb its spatial
//! signature ("room shape").

use resonance_dsp::{DelayLine, SimpleRng};

pub const ER_TAPS: usize = 12;
pub(super) const ER_BASE_MAX_MS: f32 = 220.0; // Upper bound for the longest tap at time_scale=1.0
pub(super) const ER_TIME_MIN: f32 = 0.25; // er_time=0 → 0.25× base
pub(super) const ER_TIME_MAX: f32 = 2.0; // er_time=1 → 2.0× base

/// Crossfade length for an `er_time` change, ms. The taps never
/// relocate mid-signal: the whole old tap set fades out while the new
/// one fades in (see `set_time`).
const ER_FADE_MS: f32 = 20.0;

/// Parallel multi-tap early reflections. Generates 12 discrete stereo taps
/// before the diffused tail arrives, giving the reverb its spatial signature.
pub(super) struct EarlyReflections {
    delay_l: DelayLine,
    delay_r: DelayLine,
    /// Base tap times in ms (left, right), fixed at construction.
    base_times_ms: [(f32, f32); ER_TAPS],
    /// Per-tap gains (left, right), including polarity flips.
    pub(super) gains: [(f32, f32); ER_TAPS],
    /// Current `er_time` multiplier applied to `base_times_ms`.
    time_scale: f32,
    /// Cached scaled tap times in samples. Recomputed when `time_scale` changes.
    scaled_samples: [(f32, f32); ER_TAPS],
    /// Cached scaled tap times in ms (for the viz).
    pub(super) scaled_ms: [(f32, f32); ER_TAPS],
    /// Outgoing tap set while a crossfade is running (see `set_time`).
    fade_from: [(f32, f32); ER_TAPS],
    /// Remaining crossfade samples; 0 means no fade is running.
    fade_left: u32,
    /// Total crossfade length in samples at the current sample rate.
    fade_total: u32,
    /// Newest scale requested while a fade was already running; picked
    /// up as the next fade the moment the running one completes.
    pending_scale: Option<f32>,
    /// Last scale requested — dedupes the per-block `set_time` calls.
    requested_scale: f32,
    /// False until the first processed sample. While false `set_time`
    /// snaps (configuring a fresh/reset instance must not fade in from
    /// a stale tap set).
    primed: bool,
    /// Current `er_level` applied to the summed output before it joins the wet path.
    level: f32,
    sample_rate: f32,
}

impl EarlyReflections {
    pub(super) fn new(sample_rate: f32) -> Self {
        // Enough headroom for the longest tap at time_scale=ER_TIME_MAX.
        let max_samples = ((ER_BASE_MAX_MS * ER_TIME_MAX) * 0.001 * sample_rate) as usize + 64;
        let mut rng = SimpleRng::new(0xa3f1_7b2c_0005_beef);

        // Generate tap times with a mild clustering bias: early taps
        // cluster in the first 60ms, later taps spread out toward 220ms.
        // This matches how real rooms produce dense-early / sparser-late
        // first reflections.
        let mut base_times_ms = [(0.0f32, 0.0f32); ER_TAPS];
        let mut gains = [(0.0f32, 0.0f32); ER_TAPS];
        for i in 0..ER_TAPS {
            let t = i as f32 / (ER_TAPS - 1).max(1) as f32;
            // Curve spreads the taps: sqrt gives more density early.
            let curved = t.sqrt();
            let center_ms = 4.0 + curved * (ER_BASE_MAX_MS - 4.0);

            // Small per-side jitter so L/R don't land exactly on top of each other.
            let jitter_l = ((rng.next_u32() & 0xffff) as f32 / 65535.0 - 0.5) * 8.0;
            let jitter_r = ((rng.next_u32() & 0xffff) as f32 / 65535.0 - 0.5) * 8.0;
            base_times_ms[i] = (
                (center_ms + jitter_l).max(1.0),
                (center_ms + jitter_r).max(1.0),
            );

            // Exponential gain decay across taps with random polarity.
            let decay = (-3.0 * t).exp();
            let pol_l = if rng.next_u32() & 1 == 1 { 1.0 } else { -1.0 };
            let pol_r = if rng.next_u32() & 1 == 1 { 1.0 } else { -1.0 };
            gains[i] = (decay * pol_l, decay * pol_r);
        }

        let mut me = Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            base_times_ms,
            gains,
            time_scale: 1.0,
            scaled_samples: [(0.0, 0.0); ER_TAPS],
            scaled_ms: [(0.0, 0.0); ER_TAPS],
            fade_from: [(0.0, 0.0); ER_TAPS],
            fade_left: 0,
            fade_total: ((ER_FADE_MS * 0.001 * sample_rate) as u32).max(1),
            pending_scale: None,
            requested_scale: 1.0,
            primed: false,
            level: 0.4,
            sample_rate,
        };
        me.recompute_scaled();
        me
    }

    fn recompute_scaled(&mut self) {
        for i in 0..ER_TAPS {
            let ms = (
                self.base_times_ms[i].0 * self.time_scale,
                self.base_times_ms[i].1 * self.time_scale,
            );
            self.scaled_ms[i] = ms;
            self.scaled_samples[i] = (
                (ms.0 * 0.001 * self.sample_rate).max(1.0),
                (ms.1 * 0.001 * self.sample_rate).max(1.0),
            );
        }
    }

    /// Map `er_time` (0..1) to a tap-time multiplier in [ER_TIME_MIN..ER_TIME_MAX].
    ///
    /// The tap set never relocates mid-signal: a change crossfades the
    /// whole old set out while the new one fades in over `ER_FADE_MS`.
    /// Crossfading (rather than the FDN's Doppler glide) keeps the
    /// discrete early reflections pitch-stable — twelve taps gliding by
    /// different amounts would smear the room signature into a chorus.
    /// The fade is linear, not equal-power: during a sweep consecutive
    /// tap sets are nearly coincident and strongly correlated, and an
    /// equal-power law would bulge by up to +3 dB at the midpoint. A
    /// change landing mid-fade is queued (newest wins) and started when
    /// the running fade completes, so a continuous sweep resolves into
    /// back-to-back short crossfades.
    pub(super) fn set_time(&mut self, norm: f32) {
        let scale = ER_TIME_MIN + norm.clamp(0.0, 1.0) * (ER_TIME_MAX - ER_TIME_MIN);
        if (scale - self.requested_scale).abs() <= 1e-4 {
            return;
        }
        self.requested_scale = scale;
        if !self.primed {
            self.time_scale = scale;
            self.recompute_scaled();
            self.fade_left = 0;
            self.pending_scale = None;
        } else if self.fade_left > 0 {
            self.pending_scale = Some(scale);
        } else {
            self.begin_fade(scale);
        }
    }

    /// Start crossfading from the current tap set to one scaled by
    /// `scale`.
    fn begin_fade(&mut self, scale: f32) {
        self.fade_from = self.scaled_samples;
        self.time_scale = scale;
        self.recompute_scaled();
        self.fade_left = self.fade_total;
    }

    pub(super) fn set_level(&mut self, norm: f32) {
        self.level = norm.clamp(0.0, 1.0);
    }

    /// Process a single stereo sample. Returns the ER contribution
    /// *including* `level` so the caller just sums it into the wet path.
    pub(super) fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.delay_l.push(left);
        self.delay_r.push(right);
        self.primed = true;
        let mut out_l = 0.0f32;
        let mut out_r = 0.0f32;
        if self.fade_left > 0 {
            // Crossfade: both tap sets are read, old fading out while
            // new fades in. Bounded extra cost (one more tap set), no
            // allocation.
            self.fade_left -= 1;
            let x = (self.fade_total - self.fade_left) as f32 / self.fade_total as f32;
            for i in 0..ER_TAPS {
                let (ol, or) = self.fade_from[i];
                let (nl, nr) = self.scaled_samples[i];
                let (gl, gr) = self.gains[i];
                let old_l = self.delay_l.tap_linear(ol);
                let old_r = self.delay_r.tap_linear(or);
                let new_l = self.delay_l.tap_linear(nl);
                let new_r = self.delay_r.tap_linear(nr);
                out_l += (old_l + x * (new_l - old_l)) * gl;
                out_r += (old_r + x * (new_r - old_r)) * gr;
            }
            if self.fade_left == 0 {
                if let Some(scale) = self.pending_scale.take() {
                    self.begin_fade(scale);
                }
            }
        } else {
            // Settled: single tap set, bit-identical to the pre-fade
            // code.
            for i in 0..ER_TAPS {
                let (sl, sr) = self.scaled_samples[i];
                let (gl, gr) = self.gains[i];
                out_l += self.delay_l.tap_linear(sl) * gl;
                out_r += self.delay_r.tap_linear(sr) * gr;
            }
        }
        // 1/sqrt(N) keeps the summed broadband level predictable.
        let scale = self.level * (1.0 / (ER_TAPS as f32).sqrt());
        (out_l * scale, out_r * scale)
    }

    pub(super) fn clear(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        // Cancel any fade and land on the newest requested scale, so a
        // reset instance matches a fresh one that was configured once.
        self.fade_left = 0;
        self.pending_scale = None;
        if self.time_scale != self.requested_scale {
            self.time_scale = self.requested_scale;
            self.recompute_scaled();
        }
        self.primed = false;
    }
}
