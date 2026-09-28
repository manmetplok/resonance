//! De-harsh resonance suppressor: many narrow, time-varying cuts on
//! peaks that stand out of a smoothed spectrum (the soothe-style
//! "resonance suppressor", `docs/design/deharsh-resonance-suppressor.md`).
//!
//! Structure, per the design note:
//!
//! * **STFT gain mask.** A periodic Hann window of [`StftGeometry::frame`]
//!   samples (2048 at ≤ 48 kHz), hop `frame / 8`, Hann synthesis window,
//!   weighted overlap-add. Both channels ride one complex FFT
//!   (`z = a + j·b`) and are unpacked by Hermitian symmetry.
//! * **Detection spectrum `D`.** Per-bin power averaged over a
//!   (near-)triangular kernel whose full width at half maximum is the cut
//!   width `f / Q` (at least 3 bins), then integrated over
//!   [`DETECTOR_TAU_MS`].
//! * **Reference `R`.** A peak-excluded, log-frequency-uniform (bins
//!   weighted `1/f`) one-octave moving average of `D` in dB: average,
//!   clip `D` to it, average again.
//! * **Cut.** `E = D − R` above the selectivity `T`, through a fixed
//!   [`KNEE_DB`] soft knee with infinite ratio, capped at `depth`,
//!   weighted by the band taper, smoothed per bin with attack/release at
//!   the hop rate. A steady peak `E` dB above its reference leaves at
//!   `max(T, E − depth)` dB above it.
//!
//! **Latency is exactly one frame** ([`ResonanceSuppressor::latency`]),
//! whatever the config: off, depth 0, mix 0, delta and every mode report
//! and deliver the same delay. Off is the input delayed by exactly that
//! many samples, bit-exact; the STFT keeps running underneath so turning
//! it on crossfades onto a warm path.
//!
//! Allocation discipline: everything is sized in [`ResonanceSuppressor::new`];
//! `process_*` never allocates (rustfft runs `process_with_scratch`).

use std::f32::consts::{LN_10, PI, SQRT_2};
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::stereo::{ms_decode, ms_encode};

/// Frame length at ≤ 48 kHz. Doubled per octave of rate above that.
pub const BASE_FRAME: usize = 2048;
/// Hop = frame / this (87.5 % overlap).
pub const HOP_DIVISOR: usize = 8;
/// Largest rate multiple the geometry scales to (8 × 48 kHz).
const MAX_GEOMETRY_SCALE: usize = 8;

/// Fixed soft-knee width of the gain law, dB.
pub const KNEE_DB: f32 = 4.0;
/// Fixed time integration of the detection spectrum, ms.
pub const DETECTOR_TAU_MS: f32 = 10.0;
/// Reference level below which nothing is cut, in dB relative to the
/// detection power of a full-scale sine.
pub const FLOOR_DB: f32 = -100.0;
/// Crossfade length of an on/off toggle, and the ramp time of `mix`.
pub const XFADE_SECONDS: f32 = 0.010;
/// Narrowest detection kernel (and so cut), in bins.
const MIN_KERNEL_BINS: f32 = 3.0;
/// Width of the band-edge taper outside `low_hz` / `high_hz`, octaves.
const BAND_TAPER_OCT: f32 = 1.0 / 6.0;
/// A smoothed cut below this snaps to exactly 0 (gain exactly 1).
const CUT_EPS_DB: f32 = 1e-4;

/// Param ranges, for hosts that expose the config as params.
pub const DEPTH_DB_RANGE: (f32, f32) = (0.0, 24.0);
pub const SELECTIVITY_DB_RANGE: (f32, f32) = (0.0, 18.0);
pub const SHARPNESS_Q_RANGE: (f32, f32) = (3.0, 24.0);
pub const ATTACK_MS_RANGE: (f32, f32) = (5.0, 200.0);
pub const RELEASE_MS_RANGE: (f32, f32) = (20.0, 1000.0);
pub const FREQ_HZ_RANGE: (f32, f32) = (200.0, 20_000.0);

/// Frame/hop of the suppressor's STFT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StftGeometry {
    pub frame: usize,
    pub hop: usize,
}

