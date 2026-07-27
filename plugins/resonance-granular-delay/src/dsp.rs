//! Granular-delay DSP core: a power-of-two stereo circular buffer, one
//! write head, and lock-stepped left/right grain engines granulating
//! behind it (doc #252 §1/§5/§8, doc #253).
//!
//! Everything is pre-allocated at construction; `process_block` performs
//! no allocation and takes no locks.

use resonance_dsp::{GrainEngine, GrainParams, SchedulerMode};

use crate::params::GranularSmoothers;

/// Maximum delay time the ring buffer is sized for at activation.
pub const MAX_DELAY_SECONDS: f32 = 4.0;

/// Shared seed for the two lock-stepped grain engines. Both engines
/// must consume identical RNG streams so left and right render the
/// *same* grain cloud over their respective channels (stereo balance
/// per grain). TODO(epic-196 #1077): decorrelated L/R scheduling will
/// deliberately split these seeds and add M/S width.
const ENGINE_SEED: u64 = 0x5EED_6417;

/// Block-level parameters resolved once per process call in `lib.rs`.
/// Grain-latched values need no smoothing (latched per grain at spawn);
/// the smoothed wet/dry mix comes off [`GranularSmoothers`].
pub struct BlockParams {
    /// Nominal grain read position behind the write head, seconds.
    pub delay_seconds: f32,
    pub grain_seconds: f32,
    pub density_hz: f32,
    pub scheduler: SchedulerMode,
    pub pitch_semitones: f32,
    pub detune_spread_cents: f32,
    pub texture: f32,
    /// Position jitter ("spray"), seconds.
    pub spray_seconds: f32,
    pub size_jitter: f32,
    pub level_jitter: f32,
    pub reverse_probability: f32,
    pub pan_spread: f32,
    /// HQ tier: engage the engine's rate-tracked anti-alias lowpass.
    pub anti_alias: bool,
}

pub struct GranularDsp {
    sample_rate: f32,
    /// Stereo circular source buffers; length is a power of two (the
    /// `read_hermite_wrapped` contract) sized for [`MAX_DELAY_SECONDS`].
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    mask: usize,
    /// Absolute write-head position in samples (monotonic; masked for
    /// indexing, passed absolute to the engines).
    write_pos: u64,
    /// Lock-stepped grain engines: same seed, same params, same call
    /// sequence, so their grain clouds are identical and each grain
    /// reads L and R at the same position (see [`ENGINE_SEED`]).
    engine_l: GrainEngine,
    engine_r: GrainEngine,
    /// Wet accumulation buses (pre-allocated to the max block size).
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
    /// Sink for the pan-opposite engine outputs (each engine renders a
    /// stereo pair; only its own channel's side is kept).
    discard: Vec<f32>,
}

impl GranularDsp {
    pub fn new(sample_rate: f32, max_block: usize) -> Self {
        let ring_len = ((MAX_DELAY_SECONDS * sample_rate) as usize + 1).next_power_of_two();
        let max_block = max_block.max(1);
        Self {
            sample_rate,
            buf_l: vec![0.0; ring_len],
            buf_r: vec![0.0; ring_len],
            mask: ring_len - 1,
            write_pos: 0,
            engine_l: GrainEngine::new(sample_rate, ENGINE_SEED),
            engine_r: GrainEngine::new(sample_rate, ENGINE_SEED),
            wet_l: vec![0.0; max_block],
            wet_r: vec![0.0; max_block],
            discard: vec![0.0; max_block],
        }
    }

    /// Ring-buffer length in seconds (>= [`MAX_DELAY_SECONDS`]).
    pub fn buffer_seconds(&self) -> f32 {
        self.buf_l.len() as f32 / self.sample_rate
    }

    /// Total grains spawned since construction (metering aid for tests
    /// and the future editor, ba todo #1079). The engines are in
    /// lockstep, so either count is authoritative.
    pub fn grains_spawned(&self) -> u64 {
        self.engine_l.grains_spawned()
    }

