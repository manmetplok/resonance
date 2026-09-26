//! Real-time granular engine core.
//!
//! Fixed pre-allocated grain pool, sync/async scheduler, equal-power
//! overlap compensation and click-free voice stealing per research doc
//! #252 (§2, §5, §8) and the epic #196 architecture doc #253, plus
//! per-grain pitch (doc #252 §3): playback-rate transposition on an
//! `f64` phase accumulator, symmetric ± detune spread, an optional
//! rate-tracked anti-alias lowpass and reverse (negative-rate) grains,
//! and optional WSOLA-style correlation-aligned grain onsets (doc #252
//! §4–5): before a grain spawns, a bounded window around its nominal
//! read position is searched for the lag maximizing normalized
//! cross-correlation with the natural continuation of the previous
//! onset, so splices stay phase-coherent with the sounding material.
//!
//! The engine is framework-agnostic so it can be shared by the granular
//! delay (epic #196) and a future granular instrument (epic #75):
//!
//! * Grains read from a **caller-provided circular source buffer**. The
//!   caller owns the write head and the buffer contents — the engine
//!   makes no delay/feedback/freeze assumptions. `head_advance`
//!   describes how the head moves per output sample (`1.0` for a
//!   streaming delay line, `0.0` for a frozen buffer or static corpus).
//! * The granulated result is **accumulated** (`+=`) into
//!   caller-provided stereo output blocks, so the caller can mix wet
//!   output over an existing bus without an intermediate copy.
//!
//! The render path ([`GrainEngine::process`]) performs no heap
//! allocation and takes no locks: the grain pool, its free-list and the
//! [`WindowMorph`] LUT are all built in [`GrainEngine::new`]. Grain
//! parameters are latched at spawn time (doc #253: the grain cloud
//! itself interpolates parameter changes, so per-grain values need no
//! smoothing).

use crate::interp::{
    read_bspline6_wrapped, read_hermite_wrapped, read_linear_wrapped, BandlimitedReader,
};
use crate::pan::constant_power_pan;
use crate::rng::SimpleRng;
use crate::window::WindowMorph;

/// Fixed size of the pre-allocated grain pool (doc #253: MAX ≥ 64).
pub const MAX_GRAINS: usize = 64;

/// Safety margin (samples) kept between any grain read span and the
/// caller's write head, on both sides (doc #252 §5, write-head
/// collisions). Covers the ±2-sample support of the Hermite reader.
const HEAD_MARGIN_SAMPLES: f64 = 16.0;

/// Length of the voice-steal release ramp, seconds. Short enough to free
/// the slot quickly, long enough to stay click-free (doc #252 §2: never
/// hard-kill a grain).
const STEAL_RELEASE_SECONDS: f32 = 0.002;

/// Minimum raised-cosine edge enforced on every grain, seconds — even a
/// boxcar (`texture = 0`) grain gets 1.5 ms tapers so onsets and ends
/// are click-free (doc #252 §5).
const MIN_EDGE_SECONDS: f32 = 0.0015;

/// Smallest useful grain length, seconds (must fit two minimum edges).
const MIN_GRAIN_SECONDS: f32 = 0.004;

/// Floor for the expected-overlap compensation so very sparse clouds do
/// not receive an unbounded gain boost.
const MIN_EXPECTED_OVERLAP: f32 = 0.05;

/// Hard cap on the onset-alignment search half-window, seconds per
/// side, bounding the per-spawn correlation cost regardless of
/// parameter values (doc #252 §4-5: "a small window (a few ms)").
const MAX_ALIGN_WINDOW_SECONDS: f32 = 0.01;

/// Length of the alignment comparison window, seconds — long enough to
/// span at least one period of typical pitched material (≥ 250 Hz at
/// full coverage), short enough to keep the correlator cheap.
const ALIGN_COMPARE_SECONDS: f32 = 0.004;

/// Coarse-search decimation for the alignment correlator: the first
/// pass scores every `ALIGN_DECIM`-th lag using every `ALIGN_DECIM`-th
/// comparison sample; a full-rate pass then refines around the winner.
const ALIGN_DECIM: usize = 4;

/// Reference-energy floor below which the correlator treats the
/// sounding material as silence and keeps the nominal onset.
const ALIGN_ENERGY_FLOOR: f32 = 1e-9;

/// Magnitude levels of the 8-bit µ-law quantizer: sign + 7 magnitude
/// bits (ba todo #1083, doc #252 §9 "optional µ-law lo-fi tier").
const MU_LAW_LEVELS: usize = 128;

/// µ-law companding constant (µ = 255, the telephony standard — 8-bit
/// µ-law is the format Clouds stores its buffer in on the lo-fi
/// quality settings).
const MU_LAW_MU: f32 = 255.0;