impl StftGeometry {
    /// The base frame up to 48 kHz, doubled per octave of rate above it
    /// (4096 at 96 kHz, 8192 at 192 kHz), capped at 8× — the same rule
    /// as the mastering FIR geometry, so resolution in Hz and latency in
    /// ms stay put.
    pub fn for_sample_rate(sample_rate: f32) -> Self {
        let ratio = (sample_rate / 48_000.0).max(1.0);
        let scale = (ratio.ceil() as usize)
            .next_power_of_two()
            .min(MAX_GEOMETRY_SCALE);
        let frame = BASE_FRAME * scale;
        Self {
            frame,
            hop: frame / HOP_DIVISOR,
        }
    }

    /// Algorithmic latency: exactly one frame.
    pub const fn latency(&self) -> usize {
        self.frame
    }
}

/// Which signals are measured and cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuppressorMode {
    /// L/R with linked detection: one gain curve for both channels.
    #[default]
    Stereo,
    /// Mid only; the side passes with unity gains.
    Mid,
    /// Side only; the mid passes with unity gains.
    Side,
    /// Mid and side, each detected and cut independently.
    MidSide,
}

impl SuppressorMode {
    pub const NAMES: [&'static str; 4] = ["Stereo", "Mid", "Side", "Mid+Side"];

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Mid,
            2 => Self::Side,
            3 => Self::MidSide,
            _ => Self::Stereo,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Stereo => 0,
            Self::Mid => 1,
            Self::Side => 2,
            Self::MidSide => 3,
        }
    }
}

/// Everything the suppressor reads, once per block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuppressorConfig {
    /// Off: the output is the input delayed by [`ResonanceSuppressor::latency`].
    pub enabled: bool,
    /// Cap on any single bin's cut, dB.
    pub depth_db: f32,
    /// How far above the reference a peak must stand before it is cut, dB.
    pub selectivity_db: f32,
    /// Q of each cut: the detection kernel's FWHM is `f / Q` Hz.
    pub sharpness_q: f32,
    /// Time constant for a cut to deepen, ms.
    pub attack_ms: f32,
    /// Time constant for a cut to recover, ms.
    pub release_ms: f32,
    /// Band in which cuts may happen (swapped if reversed).
    pub low_hz: f32,
    pub high_hz: f32,
    pub mode: SuppressorMode,
    /// Linear wet/dry, latency-aligned; 1 = fully processed.
    pub mix: f32,
    /// Output what is removed (`dry − out`) instead.
    pub delta: bool,
}

impl Default for SuppressorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            depth_db: 6.0,
            // Q 24 / T 5 is the exit-criterion configuration (W12 T1/T2):
            // a Q 10 resonance +15 dB is cut 7.8 dB while broadband noise
            // moves < 0.2 dB. `defaults_meet_the_exit_criterion` pins it.
            selectivity_db: 5.0,
            sharpness_q: 24.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            low_hz: 1000.0,
            high_hz: 8000.0,
            mode: SuppressorMode::Stereo,
            mix: 1.0,
            delta: false,
        }
    }
}

fn clamp_or(v: f32, (lo, hi): (f32, f32), fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

impl SuppressorConfig {
    /// Clamp every field into range; non-finite values take the default.
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let mut low = clamp_or(self.low_hz, FREQ_HZ_RANGE, d.low_hz);
        let mut high = clamp_or(self.high_hz, FREQ_HZ_RANGE, d.high_hz);
        if low > high {
            std::mem::swap(&mut low, &mut high);
        }
        Self {
            enabled: self.enabled,
            depth_db: clamp_or(self.depth_db, DEPTH_DB_RANGE, d.depth_db),
            selectivity_db: clamp_or(self.selectivity_db, SELECTIVITY_DB_RANGE, d.selectivity_db),
            sharpness_q: clamp_or(self.sharpness_q, SHARPNESS_Q_RANGE, d.sharpness_q),
            attack_ms: clamp_or(self.attack_ms, ATTACK_MS_RANGE, d.attack_ms),
            release_ms: clamp_or(self.release_ms, RELEASE_MS_RANGE, d.release_ms),
            low_hz: low,
            high_hz: high,
            mode: self.mode,
            mix: clamp_or(self.mix, (0.0, 1.0), d.mix),
            delta: self.delta,
        }
    }
}

/// Per-signal detector state: the time-smoothed detection power, its dB
/// value, and the smoothed cut per bin.
struct Detector {
    smooth: Vec<f32>,
    d_db: Vec<f32>,
    cut: Vec<f32>,
    active: bool,
}