    /// Currently sounding grains (metering aid).
    pub fn active_grains(&self) -> usize {
        self.engine_l.active_grains()
    }

    /// Silence the buffer and all grains; allocation-free apart from the
    /// buffer zeroing (called from `reset`, off the steady-state path).
    pub fn clear(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write_pos = 0;
        self.engine_l.reset();
        self.engine_r.reset();
    }

    /// Render one block in place. `left`/`right` arrive carrying the dry
    /// input and leave carrying the equal-power dry/wet mix.
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        smoothers: &mut GranularSmoothers,
        params: &BlockParams,
    ) {
        let frames = frames.min(left.len()).min(right.len()).min(self.wet_l.len());
        if frames == 0 {
            return;
        }

        // --- 1. Write the dry input into the circular buffer. ---------
        // TODO(epic-196 #1074): the feedback loop taps in here — the
        // (filtered, soft-clipped, DC-blocked) wet bus from the previous
        // block is summed with the dry input before the write, or routed
        // to the output mix only, per `fb_route`.
        // TODO(epic-196 #1075): freeze gates this write (crossfaded).
        let base = self.write_pos as usize;
        for i in 0..frames {
            let idx = (base + i) & self.mask;
            self.buf_l[idx] = left[i];
            self.buf_r[idx] = right[i];
        }

        // --- 2. Granulate behind the write head into the wet bus. -----
        self.wet_l[..frames].fill(0.0);
        self.wet_r[..frames].fill(0.0);
        self.discard[..frames].fill(0.0);

        let grain_params = GrainParams {
            density_hz: params.density_hz,
            // Bound grain length by the buffer so slow/reversed grains
            // can never be lapped by the write head (doc #252 §5); the
            // engine enforces the exact per-grain collision guard.
            grain_seconds: params
                .grain_seconds
                .min(self.buffer_seconds() * 0.5),
            position_seconds: params.delay_seconds,
            // Spray never reaches past the grain's own delay: jittered
            // positions are pre-bounded here and exactly clamped into
            // the safe zone by the engine's spawn guard.
            position_jitter_seconds: params.spray_seconds.min(params.delay_seconds),
            size_jitter: params.size_jitter,
            level_jitter: params.level_jitter,
            pan_spread: params.pan_spread,
            texture: params.texture,
            mode: params.scheduler,
            head_advance: 1.0, // streaming delay buffer
            pitch_semitones: params.pitch_semitones,
            detune_spread_cents: params.detune_spread_cents,
            reverse_probability: params.reverse_probability,
            anti_alias: params.anti_alias,
            // Later epic-196 todos grow `GrainParams` (e.g. #1080's
            // alignment fields); default the rest so this literal stays
            // source-compatible as the engine evolves.
            ..GrainParams::default()
        };

        let write_pos = self.write_pos as f64;
        // Each engine renders the full stereo pan pair for its channel;
        // keeping engine L's left and engine R's right applies the
        // per-grain constant-power pan as a stereo balance.
        {
            let (wet_l, discard) = (&mut self.wet_l[..frames], &mut self.discard[..frames]);
            self.engine_l
                .process(&self.buf_l, write_pos, &grain_params, wet_l, discard);
        }
        {
            let (discard, wet_r) = (&mut self.discard[..frames], &mut self.wet_r[..frames]);
            self.engine_r
                .process(&self.buf_r, write_pos, &grain_params, discard, wet_r);
        }

        // --- 3. Equal-power dry/wet mix (doc #252 §9), dry untouched
        // otherwise. TODO(epic-196 #1077): M/S width goes here, on the
        // wet bus only.
        for i in 0..frames {
            let mix = smoothers.mix.next().clamp(0.0, 1.0);
            let dry_gain = (1.0 - mix).sqrt();
            let wet_gain = mix.sqrt();
            left[i] = left[i] * dry_gain + self.wet_l[i] * wet_gain;
            right[i] = right[i] * dry_gain + self.wet_r[i] * wet_gain;
        }

        self.write_pos += frames as u64;
    }
}