/// Per-grain read interpolation quality (ba todo #1083, doc #252 §3).
///
/// Latched per grain at spawn — switching quality tiers mid-stream
/// never changes the kernel under a sounding grain, so tier switches
/// are click-free by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpQuality {
    /// 2-point linear: audibly dull, poor image rejection (the Lo-fi
    /// tier's character).
    Linear,
    /// 4-point cubic Hermite (Catmull-Rom): the standard sampler
    /// compromise; the Normal tier.
    #[default]
    Hermite4,
    /// 6-point, 5th-order B-spline: best-in-class image rejection of
    /// the polynomial family (Niemitalo deip.pdf); the HQ tier. See
    /// [`crate::interp::bspline6`] for the full justification.
    Bspline6,
}

/// Grain-onset scheduling mode (doc #252 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SchedulerMode {
    /// Fixed inter-onset time `IOT = 1 / density` — periodic, pitched
    /// granulation.
    #[default]
    Sync,
    /// Stochastic inter-onset time, uniform in `[0.5, 1.5) · (1 /
    /// density)` so the mean rate still equals `density`.
    Async,
}

/// Per-block grain-engine parameters.
///
/// All values are read at [`GrainEngine::process`] time; grain-latched
/// quantities (length, position, level, pan, texture) are sampled per
/// grain at its onset. Construct with struct-update syntax over
/// [`Default::default`] so new fields stay source-compatible:
///
/// ```
/// use resonance_dsp::GrainParams;
/// let params = GrainParams { density_hz: 40.0, ..GrainParams::default() };
/// ```
#[derive(Debug, Clone)]
pub struct GrainParams {
    /// Grain spawn rate in grains per second. `<= 0` disables spawning
    /// (live grains keep rendering to completion).
    pub density_hz: f32,
    /// Nominal grain duration in seconds (clamped to ≥ 4 ms).
    pub grain_seconds: f32,
    /// Nominal read position, in seconds *behind* the write head.
    pub position_seconds: f32,
    /// Uniform ± jitter applied to the position, seconds ("spray").
    pub position_jitter_seconds: f32,
    /// Uniform ± jitter applied to the grain length, as a fraction of
    /// `grain_seconds` (`0..=1`).
    pub size_jitter: f32,
    /// Random per-grain attenuation depth (`0..=1`): each grain's level
    /// is scaled by a uniform value in `[1 − level_jitter, 1]`.
    pub level_jitter: f32,
    /// Random constant-power pan spread (`0..=1`): each grain is panned
    /// uniformly within `±pan_spread` around centre.
    pub pan_spread: f32,
    /// Window morph amount for [`WindowMorph`]: 0 = boxcar (with the
    /// enforced minimum raised-cosine edges), 1 = Hann.
    pub texture: f32,
    /// Onset scheduling mode.
    pub mode: SchedulerMode,
    /// Samples the caller's write head advances per rendered output
    /// sample: `1.0` for a streaming delay buffer, `0.0` for a frozen
    /// buffer or a static sample corpus (epic #75).
    pub head_advance: f32,
    /// Grain transposition in semitones; the per-grain playback rate is
    /// `2^(semitones / 12)` (doc #252 §3).
    pub pitch_semitones: f32,
    /// Detune spread in cents: each grain adds a random offset with
    /// magnitude uniform in `[0, spread]` cents whose *sign alternates*
    /// between consecutive grains (symmetric detune), keeping the
    /// perceived centre pitch stable (doc #252 §3).
    pub detune_spread_cents: f32,
    /// Probability (`0..=1`) that a grain plays in reverse (negative
    /// playback rate). Reversed grains still window to zero at both
    /// ends and respect the write-head collision guard.
    pub reverse_probability: f32,
    /// Enable the band-limited read for upward transposition (the HQ
    /// quality tier turns this on). Engaged per grain only when
    /// `|rate| > 1`, where the read decimates the source and would fold
    /// content above `Nyquist / |rate|`: such grains read through a
    /// windowed sinc whose cutoff tracks `1 / |rate|` instead of the
    /// `interp` kernel, removing that content *before* it can fold
    /// (DSP-09; a lowpass after a polynomial read — the old one-pole —
    /// cannot tell folded partials from real ones).
    pub anti_alias: bool,
    /// Enable WSOLA-style correlation-aligned grain onsets (doc #252
    /// §4-5). Before a grain spawns, the engine searches
    /// `±align_window_seconds` around its nominal read position for the
    /// lag maximizing normalized cross-correlation between the
    /// candidate onset region (read along the new grain's rate) and the
    /// natural continuation of the previously spawned grain, then snaps
    /// the onset there. Splices become phase-coherent with the sounding
    /// material — most of the pitch-synchronous quality benefit with no
    /// pitch tracker, and it works on polyphonic material. Default off;
    /// when off, the render path is unchanged (bit-identical output, no
    /// correlation cost).
    pub align: bool,
    /// Half-width of the alignment search window, seconds (clamped to
    /// `0 ..= 10 ms`). The aligned onset never deviates from the
    /// nominal position by more than this, and never into the
    /// write-head collision guard. Only read when `align` is true.
    pub align_window_seconds: f32,
    /// Read-interpolation kernel, latched per grain at spawn (ba todo
    /// #1083): sounding grains keep the kernel they spawned with, so
    /// quality-tier switches are click-free by construction.
    pub interp: InterpQuality,
    /// Lo-fi tier (ba todo #1083): pass each grain's resampled stream
    /// through an 8-bit µ-law quantizer (Clouds-style lo-fi buffer
    /// character). Latched per grain at spawn, and applied *before* the
    /// window/gain so the quantization noise is enveloped with the
    /// grain — no buffer format ever changes, hence nothing to
    /// crossfade. Quantizing at the read instead of in the buffer
    /// keeps the stored audio pristine, so leaving the tier is
    /// instantly clean.
    pub lofi_quantize: bool,
    /// Cap on simultaneously sounding grains (clamped to `1 ..=`
    /// [`MAX_GRAINS`]): the Lo-fi tier's reduced grain count (ba todo
    /// #1083; doc #252 §2 — Clouds' pool shrinks on low-quality
    /// modes). At the cap a new onset behaves exactly like a full
    /// pool: the grain nearest completion is stolen (release-ramped,
    /// click-free) and the onset is dropped, so lowering the cap
    /// mid-stream drains the excess gracefully.
    pub max_polyphony: usize,
}

