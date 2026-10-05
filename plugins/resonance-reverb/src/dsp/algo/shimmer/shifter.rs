//! The Shimmer's pitch shifter: two crossfading read taps on a delay line,
//! and the bound on its energy gain that the loop's cap is built from.
//!
//! **Geometry.** A phase `ψ ∈ [0, 1)` advances by `(1 − r)/W` per sample
//! (`r` the pitch ratio, `W` the window in samples). Tap `k ∈ {0, 1}`
//! reads the input at delay `d_k = 1 + k/2 + W·φ_k`, `φ_k = frac(ψ + k/2)`,
//! with linear interpolation, and is weighted `w_k = s(tri(φ_k))`, where
//! `tri` is the unit triangle and `s(t) = t²(3 − 2t)` the smoothstep. A
//! moving delay changes pitch by exactly `r`; when a tap's delay wraps
//! (`φ_k` crosses 0) its weight is 0 and has zero slope, so the jump is
//! silent and the grain edges carry no corner. `w_0 + w_1 = 1` at every
//! sample (`tri(φ + ½) = 1 − tri(φ)`, `s(1 − t) = 1 − s(t)`).
//!
//! The half-sample offset of tap 1 and the half-sample fraction in `W`
//! keep the taps off a common integer lattice at `r = 2` and `4`, where
//! reading at an integer speed would otherwise return to the same
//! samples (see the bound below: it halves the worst case at +24).
//!
//! **Why this shifter.** Two taps and one smoothstep per sample, no FFT,
//! no grain scheduler: about 20 flops, cheap enough to run four inside a
//! 16-line loop. Its latency (1–51 ms) is only extra loop length here.
//! `resonance_dsp`'s `GrainEngine` schedules overlapping grains from a
//! capture buffer (more taps, a window table, its own buffer) to the same
//! end, and `DopplerShifter` is a single moving tap (it must jump, so it
//! clicks without a crossfade partner). The two-tap form is the classic
//! shimmer shifter (Eventide H910 lineage), and it is the one whose
//! energy gain has the closed bound below.
//!
//! **Energy bound.** With `w_0 + w_1 = 1`, Jensen gives per sample
//! `(w_0·a + w_1·b)² ≤ w_0·a² + w_1·b²`, and linear interpolation is a
//! convex combination too: `x(n + f)² ≤ (1 − f)·x[n]² + f·x[n+1]²`. So
//! for every prefix of time
//!
//! ```text
//! Σ_{t≤T} s(t)²  ≤  Σ_n c[n]·x[n]²,   c[n] = Σ_t Σ_k w_k(t)·λ_k(t, n)
//! ```
//!
//! where `λ_k(t, n)` is the interpolation weight tap `k` gives sample `n`
//! at time `t`. `c[n]` is the total weight with which input sample `n` is
//! ever read; it depends only on the geometry (`r`, `W`), not on the
//! signal. Its mean is 1 (every output sample hands out a total weight of
//! 1), but its maximum is above 1: a pitch-up tap passes the same stored
//! sample about `r` times, a pitch-down tap lingers on it `1/r` samples.
//! Hence `‖s‖ ≤ √c_max · ‖x‖` on every prefix, and [`ReadWeights`]
//! computes `c_max` for each ratio the parameter offers, by walking this
//! exact geometry (the same f32 phase recursion as [`PitchShifter`]).

use resonance_dsp::DelayLine;

/// Crossfade window, ms.
pub const WINDOW_MS: f32 = 50.0;
/// Delay of tap 0 at `φ = 0`, samples (`tap_linear` needs the newest
/// sample and the one before it).
const MIN_DELAY: f32 = 1.0;
/// Extra delay of tap 1, samples.
const TAP1_OFFSET: f32 = 0.5;

/// The `shimmer_pitch` labels, semitones, in parameter order.
pub const SEMITONES: [f32; 6] = [12.0, 7.0, 5.0, -12.0, 19.0, 24.0];

/// `W` at `sample_rate`: [`WINDOW_MS`], rounded, plus half a sample.
pub fn window_samples(sample_rate: f32) -> f32 {
    (WINDOW_MS * 0.001 * sample_rate).round() + 0.5
}

/// Pitch ratio of `semitones`.
pub fn ratio(semitones: f32) -> f32 {
    2f32.powf(semitones / 12.0)
}

