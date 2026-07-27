//! Real-time granular engine core.
//!
//! Fixed pre-allocated grain pool, sync/async scheduler, equal-power
//! overlap compensation and click-free voice stealing per research doc
//! #252 (§2, §5, §8) and the epic #196 architecture doc #253, plus
//! per-grain pitch (doc #252 §3): playback-rate transposition on an
//! `f64` phase accumulator, symmetric ± detune spread, an optional
//! rate-tracked anti-alias lowpass and reverse (negative-rate) grains.
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

use crate::interp::read_hermite_wrapped;
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
    /// Enable the per-grain anti-alias one-pole lowpass, tracked to
    /// `0.45 · fs / |rate|` and engaged only when `|rate| > 1` (upward
    /// transposition folds content above `Nyquist / rate`; the HQ
    /// quality tier turns this on). The one-pole runs on the resampled
    /// grain stream, attenuating the folded images (doc #252 §3).
    pub anti_alias: bool,
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
    /// True when the anti-alias one-pole is engaged for this grain.
    aa_active: bool,
    /// One-pole coefficient `e^(-2π·fc/fs)` with `fc = 0.45·fs/|rate|`.
    aa_coeff: f32,
    /// One-pole state (per grain, reset at spawn).
    aa_state: f32,
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
        aa_coeff: 0.0,
        aa_state: 0.0,
    };
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
                self.spawn(source.len(), write_pos, onset, params);
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
            ..
        } = *self;
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
                let mut s = read_hermite_wrapped(source, grain.read_pos);
                if grain.aa_active {
                    grain.aa_state = s + grain.aa_coeff * (grain.aa_state - s);
                    s = grain.aa_state;
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
    }

    /// Latch a new grain at fractional block offset `onset`. Claims a
    /// free slot; with the pool full it instead steals (see
    /// [`Self::steal`]) and drops this onset.
    fn spawn(&mut self, source_len: usize, write_pos: f64, onset: f64, params: &GrainParams) {
        let sr = self.sample_rate as f64;

        // Grain-latched draws happen unconditionally so the RNG stream
        // does not depend on pool occupancy.
        let size_jitter = params.size_jitter.clamp(0.0, 1.0) * self.bipolar();
        let position_jitter = params.position_jitter_seconds * self.bipolar();
        let level = 1.0 - params.level_jitter.clamp(0.0, 1.0) * self.unit();
        let pan = params.pan_spread.clamp(0.0, 1.0) * self.bipolar();

        let dur_seconds = (params.grain_seconds * (1.0 + size_jitter)).max(MIN_GRAIN_SECONDS);
        let dur = (dur_seconds as f64 * sr).max(4.0);
        let advance = params.head_advance as f64;

        let slot = if self.free_len > 0 {
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

        // Optional rate-tracked anti-alias one-pole: only upward
        // transposition (`|rate| > 1`) folds content past Nyquist/rate.
        let abs_rate = rate.abs();
        let aa_active = params.anti_alias && abs_rate > 1.0;
        let aa_coeff = if aa_active {
            let fc = 0.45 * self.sample_rate / abs_rate as f32;
            (-(std::f32::consts::TAU * fc / self.sample_rate)).exp()
        } else {
            0.0
        };

        // Write-head collision guard (doc #252 §5): over the grain's
        // lifetime the reader must neither overtake the head (fast
        // grains) nor be lapped by it (slow, frozen or reversed grains
        // on long buffers). `d` is the start offset behind the head.
        let d_min = HEAD_MARGIN_SAMPLES + dur * (rate - advance).max(0.0);
        let d_max = source_len as f64 - HEAD_MARGIN_SAMPLES - dur * (advance - rate).max(0.0);
        if d_max < d_min {
            // The buffer cannot hold a grain of this length/rate at all.
            self.free[self.free_len] = slot;
            self.free_len += 1;
            return;
        }
        let position = (params.position_seconds as f64 + position_jitter as f64) * sr;
        let offset = position.clamp(d_min, d_max);

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
        let head_at_onset = write_pos + onset * advance;
        self.grains[slot] = Grain {
            active: true,
            read_pos: head_at_onset - offset,
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
            aa_coeff,
            aa_state: 0.0,
        };
        self.spawned += 1;
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