impl Default for GrainParams {
    fn default() -> Self {
        Self {
            density_hz: 25.0,
            grain_seconds: 0.08,
            position_seconds: 0.25,
            position_jitter_seconds: 0.0,
            size_jitter: 0.0,
            level_jitter: 0.0,
            pan_spread: 0.0,
            texture: 0.5,
            mode: SchedulerMode::Sync,
            head_advance: 1.0,
            pitch_semitones: 0.0,
            detune_spread_cents: 0.0,
            reverse_probability: 0.0,
            anti_alias: false,
            align: false,
            align_window_seconds: 0.005,
            interp: InterpQuality::Hermite4,
            lofi_quantize: false,
            max_polyphony: MAX_GRAINS,
        }
    }
}

/// One pooled grain voice. All fields are plain scalars so the pool is a
/// fixed flat array with no per-grain indirection.
#[derive(Debug, Clone, Copy)]
struct Grain {
    active: bool,
    /// Fractional read position in the source buffer (absolute, wraps
    /// through [`read_hermite_wrapped`]'s power-of-two mask).
    read_pos: f64,
    /// Playback rate in source samples per output sample.
    rate: f64,
    /// Total duration in output samples.
    dur: f64,
    /// `1 / dur`, cached for the per-sample envelope phase.
    inv_dur: f64,
    /// Envelope phase accumulator in output samples (`0..dur`).
    env_phase: f64,
    /// Effective window texture (params texture with the minimum edge
    /// enforced).
    texture: f32,
    /// Overall grain gain: overlap compensation × level jitter.
    gain: f32,
    gain_l: f32,
    gain_r: f32,
    /// True once the voice-steal release ramp is running.
    releasing: bool,
    /// Multiplicative steal-release ramp, 1 → 0.
    release_gain: f32,
    /// Intra-block onset offset for a grain spawned this block; reset to
    /// 0 after its first render pass.
    start_offset: u32,
    /// Block offset at which a pending steal starts the release ramp;
    /// `u32::MAX` = no steal pending.
    release_at: u32,
    /// True when this grain reads through the band-limited sinc
    /// (anti-alias on and `|rate| > 1`).
    aa_active: bool,
    /// Read-interpolation kernel, latched at spawn (ba todo #1083).
    interp: InterpQuality,
    /// 8-bit µ-law quantization of the resampled stream, latched at
    /// spawn (Lo-fi tier, ba todo #1083).
    lofi: bool,
}

impl Grain {
    const INACTIVE: Grain = Grain {
        active: false,
        read_pos: 0.0,
        rate: 1.0,
        dur: 0.0,
        inv_dur: 0.0,
        env_phase: 0.0,
        texture: 0.0,
        gain: 0.0,
        gain_l: 0.0,
        gain_r: 0.0,
        releasing: false,
        release_gain: 0.0,
        start_offset: 0,
        release_at: u32::MAX,
        aa_active: false,
        interp: InterpQuality::Hermite4,
        lofi: false,
    };
}