impl Detector {
    fn new(bins: usize) -> Self {
        Self {
            smooth: vec![0.0; bins],
            d_db: vec![0.0; bins],
            cut: vec![0.0; bins],
            active: false,
        }
    }

    fn reset(&mut self) {
        self.smooth.fill(0.0);
        self.d_db.fill(0.0);
        self.cut.fill(0.0);
        self.active = false;
    }
}

/// Per-hop constants derived from the config.
#[derive(Clone, Copy)]
struct HopCoefs {
    depth_db: f32,
    selectivity_db: f32,
    attack: f32,
    release: f32,
}

/// Geometry-only tables and scratch shared by the detectors.
struct Analysis {
    /// Number of bins `0..=frame/2`.
    bins: usize,
    /// Boxcar half-width per bin for the current Q.
    kernel_half: Vec<u32>,
    cached_q: f32,
    /// One-octave reference window per bin, inclusive bin bounds.
    ref_lo: Vec<u32>,
    ref_hi: Vec<u32>,
    /// Prefix sum of the `1/k` reference weights (index `k + 1` covers
    /// bins `..=k`; DC weighs 0).
    weight_prefix: Vec<f64>,
    /// Band weight per bin for the current band.
    band: Vec<f32>,
    cached_band: (f32, f32),
    /// Scratch.
    prefix: Vec<f64>,
    box1: Vec<f32>,
    r1: Vec<f32>,
    clipped: Vec<f32>,
    /// Detector integration coefficient (per hop).
    det_alpha: f32,
    /// [`FLOOR_DB`] as an absolute detection level.
    floor_db: f32,
    bin_hz: f32,
}

impl Analysis {
    fn new(geometry: StftGeometry, sample_rate: f32) -> Self {
        let bins = geometry.frame / 2 + 1;
        let half = bins - 1;
        let mut ref_lo = vec![0u32; bins];
        let mut ref_hi = vec![0u32; bins];
        for k in 1..bins {
            let kf = k as f32;
            ref_lo[k] = ((kf / SQRT_2).ceil() as usize).clamp(1, half) as u32;
            ref_hi[k] = ((kf * SQRT_2).floor() as usize).clamp(1, half) as u32;
        }
        let mut weight_prefix = vec![0.0f64; bins + 1];
        for k in 0..bins {
            let w = if k == 0 { 0.0 } else { 1.0 / k as f64 };
            weight_prefix[k + 1] = weight_prefix[k] + w;
        }
        let hop_s = geometry.hop as f32 / sample_rate;
        // A full-scale sine's windowed bin magnitude is frame/4.
        let full_scale_db = 20.0 * (geometry.frame as f32 / 4.0).log10();
        let mut a = Self {
            bins,
            kernel_half: vec![1; bins],
            cached_q: f32::NAN,
            ref_lo,
            ref_hi,
            weight_prefix,
            band: vec![0.0; bins],
            cached_band: (f32::NAN, f32::NAN),
            prefix: vec![0.0; bins + 1],
            box1: vec![0.0; bins],
            r1: vec![0.0; bins],
            clipped: vec![0.0; bins],
            det_alpha: 1.0 - (-hop_s / (DETECTOR_TAU_MS * 1e-3)).exp(),
            floor_db: full_scale_db + FLOOR_DB,
            bin_hz: sample_rate / geometry.frame as f32,
        };
        let d = SuppressorConfig::default();
        a.update_tables(&d);
        a
    }

    fn update_tables(&mut self, cfg: &SuppressorConfig) {
        if cfg.sharpness_q != self.cached_q {
            self.cached_q = cfg.sharpness_q;
            for k in 0..self.bins {
                let width = (k as f32 / cfg.sharpness_q).max(MIN_KERNEL_BINS);
                self.kernel_half[k] = ((width - 1.0) * 0.5).round().max(1.0) as u32;
            }
        }
        let band = (cfg.low_hz, cfg.high_hz);
        if band != self.cached_band {
            self.cached_band = band;
            for k in 0..self.bins {
                self.band[k] = if k == 0 {
                    0.0
                } else {
                    band_weight(k as f32 * self.bin_hz, band.0, band.1)
                };
            }
        }
    }