/// Crossfade weight of tap 0 at phase `phi` (tap 1's is `1 −` it).
#[inline]
fn weight(phi: f32) -> f32 {
    let t = 1.0 - (2.0 * phi - 1.0).abs();
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn advance(phase: f32, step: f32) -> f32 {
    let mut p = phase + step;
    if p >= 1.0 {
        p -= 1.0;
    } else if p < 0.0 {
        p += 1.0;
    }
    p
}

/// One two-tap crossfading pitch shifter.
pub struct PitchShifter {
    line: DelayLine,
    window: f32,
    phase: f32,
    phase0: f32,
    step: f32,
}

impl PitchShifter {
    /// A shifter at ratio 1 (no shift) starting at phase `phase0`.
    pub fn new(sample_rate: f32, phase0: f32) -> Self {
        let window = window_samples(sample_rate);
        let phase0 = phase0.rem_euclid(1.0);
        Self {
            line: DelayLine::new((window + MIN_DELAY + TAP1_OFFSET) as usize + 4),
            window,
            phase: phase0,
            phase0,
            step: 0.0,
        }
    }

    /// Pitch ratio (output frequency over input frequency). Takes effect
    /// on the next sample; the taps stay where they are, so a change does
    /// not step the output.
    pub fn set_ratio(&mut self, r: f32) {
        self.step = (1.0 - r) / self.window;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.line.push(x);
        let a = self.phase;
        let b = if a < 0.5 { a + 0.5 } else { a - 0.5 };
        let wa = weight(a);
        let ya = self.line.tap_linear(MIN_DELAY + self.window * a);
        let yb = self.line.tap_linear(MIN_DELAY + TAP1_OFFSET + self.window * b);
        self.phase = advance(a, self.step);
        wa * ya + (1.0 - wa) * yb
    }

    /// Back to the freshly constructed state (ratio kept).
    pub fn clear(&mut self) {
        self.line.clear();
        self.phase = self.phase0;
    }

    /// Bytes held in the delay buffer.
    pub fn buffer_bytes(&self) -> usize {
        ((self.window + MIN_DELAY + TAP1_OFFSET) as usize + 4).next_power_of_two()
            * std::mem::size_of::<f32>()
    }
}

/// `c_max` (see the module docs) for every [`SEMITONES`] ratio at one
/// sample rate, computed once at construction.
#[derive(Clone, Copy, Debug)]
pub struct ReadWeights {
    c_max: [f32; SEMITONES.len()],
}

impl ReadWeights {
    /// Walks the geometry for each ratio over 16 sweeps of the window
    /// (allocates a scratch buffer: construction only).
    pub fn new(sample_rate: f32) -> Self {
        Self {
            c_max: SEMITONES.map(|s| max_read_weight(ratio(s), sample_rate)),
        }
    }

    /// `c_max` for `semitones`: the computed value for a label, else the
    /// geometric worst case `2·(max(r, 1/r) + 2)` (each tap reaches a
    /// sample at most `⌈r⌉ + 1` times going up, with interpolation weight
    /// at most 1 each; going down at most once, with weight at most
    /// `1/r + 1`).
    pub fn c_max(&self, semitones: f32) -> f32 {
        match SEMITONES.iter().position(|&s| s == semitones) {
            Some(i) => self.c_max[i],
            None => {
                let r = ratio(semitones);
                2.0 * (r.max(1.0 / r) + 2.0)
            }
        }
    }
}

/// The largest total read weight any input sample receives at ratio `r`.
pub fn max_read_weight(r: f32, sample_rate: f32) -> f32 {
    let window = window_samples(sample_rate);
    let step = (1.0 - r) / window;
    let sweeps = 16.0 / (1.0 - r).abs().max(0.05);
    let len = (sweeps * window) as usize + 2 * window as usize;
    let span = window as usize + 4;
    let mut c = vec![0.0f64; len + span];
    let mut phase = 0.0f32;
    // `t` counts pushes: after push `t`, `tap(j)` is input sample `t − j`.
    for t in 0..len + span {
        let a = phase;
        let b = if a < 0.5 { a + 0.5 } else { a - 0.5 };
        let wa = weight(a) as f64;
        for (w, d) in [
            (wa, MIN_DELAY + window * a),
            (1.0 - wa, MIN_DELAY + TAP1_OFFSET + window * b),
        ] {
            let di = d as usize;
            let f = (d - di as f32) as f64;
            if t >= di + 1 {
                c[t - di] += w * (1.0 - f);
                c[t - di - 1] += w * f;
            }
        }
        phase = advance(a, step);
    }
    // Samples from `len` on have not had all their reads yet.
    c[..len].iter().copied().fold(0.0, f64::max) as f32
}
