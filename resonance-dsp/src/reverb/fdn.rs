//! Feedback delay network with per-line absorption (Jot & Chaigne), an
//! orthogonal feedback matrix and per-line random modulation.
//!
//! One sample of [`Fdn::tick`]:
//!
//! ```text
//!   y_i  = absorption_i( line_i read at len_i − 1 + mod_i )
//!   out  = y                          (returned: per-line outputs)
//!   line_i ← (M·y)_i + input_i        (input ignored while frozen)
//! ```
//!
//! - **Lengths.** `N` base lengths are drawn once from the seed,
//!   log-uniformly spread over `[min_ms, max_ms]` (one per stratum, jittered
//!   by ±30 % of a stratum). [`Fdn::set_size`] scales them and snaps each to
//!   the next unused prime, in ascending order, so the lengths are distinct
//!   primes, hence pairwise coprime, at every size. The search is bounded
//!   (prime gaps below 10⁵ are < 72), allocation free and cheap enough for
//!   a parameter change on the audio thread.
//! - **Size on running audio.** By default the read heads jump to the new
//!   lengths (an audible discontinuity: an engine crossfades or steps size
//!   while silent). With [`Fdn::set_glide`] > 0 the heads slew towards the
//!   new lengths at that many samples per sample instead: a short Doppler
//!   pitch bend, no click. Gains and absorption follow the new lengths at
//!   once either way.
//! - **Decay.** Each line's [`Absorption`] is designed from its own length,
//!   so every line loses 60 dB in exactly T60 per band (fixes L1/L2).
//! - **Modulation.** Each line has its own [`SmoothRandom`] (seed + line
//!   index), moving its read by up to `depth` samples through an allpass
//!   interpolator ([`super::allpass_read`]: flat magnitude, so modulation
//!   does not shorten the treble decay the way a linear read would). With
//!   depth 0 the read is an integer tap. A moving read is not exactly
//!   lossless (the interpolator's transients, Doppler): at 4–16 samples
//!   depth and 0.7 Hz the broadband T60 comes out 1–5 % short. Cubic
//!   Hermite measured 7–12 % short, linear worse.
//! - **Freeze.** [`Fdn::set_freeze`] makes the loop lossless *and mutes the
//!   input itself* (callers need not gate it). The modulation fades out
//!   over 50 ms so the reads land on integer taps: the allpass read is
//!   lossless only for a fixed fraction, and a frozen tail must hold its
//!   energy for minutes.
//!
//! All buffers are allocated in [`Fdn::new`]; nothing after that allocates.

use super::{allpass_read, Absorption, DecayBands, MatrixKind, SmoothRandom};
use crate::{DelayLine, SimpleRng};

/// Construction-time settings of an [`Fdn`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FdnConfig {
    /// Shortest and longest line at size 1.0, milliseconds.
    pub min_ms: f32,
    pub max_ms: f32,
    /// The largest `size` [`Fdn::set_size`] accepts (buffers are sized
    /// for it).
    pub max_size: f32,
    /// The largest modulation depth, in samples.
    pub max_mod_depth: f32,
    pub seed: u64,
    /// `None` picks [`MatrixKind::default_for`] N.
    pub matrix: Option<MatrixKind>,
}

impl FdnConfig {
    pub fn new(min_ms: f32, max_ms: f32) -> Self {
        Self {
            min_ms,
            max_ms,
            max_size: 2.0,
            max_mod_depth: 32.0,
            seed: 0x5EED_F00D,
            matrix: None,
        }
    }
}

const MIN_SIZE: f32 = 0.05;

pub struct Fdn<const N: usize> {
    sample_rate: f32,
    lines: [DelayLine; N],
    base_ms: [f32; N],
    len: [usize; N],
    read: [f32; N],
    interp: [f32; N],
    glide: f32,
    max_len: usize,
    size: f32,
    bands: DecayBands,
    absorb: [Absorption; N],
    mods: [SmoothRandom; N],
    mod_depth: f32,
    max_mod_depth: f32,
    mod_scale: f32,
    mod_step: f32,
    matrix: MatrixKind,
    freeze: bool,
    out: [f32; N],
}