/// Read-only view of one sounding grain (metering aid, ba todo #1135):
/// a plain-scalar snapshot for editors/visualizers, decoupled from the
/// pool's internal [`Grain`] layout.
#[derive(Debug, Clone, Copy)]
pub struct GrainView {
    /// Fractional read position in the source buffer, absolute samples
    /// (wraps through the caller's power-of-two mask).
    pub read_pos: f64,
    /// Playback rate in source samples per output sample (negative for
    /// reversed grains).
    pub rate: f64,
    /// Total grain duration in output samples.
    pub dur_samples: f64,
    /// Current enveloped level: window value × grain gain (overlap
    /// compensation × level jitter) × steal-release ramp. May exceed 1
    /// for sparse clouds (overlap-compensation boost).
    pub level: f32,
}

/// Real-time grain engine: fixed pool, scheduler, overlap-compensated
/// gain and voice stealing. See the module docs for the reuse contract.
pub struct GrainEngine {
    window: WindowMorph,
    grains: [Grain; MAX_GRAINS],
    /// Free-slot stack over `grains` (indices `free[..free_len]`).
    free: [usize; MAX_GRAINS],
    free_len: usize,
    rng: SimpleRng,
    sample_rate: f32,
    /// Samples from the start of the next block to the next grain onset.
    next_onset: f64,
    spawned: u64,
    /// Sign of the next detune offset; alternates per spawned grain so
    /// the detune cloud stays symmetric around the centre pitch.
    detune_sign: f32,
    /// Full-rate correlation template scratch, pre-allocated in
    /// [`GrainEngine::new`]: the previous onset's continuation, read
    /// along its own playback rate. Never resized on the render path.
    align_template: Vec<f32>,
    /// Total output samples processed; drives the continuation
    /// projection of the previous onset to the current onset time.
    time_samples: f64,
    /// True once a grain has spawned since construction/reset, i.e. the
    /// `align_prev_*` fields describe a real onset.
    align_armed: bool,
    /// Read position of the most recently spawned grain at its onset.
    align_prev_pos: f64,
    /// Playback rate of the most recently spawned grain.
    align_prev_rate: f64,
    /// Absolute output-sample time of the most recent onset.
    align_prev_time: f64,
    /// Largest |lag| the aligner has applied since construction/reset.
    max_abs_align_lag: f64,
    /// Spawns the aligner moved off their nominal onset (nonzero lag).
    aligned_spawns: u64,
    /// Decode table of the 8-bit µ-law quantizer (ba todo #1083):
    /// `mu_law_decode[n]` is the magnitude decoded from companded
    /// level `n` of [`MU_LAW_LEVELS`]. Built once in
    /// [`GrainEngine::new`] so the per-sample Lo-fi path is a
    /// log + round + table read — no `powf` on the render path.
    mu_law_decode: [f32; MU_LAW_LEVELS],
    /// Band-limited read kernel for anti-aliased upward-transposed
    /// grains (DSP-09); its table is built in [`GrainEngine::new`].
    bl_reader: BandlimitedReader,
}