    /// Boxcar mean of `src` over each bin's kernel into `dst`.
    fn boxcar(prefix: &mut [f64], kernel_half: &[u32], src: &[f32], dst: &mut [f32]) {
        let bins = src.len();
        let half = bins - 1;
        prefix[0] = 0.0;
        for k in 0..bins {
            prefix[k + 1] = prefix[k] + src[k] as f64;
        }
        for k in 1..bins {
            let h = kernel_half[k] as usize;
            let lo = k.saturating_sub(h).max(1);
            let hi = (k + h).min(half);
            dst[k] = ((prefix[hi + 1] - prefix[lo]) / (hi + 1 - lo) as f64) as f32;
        }
        dst[0] = src[0];
    }

    /// `1/k`-weighted one-octave mean of `src` into `dst`.
    fn log_mean(&mut self, src_is_clipped: bool, dst_is_r1: bool, d_db: &[f32]) {
        let src: &[f32] = if src_is_clipped { &self.clipped } else { d_db };
        let bins = self.bins;
        self.prefix[0] = 0.0;
        self.prefix[1] = 0.0;
        for k in 1..bins {
            self.prefix[k + 1] = self.prefix[k] + src[k] as f64 / k as f64;
        }
        let dst: &mut [f32] = if dst_is_r1 { &mut self.r1 } else { &mut self.box1 };
        for k in 1..bins {
            let lo = self.ref_lo[k] as usize;
            let hi = self.ref_hi[k] as usize;
            let w = self.weight_prefix[hi + 1] - self.weight_prefix[lo];
            dst[k] = ((self.prefix[hi + 1] - self.prefix[lo]) / w) as f32;
        }
    }

    /// Run one detector on `power` (bins `0..=frame/2`) and advance its
    /// cuts. `box1` doubles as scratch and ends holding the reference.
    fn detect(&mut self, det: &mut Detector, power: &[f32], coefs: HopCoefs) {
        let bins = self.bins;
        // A NaN or infinity in the frame would poison the smoothed power
        // for good, and a NaN level reads as over the reference at full
        // depth: one bad sample latched a full cut. Drop the state and
        // cut nothing until the frame is clean; the next clean frame
        // restarts the detector from its own measurement.
        if !power.iter().all(|p| p.is_finite()) {
            det.reset();
            return;
        }
        if !det.active {
            // First frame after (re)activation: start the integrator at
            // the current measurement instead of ramping up from zero.
            det.active = true;
            Self::boxcar(&mut self.prefix, &self.kernel_half, power, &mut self.box1);
            Self::boxcar(&mut self.prefix, &self.kernel_half, &self.box1, &mut self.clipped);
            det.smooth.copy_from_slice(&self.clipped);
        } else {
            Self::boxcar(&mut self.prefix, &self.kernel_half, power, &mut self.box1);
            Self::boxcar(&mut self.prefix, &self.kernel_half, &self.box1, &mut self.clipped);
            let a = self.det_alpha;
            for k in 0..bins {
                det.smooth[k] += a * (self.clipped[k] - det.smooth[k]);
            }
        }
        for k in 0..bins {
            det.d_db[k] = 10.0 * (det.smooth[k] + 1e-30).log10();
        }
        // Reference: plain one-octave mean (r1), then the mean of D
        // clipped to it (box1).
        self.log_mean(false, true, &det.d_db);
        for k in 1..bins {
            self.clipped[k] = det.d_db[k].min(self.r1[k]);
        }
        self.log_mean(true, false, &det.d_db);

        let knee_half = 0.5 * KNEE_DB;
        for k in 1..bins {
            let w = self.band[k];
            let reference = self.box1[k];
            let target = if w == 0.0 || reference < self.floor_db {
                0.0
            } else {
                let x = det.d_db[k] - reference - coefs.selectivity_db;
                let over = if x <= -knee_half {
                    0.0
                } else if x < knee_half {
                    (x + knee_half) * (x + knee_half) / (2.0 * KNEE_DB)
                } else {
                    x
                };
                w * over.min(coefs.depth_db)
            };
            let c = det.cut[k];
            let coef = if target > c { coefs.attack } else { coefs.release };
            let mut next = target + (c - target) * coef;
            if next < CUT_EPS_DB {
                next = 0.0;
            }
            det.cut[k] = next;
        }
        det.cut[0] = 0.0;
    }
}

/// Band weight: 1 inside `[low, high]`, a raised-cosine fall to 0 over
/// [`BAND_TAPER_OCT`] outside each edge.
fn band_weight(f: f32, low: f32, high: f32) -> f32 {
    let taper = |oct: f32| {
        if oct >= BAND_TAPER_OCT {
            0.0
        } else {
            0.5 + 0.5 * (PI * oct / BAND_TAPER_OCT).cos()
        }
    };
    if f < low {
        taper((low / f).log2())
    } else if f > high {
        taper((f / high).log2())
    } else {
        1.0
    }
}