impl<const N: usize> Fdn<N> {
    /// Allocates every line for `config.max_size`; starts at size 1.0,
    /// T60 2 s flat, no modulation, not frozen.
    pub fn new(sample_rate: f32, config: FdnConfig) -> Self {
        assert!(N >= 2, "an FDN needs at least two lines");
        let matrix = config.matrix.unwrap_or(MatrixKind::default_for(N));
        assert!(
            matrix != MatrixKind::Hadamard || N.is_power_of_two(),
            "Hadamard feedback needs a power-of-two line count, got {N}"
        );
        let (lo, hi) = (config.min_ms.max(0.1), config.max_ms.max(config.min_ms.max(0.1)));
        let mut rng = SimpleRng::new(config.seed);
        let base_ms = std::array::from_fn(|i| {
            let u = rng.next_u32() as f32 / u32::MAX as f32;
            let pos = if N > 1 {
                (i as f32 + 0.6 * (u - 0.5)) / (N - 1) as f32
            } else {
                0.0
            };
            lo * (hi / lo).powf(pos.clamp(0.0, 1.0))
        });
        let max_size = config.max_size.max(MIN_SIZE);
        let max_mod_depth = config.max_mod_depth.max(0.0);
        let longest = (hi * max_size * sample_rate / 1000.0).ceil() as usize;
        // Headroom for the prime search (N strictly increasing primes past
        // the longest target) and the modulation swing.
        let max_len = longest + 80 * N + max_mod_depth.ceil() as usize + 4;
        let bands = DecayBands::flat(2.0);
        let mut fdn = Self {
            sample_rate,
            lines: std::array::from_fn(|_| DelayLine::new(max_len + 2)),
            base_ms,
            len: [0; N],
            read: [0.0; N],
            interp: [0.0; N],
            glide: 0.0,
            max_len,
            size: 1.0,
            bands,
            absorb: [Absorption::lossless(); N],
            mods: std::array::from_fn(|i| {
                SmoothRandom::new(
                    config.seed.wrapping_add(0x9E37_79B9 * (i as u64 + 1)),
                    0.5,
                    0.0,
                    sample_rate,
                )
            }),
            mod_depth: 0.0,
            max_mod_depth,
            mod_scale: 1.0,
            mod_step: 1.0 / (0.05 * sample_rate).max(1.0),
            matrix,
            freeze: false,
            out: [0.0; N],
        };
        // Glide is 0 here, so this also puts the read heads on the lengths.
        fdn.set_size(1.0);
        fdn
    }

    /// Scale the line lengths (see the module docs for what this does to
    /// running audio). Clamped to `[0.05, max_size]`.
    pub fn set_size(&mut self, size: f32) {
        let max_size = self.max_size();
        self.size = size.clamp(MIN_SIZE, max_size);
        let floor = self.max_mod_depth.ceil() as usize + 2;
        let ceiling = self.max_len - self.max_mod_depth.ceil() as usize - 2;
        let mut prev = 0usize;
        for i in 0..N {
            let target = (self.base_ms[i] * self.size * self.sample_rate / 1000.0).round() as usize;
            let p = next_prime(target.max(prev + 1).max(floor));
            // The buffers carry 80 samples of prime-search headroom per line.
            debug_assert!(p <= ceiling, "line {i}: prime {p} past the buffer ({ceiling})");
            let p = p.min(ceiling);
            self.len[i] = p;
            prev = p;
        }
        if self.glide <= 0.0 {
            self.read = self.len.map(|l| l as f32);
        }
        self.redesign();
    }

    /// Read-head slew for size changes, samples per sample (0 = jump).
    /// Turning the glide off mid-slew jumps the heads to their targets
    /// (a zero slew would otherwise strand them short of the lengths the
    /// absorption is designed for).
    pub fn set_glide(&mut self, samples_per_sample: f32) {
        self.glide = samples_per_sample.max(0.0);
        if self.glide <= 0.0 {
            self.read = self.len.map(|l| l as f32);
        }
    }

    /// The three-band decay target.
    pub fn set_decay(&mut self, bands: DecayBands) {
        self.bands = bands;
        self.redesign();
    }

    /// Random modulation: `rate_hz` new targets per second, `depth_samples`
    /// peak read offset (clamped to the configured maximum).
    pub fn set_modulation(&mut self, rate_hz: f32, depth_samples: f32) {
        self.mod_depth = depth_samples.clamp(0.0, self.max_mod_depth);
        for (i, m) in self.mods.iter_mut().enumerate() {
            // A few per cent apart so the lines never move in step.
            m.set_rate(rate_hz * (1.0 + 0.07 * i as f32 / N as f32), self.sample_rate);
            m.set_depth(self.mod_depth);
        }
    }