impl GrainEngine {
    /// Build an engine for `sample_rate`, pre-allocating the grain pool,
    /// free-list and window LUT. `seed` drives all stochastic behaviour
    /// (async IOT and every jitter), so runs are reproducible.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not strictly positive.
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        assert!(
            sample_rate > 0.0,
            "sample rate must be positive, got {sample_rate}"
        );
        let mut free = [0usize; MAX_GRAINS];
        for (i, slot) in free.iter_mut().enumerate() {
            *slot = i;
        }
        let compare_len =
            ((ALIGN_COMPARE_SECONDS * sample_rate).ceil() as usize).max(2 * ALIGN_DECIM);
        // µ-law decode table: level n of the companded magnitude maps
        // back to ((1+µ)^(n/(N-1)) − 1) / µ.
        let mut mu_law_decode = [0.0f32; MU_LAW_LEVELS];
        for (n, slot) in mu_law_decode.iter_mut().enumerate() {
            let y = n as f32 / (MU_LAW_LEVELS - 1) as f32;
            *slot = ((1.0 + MU_LAW_MU).powf(y) - 1.0) / MU_LAW_MU;
        }
        Self {
            window: WindowMorph::new(),
            grains: [Grain::INACTIVE; MAX_GRAINS],
            free,
            free_len: MAX_GRAINS,
            rng: SimpleRng::new(seed),
            sample_rate,
            next_onset: 0.0,
            spawned: 0,
            detune_sign: 1.0,
            align_template: vec![0.0; compare_len],
            time_samples: 0.0,
            align_armed: false,
            align_prev_pos: 0.0,
            align_prev_rate: 1.0,
            align_prev_time: 0.0,
            max_abs_align_lag: 0.0,
            aligned_spawns: 0,
            mu_law_decode,
            bl_reader: BandlimitedReader::new(),
        }
    }

    /// Silence and free every grain and rearm the scheduler. Call on
    /// transport/plugin reactivation; allocation-free.
    pub fn reset(&mut self) {
        for (i, (grain, slot)) in self.grains.iter_mut().zip(self.free.iter_mut()).enumerate() {
            *grain = Grain::INACTIVE;
            *slot = i;
        }
        self.free_len = MAX_GRAINS;
        self.next_onset = 0.0;
        self.detune_sign = 1.0;
        self.time_samples = 0.0;
        self.align_armed = false;
        self.align_prev_pos = 0.0;
        self.align_prev_rate = 1.0;
        self.align_prev_time = 0.0;
        self.max_abs_align_lag = 0.0;
        self.aligned_spawns = 0;
    }

    /// Number of currently sounding grains (never exceeds
    /// [`MAX_GRAINS`]).
    pub fn active_grains(&self) -> usize {
        self.grains.iter().filter(|g| g.active).count()
    }

    /// Total grains spawned since construction (metering/testing aid).
    pub fn grains_spawned(&self) -> u64 {
        self.spawned
    }

    /// Playback rates of the currently sounding grains, in source
    /// samples per output sample (negative for reversed grains).
    /// Allocation-free inspection aid for metering and tests.
    pub fn active_rates(&self) -> impl Iterator<Item = f64> + '_ {
        self.grains.iter().filter(|g| g.active).map(|g| g.rate)
    }

    /// Read-only views of the currently sounding grains (allocation-free
    /// metering aid, ba todo #1135): read position, rate, duration and
    /// the grain's current enveloped level (window × gain × steal-release
    /// ramp) — everything an editor needs to draw the live grain cloud.
    pub fn active_grain_views(&self) -> impl Iterator<Item = GrainView> + '_ {
        self.grains.iter().filter(|g| g.active).map(|g| GrainView {
            read_pos: g.read_pos,
            rate: g.rate,
            dur_samples: g.dur,
            level: self
                .window
                .evaluate((g.env_phase * g.inv_dur) as f32, g.texture)
                * g.gain
                * g.release_gain,
        })
    }

    /// Largest onset-alignment lag magnitude applied since
    /// construction/reset, in samples (0 until the aligner moves a
    /// grain). Never exceeds the clamped `align_window_seconds` in
    /// samples — metering/testing aid for the alignment bound.
    pub fn max_abs_align_lag_samples(&self) -> f64 {
        self.max_abs_align_lag
    }

    /// Number of grain onsets the aligner has moved off their nominal
    /// position since construction/reset (metering/testing aid).
    pub fn aligned_spawns(&self) -> u64 {
        self.aligned_spawns
    }

    /// Granulate `source` into `out_left` / `out_right` (accumulating).
    ///
    /// * `source` — circular source buffer; its length must be a power
    ///   of two ≥ 4 (the [`read_hermite_wrapped`] contract).
    /// * `write_pos` — the caller's write-head position, in samples, at
    ///   the first sample of this block. The engine assumes the head
    ///   moves by `params.head_advance` per output sample.
    /// * Output blocks must have equal lengths; the block length is
    ///   `out_left.len()`.
    ///
    /// Onsets are scheduled with intra-block sample accuracy: an IOT
    /// smaller than the block length spawns multiple grains at their
    /// exact offsets, and the fractional part of an onset time refines
    /// the grain's initial read position. No allocation, no locks.
    ///
    /// # Panics
    /// Panics if the output blocks differ in length or `source` violates
    /// the power-of-two contract.
    pub fn process(
        &mut self,
        source: &[f32],
        write_pos: f64,
        params: &GrainParams,
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) {
        assert_eq!(
            out_left.len(),
            out_right.len(),
            "stereo output blocks must have equal lengths"
        );
        let block_len = out_left.len();
        if block_len == 0 {
            return;
        }

        // --- Phase 1: schedule every onset that falls in this block. --
        if params.density_hz > 0.0 {
            let base_iot = (self.sample_rate as f64 / params.density_hz as f64).max(1.0);
            while self.next_onset < block_len as f64 {
                let onset = self.next_onset;
                self.spawn(source, write_pos, onset, params);
                let iot = match params.mode {
                    SchedulerMode::Sync => base_iot,
                    SchedulerMode::Async => (base_iot * (0.5 + self.unit() as f64)).max(1.0),
                };
                self.next_onset += iot;
            }
        }
        self.next_onset = (self.next_onset - block_len as f64).max(0.0);

        // --- Phase 2: render every live grain over its block span. ----
        let release_step = 1.0 / (STEAL_RELEASE_SECONDS * self.sample_rate);
        let Self {
            ref window,
            ref mut grains,
            ref mut free,
            ref mut free_len,
            ref mu_law_decode,
            ref bl_reader,
            ..
        } = *self;
        // 8-bit µ-law quantizer for Lo-fi grains: compand, round to one
        // of [`MU_LAW_LEVELS`] magnitude levels, decode via the table.
        let inv_log_mu = 1.0 / (1.0 + MU_LAW_MU).ln();
        let mu_law = |s: f32| -> f32 {
            let mag = s.abs().min(1.0);
            let y = (1.0 + MU_LAW_MU * mag).ln() * inv_log_mu;
            let n = (y * (MU_LAW_LEVELS - 1) as f32).round() as usize;
            mu_law_decode[n.min(MU_LAW_LEVELS - 1)].copysign(s)
        };
        for (idx, grain) in grains.iter_mut().enumerate() {
            if !grain.active {
                continue;
            }
            let start = (grain.start_offset as usize).min(block_len);
            grain.start_offset = 0;
            let mut finished = false;
            for k in start..block_len {
                if !grain.releasing && k as u32 >= grain.release_at {
                    grain.releasing = true;
                }
                let w = window.evaluate((grain.env_phase * grain.inv_dur) as f32, grain.texture);
                // Per-grain kernel, latched at spawn (ba todo #1083);
                // upward-transposed anti-aliased grains band-limit at
                // the read (DSP-09).
                let mut s = if grain.aa_active {
                    bl_reader.read_wrapped(source, grain.read_pos, grain.rate)
                } else {
                    match grain.interp {
                        InterpQuality::Linear => read_linear_wrapped(source, grain.read_pos),
                        InterpQuality::Hermite4 => read_hermite_wrapped(source, grain.read_pos),
                        InterpQuality::Bspline6 => read_bspline6_wrapped(source, grain.read_pos),
                    }
                };
                if grain.lofi {
                    // Applied before window/gain so the quantization
                    // noise is enveloped with the grain (click-free at
                    // the grain edges).
                    s = mu_law(s);
                }
                let v = s * w * grain.gain * grain.release_gain;
                out_left[k] += v * grain.gain_l;
                out_right[k] += v * grain.gain_r;
                grain.read_pos += grain.rate;
                grain.env_phase += 1.0;
                if grain.releasing {
                    grain.release_gain -= release_step;
                    if grain.release_gain <= 0.0 {
                        finished = true;
                        break;
                    }
                }
                if grain.env_phase >= grain.dur {
                    finished = true;
                    break;
                }
            }
            if finished {
                *grain = Grain::INACTIVE;
                free[*free_len] = idx;
                *free_len += 1;
            }
        }
        self.time_samples += block_len as f64;
    }

    /// Latch a new grain at fractional block offset `onset`. Claims a
    /// free slot; with the pool full it instead steals (see
    /// [`Self::steal`]) and drops this onset.
    fn spawn(&mut self, source: &[f32], write_pos: f64, onset: f64, params: &GrainParams) {
        let sr = self.sample_rate as f64;
        let source_len = source.len();

        // Grain-latched draws happen unconditionally so the RNG stream
        // does not depend on pool occupancy.
        let size_jitter = params.size_jitter.clamp(0.0, 1.0) * self.bipolar();
        let position_jitter = params.position_jitter_seconds * self.bipolar();
        let level = 1.0 - params.level_jitter.clamp(0.0, 1.0) * self.unit();
        let pan = params.pan_spread.clamp(0.0, 1.0) * self.bipolar();

        let dur_seconds = (params.grain_seconds * (1.0 + size_jitter)).max(MIN_GRAIN_SECONDS);
        let dur = (dur_seconds as f64 * sr).max(4.0);
        let advance = params.head_advance as f64;

        // Polyphony cap (ba todo #1083): at (or above, after a
        // mid-stream cap reduction) the cap, a new onset behaves
        // exactly like a full pool — steal the grain nearest
        // completion (release-ramped) and drop this onset.
        let cap = params.max_polyphony.clamp(1, MAX_GRAINS);
        let in_use = MAX_GRAINS - self.free_len;
        let slot = if self.free_len > 0 && in_use < cap {
            self.free_len -= 1;
            self.free[self.free_len]
        } else {
            self.steal(onset as u32);
            return;
        };

        // Per-grain playback rate (doc #252 §3): transpose plus a
        // symmetric detune whose sign alternates between consecutive
        // spawned grains, so the detune cloud has ~zero mean and the
        // perceived centre pitch stays put.
        let mut semitones = params.pitch_semitones;
        if params.detune_spread_cents > 0.0 {
            let magnitude_cents = self.unit() * params.detune_spread_cents;
            semitones += self.detune_sign * magnitude_cents * (1.0 / 100.0);
            self.detune_sign = -self.detune_sign;
        }
        let mut rate = (semitones as f64 / 12.0).exp2();
        if params.reverse_probability > 0.0
            && self.unit() < params.reverse_probability.min(1.0)
        {
            rate = -rate;
        }

        // Optional band-limited read: only upward transposition
        // (`|rate| > 1`) folds content past Nyquist/rate (DSP-09).
        let aa_active = params.anti_alias && rate.abs() > 1.0;
        // The sinc reaches `half_width` source samples either side of
        // the read position; widen the head clearance to match.
        let margin = if aa_active {
            HEAD_MARGIN_SAMPLES + BandlimitedReader::half_width(rate)
        } else {
            HEAD_MARGIN_SAMPLES
        };

        // Write-head collision guard (doc #252 §5): over the grain's
        // lifetime the reader must neither overtake the head (fast
        // grains) nor be lapped by it (slow, frozen or reversed grains
        // on long buffers). `d` is the start offset behind the head.
        let d_min = margin + dur * (rate - advance).max(0.0);
        let d_max = source_len as f64 - margin - dur * (advance - rate).max(0.0);
        if d_max < d_min {
            // The buffer cannot hold a grain of this length/rate at all.
            self.free[self.free_len] = slot;
            self.free_len += 1;
            return;
        }
        let position = (params.position_seconds as f64 + position_jitter as f64) * sr;
        let offset = position.clamp(d_min, d_max);
        let head_at_onset = write_pos + onset * advance;
        let now = self.time_samples + onset;

        // WSOLA-style onset alignment (doc #252 §4-5): snap the read
        // position to the lag, within the bounded search window, that
        // maximizes normalized cross-correlation with the natural
        // continuation of the previous onset. The lag range is
        // intersected with the head-collision guard, so alignment can
        // never move a grain into the unsafe region.
        let mut align_lag = 0.0_f64;
        if params.align && self.align_armed {
            let window = (params.align_window_seconds.clamp(0.0, MAX_ALIGN_WINDOW_SECONDS) as f64
                * sr)
                .floor();
            let lo = (-window).max(offset - d_max);
            let hi = window.min(offset - d_min);
            if window >= 1.0 && lo <= hi {
                align_lag = self.correlate_lag(source, head_at_onset - offset, rate, now, lo, hi);
                self.max_abs_align_lag = self.max_abs_align_lag.max(align_lag.abs());
                if align_lag != 0.0 {
                    self.aligned_spawns += 1;
                }
            }
        }
        let read_pos = head_at_onset - offset + align_lag;

        // Equal-power overlap compensation (doc #252 §2): uncorrelated
        // grain loudness grows with sqrt(overlap), so scale each grain
        // by 1/sqrt(expected_overlap).
        let expected_overlap =
            (params.grain_seconds.max(MIN_GRAIN_SECONDS) * params.density_hz.max(0.0))
                .max(MIN_EXPECTED_OVERLAP);
        let gain = level / expected_overlap.sqrt();
        let (gain_l, gain_r) = constant_power_pan(pan);

        // Enforce the minimum raised-cosine edge even at texture = 0.
        let min_texture = (2.0 * MIN_EDGE_SECONDS / dur_seconds).min(1.0);
        let texture = params.texture.clamp(0.0, 1.0).max(min_texture);

        // The fractional part of the onset refines the read position
        // (Beads-style fractional grain starts); the envelope starts at
        // the integer sample `onset as u32`.
        self.grains[slot] = Grain {
            active: true,
            read_pos,
            rate,
            dur,
            inv_dur: 1.0 / dur,
            env_phase: 0.0,
            texture,
            gain,
            gain_l,
            gain_r,
            releasing: false,
            release_gain: 1.0,
            start_offset: onset as u32,
            release_at: u32::MAX,
            aa_active,
            interp: params.interp,
            lofi: params.lofi_quantize,
        };
        self.spawned += 1;

        // Record this onset as the alignment reference for the next
        // spawn. Kept up to date even with alignment off (pure state,
        // no RNG draw, no output effect), so enabling it mid-stream
        // aligns immediately.
        self.align_prev_pos = read_pos;
        self.align_prev_rate = rate;
        self.align_prev_time = now;
        self.align_armed = true;
    }

    /// Score integer lags `lo ..= hi` (samples, relative to `cand_pos`)
    /// and return the one maximizing normalized cross-correlation
    /// between the candidate onset region — read along the new grain's
    /// `rate` — and the previous onset's natural continuation projected
    /// to `now` along its own rate (WSOLA: compare what will sound
    /// against what is sounding). Two passes bound the cost: a coarse
    /// pass on every [`ALIGN_DECIM`]-th lag using every
    /// [`ALIGN_DECIM`]-th comparison sample, then a full-rate
    /// refinement around the coarse winner. The coarse grid passes
    /// through lag 0, which is scored first, so an already coherent
    /// nominal onset (or a silent/degenerate reference) keeps its
    /// nominal position. Runs entirely on the pre-allocated template
    /// scratch — no allocation.
    fn correlate_lag(
        &mut self,
        source: &[f32],
        cand_pos: f64,
        rate: f64,
        now: f64,
        lo: f64,
        hi: f64,
    ) -> f64 {
        let mask = (source.len() - 1) as i64;
        let n = self.align_template.len();

        // Template: the previously spawned grain's read trajectory,
        // advanced to this onset time.
        let t_pos = self.align_prev_pos + self.align_prev_rate * (now - self.align_prev_time);
        let mut t_energy_full = 0.0_f32;
        for (j, slot) in self.align_template.iter_mut().enumerate() {
            let idx = ((t_pos + self.align_prev_rate * j as f64).round() as i64 & mask) as usize;
            let s = source[idx];
            *slot = s;
            t_energy_full += s * s;
        }
        if t_energy_full <= ALIGN_ENERGY_FLOOR {
            return 0.0;
        }
        let template: &[f32] = &self.align_template;
        let mut t_energy_coarse = 0.0_f32;
        let mut j = 0;
        while j < n {
            t_energy_coarse += template[j] * template[j];
            j += ALIGN_DECIM;
        }

        // Normalized cross-correlation at an integer lag, evaluated on
        // every `step`-th comparison sample.
        let ncc = |lag: i64, step: usize, t_energy: f32| -> f32 {
            let mut dot = 0.0_f32;
            let mut c_energy = 0.0_f32;
            let mut j = 0;
            while j < n {
                let idx =
                    (((cand_pos + lag as f64 + rate * j as f64).round() as i64) & mask) as usize;
                let c = source[idx];
                dot += c * template[j];
                c_energy += c * c;
                j += step;
            }
            dot / (c_energy * t_energy).sqrt().max(1e-12)
        };

        let lo_i = lo.ceil() as i64;
        let hi_i = hi.floor() as i64;
        let step = ALIGN_DECIM as i64;
        let zero = 0_i64.clamp(lo_i, hi_i);
        let mut best_lag = zero;
        let mut best = ncc(zero, ALIGN_DECIM, t_energy_coarse);
        let mut lag = zero - ((zero - lo_i) / step) * step;
        while lag <= hi_i {
            if lag != zero {
                let s = ncc(lag, ALIGN_DECIM, t_energy_coarse);
                if s > best {
                    best = s;
                    best_lag = lag;
                }
            }
            lag += step;
        }
        let mut refined_lag = best_lag;
        let mut refined = ncc(best_lag, 1, t_energy_full);
        for lag in (best_lag - step + 1).max(lo_i)..=(best_lag + step - 1).min(hi_i) {
            if lag != best_lag {
                let s = ncc(lag, 1, t_energy_full);
                if s > refined {
                    refined = s;
                    refined_lag = lag;
                }
            }
        }
        refined_lag as f64
    }

    /// Pool-full policy: force the live grain nearest completion into
    /// the short release ramp starting at block offset `at`. The grain
    /// is never hard-killed; its slot frees once the ramp reaches zero.
    fn steal(&mut self, at: u32) {
        let mut victim: Option<usize> = None;
        let mut least_remaining = f64::INFINITY;
        for (i, g) in self.grains.iter().enumerate() {
            if !g.active || g.releasing || g.release_at != u32::MAX {
                continue;
            }
            let remaining = g.dur - g.env_phase;
            if remaining < least_remaining {
                least_remaining = remaining;
                victim = Some(i);
            }
        }
        if let Some(i) = victim {
            self.grains[i].release_at = at;
        }
    }

    /// Uniform draw in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.rng.next_u32() >> 8) as f32 * (1.0 / (1 << 24) as f32)
    }

    /// Uniform draw in `[-1, 1)`.
    fn bipolar(&mut self) -> f32 {
        2.0 * self.unit() - 1.0
    }
}