#[inline]
fn cut_to_gain(cut_db: f32) -> f32 {
    if cut_db == 0.0 {
        1.0
    } else {
        (-cut_db * (LN_10 / 20.0)).exp()
    }
}

/// The resonance suppressor. See the module docs.
pub struct ResonanceSuppressor {
    geometry: StftGeometry,
    sample_rate: f32,
    mask: usize,
    /// Periodic Hann, used for analysis and synthesis.
    window: Vec<f32>,
    /// `1 / (frame · Σw²)`: undoes rustfft's unnormalised inverse and
    /// the WOLA window-square sum.
    out_scale: f32,
    /// Last `frame` input samples (L/R), circular at `pos`.
    hist_l: Vec<f32>,
    hist_r: Vec<f32>,
    /// Overlap-add accumulators (L/R), circular at `pos`.
    acc_l: Vec<f32>,
    acc_r: Vec<f32>,
    pos: usize,
    /// Samples until the next frame runs.
    countdown: usize,
    phase_offset: usize,

    buf: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    fft_forward: Arc<dyn Fft<f32> + Send + Sync>,
    fft_inverse: Arc<dyn Fft<f32> + Send + Sync>,

    power_a: Vec<f32>,
    power_b: Vec<f32>,
    gain_a: Vec<f32>,
    gain_b: Vec<f32>,
    det_a: Detector,
    det_b: Detector,
    analysis: Analysis,

    /// Config the current block runs with (sanitised).
    cfg: SuppressorConfig,
    coefs: HopCoefs,
    /// On/off crossfade position, 0 = dry delay, 1 = processed.
    fade: f32,
    /// Smoothed `mix`.
    mix: f32,
    /// Per-sample step of `fade` and `mix`.
    ramp_step: f32,
    primed: bool,
    max_cut_db: f32,
}