    /// Lossless loop with the input muted (see the module docs).
    pub fn set_freeze(&mut self, on: bool) {
        self.freeze = on;
        self.redesign();
    }

    pub fn is_frozen(&self) -> bool {
        self.freeze
    }

    pub fn size(&self) -> f32 {
        self.size
    }

    pub fn max_size(&self) -> f32 {
        let longest_ms = self.base_ms.iter().cloned().fold(0.0, f32::max);
        let budget = self.max_len - 80 * N - self.max_mod_depth.ceil() as usize - 4;
        budget as f32 / (longest_ms * self.sample_rate / 1000.0)
    }

    pub fn decay(&self) -> DecayBands {
        self.bands
    }

    /// Current (target) line lengths in samples: distinct primes, ascending.
    pub fn line_lengths(&self) -> [usize; N] {
        self.len
    }

    /// Each line's absorption filter, as designed for its length.
    pub fn absorption(&self, line: usize) -> &Absorption {
        &self.absorb[line]
    }

    /// The per-line outputs of the last [`Fdn::tick`].
    pub fn outputs(&self) -> &[f32; N] {
        &self.out
    }

    /// One sample: `input[i]` is added to line `i`'s write; returns the
    /// per-line (absorbed) outputs.
    #[inline]
    pub fn tick(&mut self, input: &[f32; N]) -> &[f32; N] {
        let target_scale = if self.freeze { 0.0 } else { 1.0 };
        if self.mod_scale != target_scale {
            self.mod_scale = if self.mod_scale < target_scale {
                (self.mod_scale + self.mod_step).min(1.0)
            } else {
                (self.mod_scale - self.mod_step).max(0.0)
            };
        }
        let modulated = self.mod_depth > 0.0 && self.mod_scale > 0.0;
        let mut y = [0.0f32; N];
        for i in 0..N {
            let target = self.len[i] as f32;
            if self.read[i] != target {
                let d = target - self.read[i];
                self.read[i] += d.clamp(-self.glide, self.glide);
            }
            let raw = if modulated {
                let m = self.mods[i].next_sample() * self.mod_scale;
                allpass_read(&self.lines[i], self.read[i] - 1.0 + m, &mut self.interp[i])
            } else if self.read[i] == target {
                let s = self.lines[i].tap(self.len[i] - 1);
                self.interp[i] = s;
                s
            } else {
                allpass_read(&self.lines[i], self.read[i] - 1.0, &mut self.interp[i])
            };
            y[i] = self.absorb[i].process(raw);
        }
        self.out = y;
        self.matrix.apply(&mut y);
        let gate = if self.freeze { 0.0 } else { 1.0 };
        for i in 0..N {
            self.lines[i].push(y[i] + gate * input[i]);
        }
        &self.out
    }

    /// Back to the freshly constructed state for the current settings:
    /// lines, filters, modulators and outputs cleared, read heads at their
    /// targets, modulation fade complete.
    pub fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        for a in &mut self.absorb {
            a.clear();
        }
        for m in &mut self.mods {
            m.reset();
        }
        self.read = self.len.map(|l| l as f32);
        self.interp = [0.0; N];
        self.mod_scale = if self.freeze { 0.0 } else { 1.0 };
        self.out = [0.0; N];
    }

    /// Sum of squares of every sample currently in flight (the last
    /// `len_i` writes of each line). A lossless, unmodulated network keeps
    /// it constant. Diagnostic: O(total length), not for the audio path.
    pub fn stored_energy(&self) -> f64 {
        let mut e = 0.0f64;
        for (line, &len) in self.lines.iter().zip(&self.len) {
            for k in 0..len {
                let v = line.tap(k) as f64;
                e += v * v;
            }
        }
        e
    }

    fn redesign(&mut self) {
        for i in 0..N {
            if self.freeze {
                self.absorb[i].set_lossless();
            } else {
                self.absorb[i].design(&self.bands, self.len[i] as f32, self.sample_rate);
            }
        }
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n.is_multiple_of(2) {
        return n == 2;
    }
    let mut d = 3;
    while d * d <= n {
        if n.is_multiple_of(d) {
            return false;
        }
        d += 2;
    }
    true
}

/// The smallest prime `≥ n`.
pub fn next_prime(n: usize) -> usize {
    let mut p = n.max(2);
    while !is_prime(p) {
        p += 1;
    }
    p
}