impl ResonanceSuppressor {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let geometry = StftGeometry::for_sample_rate(sample_rate);
        let n = geometry.frame;
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos() as f32)
            .collect();
        // Σ w²[i + m·hop] is the same for every i (COLA for Hann² at
        // hop ≤ N/4); take it from the table rather than the formula.
        let mut cola = 0.0f64;
        let mut i = 0;
        while i < n {
            cola += (window[i] as f64).powi(2);
            i += geometry.hop;
        }
        let mut planner = FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(n);
        let fft_inverse = planner.plan_fft_inverse(n);
        let scratch_len = fft_forward
            .get_inplace_scratch_len()
            .max(fft_inverse.get_inplace_scratch_len());
        let bins = n / 2 + 1;
        let mut s = Self {
            geometry,
            sample_rate,
            mask: n - 1,
            window,
            out_scale: (1.0 / (n as f64 * cola)) as f32,
            hist_l: vec![0.0; n],
            hist_r: vec![0.0; n],
            acc_l: vec![0.0; n],
            acc_r: vec![0.0; n],
            pos: 0,
            countdown: geometry.hop,
            phase_offset: 0,
            buf: vec![Complex::new(0.0, 0.0); n],
            fft_scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            fft_forward,
            fft_inverse,
            power_a: vec![0.0; bins],
            power_b: vec![0.0; bins],
            gain_a: vec![1.0; bins],
            gain_b: vec![1.0; bins],
            det_a: Detector::new(bins),
            det_b: Detector::new(bins),
            analysis: Analysis::new(geometry, sample_rate),
            cfg: SuppressorConfig::default(),
            coefs: HopCoefs {
                depth_db: 0.0,
                selectivity_db: 0.0,
                attack: 0.0,
                release: 0.0,
            },
            fade: 0.0,
            mix: 1.0,
            ramp_step: 1.0 / (XFADE_SECONDS * sample_rate).max(1.0),
            primed: false,
            max_cut_db: 0.0,
        };
        s.set_config(&SuppressorConfig::default());
        s
    }

    pub fn geometry(&self) -> StftGeometry {
        self.geometry
    }

    /// Latency in samples: one frame, for every config.
    pub fn latency(&self) -> usize {
        self.geometry.latency()
    }

    /// Centre frequency of bin `k` of [`Self::cut_db`], Hz.
    pub fn bin_hz(&self, k: usize) -> f32 {
        k as f32 * self.sample_rate / self.geometry.frame as f32
    }

    /// Deepest current cut over all bins and channels, dB (≥ 0).
    pub fn max_cut_db(&self) -> f32 {
        self.max_cut_db
    }

    /// Current per-bin cut in dB (bins `0..=frame/2`). Channel 0 is the
    /// linked L/R curve or the mid, channel 1 the side.
    pub fn cut_db(&self, channel: usize) -> &[f32] {
        if channel == 0 {
            &self.det_a.cut
        } else {
            &self.det_b.cut
        }
    }

    /// Move when the FFT runs within the hop, without changing the
    /// latency (DSP-16 stagger). Takes effect from the next [`Self::reset`]
    /// (and is applied now if nothing has been processed yet).
    pub fn set_phase_offset(&mut self, offset: usize) {
        self.phase_offset = offset % self.geometry.hop;
        if !self.primed {
            self.countdown = self.geometry.hop - self.phase_offset;
        }
    }

    pub fn reset(&mut self) {
        self.hist_l.fill(0.0);
        self.hist_r.fill(0.0);
        self.acc_l.fill(0.0);
        self.acc_r.fill(0.0);
        self.pos = 0;
        self.countdown = self.geometry.hop - self.phase_offset;
        self.det_a.reset();
        self.det_b.reset();
        self.gain_a.fill(1.0);
        self.gain_b.fill(1.0);
        self.primed = false;
        self.max_cut_db = 0.0;
    }

    fn set_config(&mut self, cfg: &SuppressorConfig) {
        let cfg = cfg.sanitized();
        self.analysis.update_tables(&cfg);
        let hop_ms = self.geometry.hop as f32 * 1000.0 / self.sample_rate;
        self.coefs = HopCoefs {
            depth_db: cfg.depth_db,
            selectivity_db: cfg.selectivity_db,
            attack: (-hop_ms / cfg.attack_ms).exp(),
            release: (-hop_ms / cfg.release_ms).exp(),
        };
        self.cfg = cfg;
        if !self.primed {
            self.primed = true;
            self.fade = if cfg.enabled { 1.0 } else { 0.0 };
            self.mix = cfg.mix;
        }
    }

    /// Process a stereo block in place.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SuppressorConfig) {
        self.set_config(cfg);
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let (l, r) = self.step(left[i], right[i]);
            left[i] = l;
            right[i] = r;
        }
    }

    /// Process a mono block in place (linked detection on the one
    /// channel; the mode is ignored).
    pub fn process_mono(&mut self, buffer: &mut [f32], cfg: &SuppressorConfig) {
        let mut cfg = *cfg;
        cfg.mode = SuppressorMode::Stereo;
        self.set_config(&cfg);
        for x in buffer.iter_mut() {
            *x = self.step(*x, *x).0;
        }
    }

    #[inline]
    fn step(&mut self, l: f32, r: f32) -> (f32, f32) {
        let idx = self.pos;
        // Before the write, the history slot holds x[s − frame]: the
        // dry path, delayed by exactly the latency.
        let dry_l = self.hist_l[idx];
        let dry_r = self.hist_r[idx];
        let y_l = self.acc_l[idx];
        let y_r = self.acc_r[idx];
        self.acc_l[idx] = 0.0;
        self.acc_r[idx] = 0.0;
        self.hist_l[idx] = l;
        self.hist_r[idx] = r;
        self.pos = (idx + 1) & self.mask;
        self.countdown -= 1;
        if self.countdown == 0 {
            self.countdown = self.geometry.hop;
            self.run_frame();
        }

        let step = self.ramp_step;
        let mix_target = self.cfg.mix;
        if self.mix != mix_target {
            self.mix = if mix_target > self.mix {
                (self.mix + step).min(mix_target)
            } else {
                (self.mix - step).max(mix_target)
            };
        }
        let fade_target = if self.cfg.enabled { 1.0 } else { 0.0 };
        if self.fade != fade_target {
            self.fade = if fade_target > self.fade {
                (self.fade + step).min(fade_target)
            } else {
                (self.fade - step).max(fade_target)
            };
        }
        if self.fade == 0.0 {
            return (dry_l, dry_r);
        }
        let (mut out_l, mut out_r) = if self.mix == 1.0 {
            (y_l, y_r)
        } else {
            (dry_l + self.mix * (y_l - dry_l), dry_r + self.mix * (y_r - dry_r))
        };
        if self.cfg.delta {
            out_l = dry_l - out_l;
            out_r = dry_r - out_r;
        }
        if self.fade == 1.0 {
            (out_l, out_r)
        } else {
            (dry_l + self.fade * (out_l - dry_l), dry_r + self.fade * (out_r - dry_r))
        }
    }

    /// One STFT frame over the last `frame` inputs: analyse, cut,
    /// resynthesise, overlap-add.
    fn run_frame(&mut self) {
        let n = self.geometry.frame;
        let mask = self.mask;
        let mode = self.cfg.mode;
        let ms = mode != SuppressorMode::Stereo;
        for i in 0..n {
            let j = (self.pos + i) & mask;
            let (l, r) = (self.hist_l[j], self.hist_r[j]);
            let (a, b) = if ms { ms_encode(l, r) } else { (l, r) };
            let w = self.window[i];
            self.buf[i] = Complex::new(w * a, w * b);
        }
        self.fft_forward
            .process_with_scratch(&mut self.buf, &mut self.fft_scratch);

        let half = n / 2;
        for k in 0..=half {
            let z = self.buf[k];
            let zc = self.buf[(n - k) & mask].conj();
            let s = z + zc;
            let d = z - zc;
            // A = (z + z̄')/2, B = (z − z̄')/(2j).
            self.power_a[k] = 0.25 * s.norm_sqr();
            self.power_b[k] = 0.25 * d.norm_sqr();
        }

        let coefs = self.coefs;
        let (use_a, use_b, linked) = match mode {
            SuppressorMode::Stereo => (true, false, true),
            SuppressorMode::Mid => (true, false, false),
            SuppressorMode::Side => (false, true, false),
            SuppressorMode::MidSide => (true, true, false),
        };
        if linked {
            for k in 0..=half {
                self.power_a[k] = 0.5 * (self.power_a[k] + self.power_b[k]);
            }
        }
        if use_a {
            self.analysis.detect(&mut self.det_a, &self.power_a, coefs);
        } else if self.det_a.active {
            self.det_a.reset();
        }
        if use_b {
            self.analysis.detect(&mut self.det_b, &self.power_b, coefs);
        } else if self.det_b.active {
            self.det_b.reset();
        }

        let mut max_cut = 0.0f32;
        let mut any_cut = false;
        for k in 0..=half {
            let ca = self.det_a.cut[k];
            let cb = if linked { ca } else { self.det_b.cut[k] };
            max_cut = max_cut.max(ca).max(cb);
            any_cut |= ca != 0.0 || cb != 0.0;
            self.gain_a[k] = cut_to_gain(ca);
            self.gain_b[k] = cut_to_gain(cb);
        }
        self.max_cut_db = max_cut;

        if any_cut {
            for k in 0..=half {
                let (ga, gb) = (self.gain_a[k], self.gain_b[k]);
                if ga == 1.0 && gb == 1.0 {
                    continue;
                }
                let nk = (n - k) & mask;
                if ga == gb {
                    self.buf[k] *= ga;
                    if nk != k {
                        self.buf[nk] *= ga;
                    }
                    continue;
                }
                let z = self.buf[k];
                let zc = self.buf[nk].conj();
                let a = (z + zc) * 0.5;
                let d = z - zc;
                let b = Complex::new(0.5 * d.im, -0.5 * d.re);
                // out[k] = ga·A + j·gb·B; out[N−k] = conj(ga·A − j·gb·B)
                // = ga·Ā + j·gb·B̄ (both spectra Hermitian).
                let ja = a * ga;
                let jb = Complex::new(-b.im * gb, b.re * gb);
                self.buf[k] = ja + jb;
                if nk != k {
                    self.buf[nk] = (ja - jb).conj();
                }
            }
        }

        self.fft_inverse
            .process_with_scratch(&mut self.buf, &mut self.fft_scratch);
        let scale = self.out_scale;
        for i in 0..n {
            let j = (self.pos + i) & mask;
            let g = self.window[i] * scale;
            let (a, b) = (self.buf[i].re * g, self.buf[i].im * g);
            let (l, r) = if ms { ms_decode(a, b) } else { (a, b) };
            self.acc_l[j] += l;
            self.acc_r[j] += r;
        }
    }
}
