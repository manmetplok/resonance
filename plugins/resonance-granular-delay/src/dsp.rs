//! Granular-delay DSP core: a power-of-two stereo circular buffer, one
//! write head, and lock-stepped left/right grain engines granulating
//! behind it (doc #252 §1/§5/§8, doc #253), plus the stereo stage
//! (ba todo #1077): a decorrelated right-channel engine that fades in
//! whenever Pan Spread is non-zero, M/S width on the wet sum and a
//! ping-pong feedback route.
//!
//! Everything is pre-allocated at construction; `process_block` performs
//! no allocation and takes no locks.

use resonance_dsp::{DcBlocker, GrainEngine, GrainParams, OnePole, SchedulerMode};

use crate::params::GranularSmoothers;

/// Maximum delay time the ring buffer is sized for at activation.
pub const MAX_DELAY_SECONDS: f32 = 4.0;

/// Freeze engage/resume ramp length (ba todo #1075, doc #252 §1): the
/// write gain crossfades over this many seconds so stopping/restarting
/// the write head never splices a discontinuity into the buffer.
pub const FREEZE_RAMP_SECONDS: f32 = 0.005;

/// Shared seed for the two lock-stepped grain engines. Both engines
/// must consume identical RNG streams so left and right render the
/// *same* grain cloud over their respective channels (stereo balance
/// per grain) — the mono-compatible mode, active while Pan Spread is 0.
const ENGINE_SEED: u64 = 0x5EED_6417;

/// Independent seed for the decorrelated right-channel engine (ba todo
/// #1077, doc #252 §5): with Pan Spread > 0 the right wet bus crossfades
/// to a grain cloud whose jitter/scheduling stream is split from
/// [`ENGINE_SEED`], so left and right content genuinely decorrelates.
/// Pan Spread = 0 fades back to the lock-stepped pair, keeping the
/// mono-compatible mode reachable.
const DECOR_SEED: u64 = 0xD3C0_44E1;

/// Length of the lock-stepped ↔ decorrelated crossfade, milliseconds
/// (equal-power, applied per sample off a smoother).
pub const DECOR_FADE_MS: f32 = 50.0;

/// Feedback topology (doc #252 §1, ba todo #1074).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbRoute {
    /// Default: the granulated wet output is filtered, soft-clipped,
    /// DC-blocked and summed with the dry input at the buffer write
    /// point, so every recirculation is re-granulated (the defining
    /// granular-delay sound).
    WetToBuffer,
    /// "Clean repeats": the buffer receives the dry input only; the
    /// feedback recirculates in the *output* mix through a dedicated
    /// wet-recirculation ring read at the delay time.
    OutputOnly,
    /// Wet→Buffer with the channels crossed at the feedback write tap
    /// (ba todo #1077, doc #252 §5): each recirculation the conditioned
    /// left wet feeds the right buffer input and vice versa, so repeats
    /// alternate sides.
    PingPong,
}

/// One channel of the in-loop feedback conditioning chain (doc #252 §5):
/// damping filter (LP, or HP as input-minus-LP so the one-pole state
/// stays valid across type switches) → tanh soft clip → DC blocker.
/// The tanh bounds the recirculated signal to ±1 no matter the loop
/// gain, which is what keeps over-unity (>100 %) feedback stable.
struct FeedbackChain {
    filter: OnePole,
    dc: DcBlocker,
}

impl FeedbackChain {
    fn new() -> Self {
        Self {
            filter: OnePole::new(),
            dc: DcBlocker::default(),
        }
    }

    fn reset(&mut self) {
        self.filter.clear();
        self.dc.reset();
    }

    #[inline(always)]
    fn process(&mut self, x: f32, highpass: bool) -> f32 {
        let lp = self.filter.process(x);
        let damped = if highpass { x - lp } else { lp };
        self.dc.process(damped.tanh())
    }
}

/// Block-level parameters resolved once per process call in `lib.rs`.
/// Grain-latched values need no smoothing (latched per grain at spawn);
/// the smoothed wet/dry mix, feedback amount and damping cutoff come
/// off [`GranularSmoothers`].
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
    /// Feedback topology (ba todo #1074).
    pub fb_route: FbRoute,
    /// Damping filter type in the feedback loop: LP (false) or HP.
    pub filter_is_highpass: bool,
    /// Freeze/hold (ba todo #1075): stop the write head and hold the
    /// buffer; grains keep reading the static content. Engage and
    /// resume are equal-power crossfades on the write gain.
    pub freeze: bool,
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
    /// Decorrelated right-channel engine (see [`DECOR_SEED`]): rendered
    /// whenever Pan Spread > 0 (or while its tail drains) and blended
    /// into the right wet bus with an equal-power crossfade.
    engine_r_decor: GrainEngine,
    /// Wet accumulation buses (pre-allocated to the max block size).
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
    /// Decorrelated right-channel wet bus, blended into `wet_r`.
    wet_r_decor: Vec<f32>,
    /// Sink for the pan-opposite engine outputs (each engine renders a
    /// stereo pair; only its own channel's side is kept).
    discard: Vec<f32>,
    /// Wet→Buffer feedback bus: the conditioned (damped, soft-clipped,
    /// DC-blocked) wet output of the *previous* block, summed with the
    /// dry input at the write point of the current block. The one-block
    /// loop latency is far below the minimum grain delay (10 ms), so it
    /// is inaudible in the repeat spacing (ba todo #1074).
    fb_l: Vec<f32>,
    fb_r: Vec<f32>,
    /// Valid prefix of `fb_l`/`fb_r` (0 when the previous block ran the
    /// Output-only route; shrinks safely if the host varies block size).
    fb_len: usize,
    /// Output-only recirculation rings (same length/mask as the source
    /// buffers): hold the wet-path output so "clean repeats" can
    /// recirculate at the delay time without touching the grain source
    /// buffer. Written every block regardless of route so switching
    /// topologies is seamless.
    fb_ring_l: Vec<f32>,
    fb_ring_r: Vec<f32>,
    /// In-loop conditioning (shared by both topologies; only one route
    /// runs per block).
    fb_chain_l: FeedbackChain,
    fb_chain_r: FeedbackChain,
    /// Recirculation-time counter for the feedback stage: identical to
    /// `write_pos` while streaming, but it keeps advancing while frozen
    /// so the Output-only recirc ring keeps its own time axis when the
    /// write head stops (ba todo #1075).
    fb_pos: u64,
    /// Freeze crossfade position: 0 = live (writes at full gain), 1 =
    /// fully frozen (write head stopped, buffer untouched). Ramps by
    /// one sample step per input sample toward the freeze target, and
    /// the write is an equal-power blend of held content and incoming
    /// signal while in between (ba todo #1075).
    freeze_xf: f32,
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
            engine_r_decor: GrainEngine::new(sample_rate, DECOR_SEED),
            wet_l: vec![0.0; max_block],
            wet_r: vec![0.0; max_block],
            wet_r_decor: vec![0.0; max_block],
            discard: vec![0.0; max_block],
            fb_l: vec![0.0; max_block],
            fb_r: vec![0.0; max_block],
            fb_len: 0,
            fb_ring_l: vec![0.0; ring_len],
            fb_ring_r: vec![0.0; ring_len],
            fb_chain_l: FeedbackChain::new(),
            fb_chain_r: FeedbackChain::new(),
            fb_pos: 0,
            freeze_xf: 0.0,
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
        self.engine_r_decor.reset();
        self.wet_r_decor.fill(0.0);
        self.fb_l.fill(0.0);
        self.fb_r.fill(0.0);
        self.fb_len = 0;
        self.fb_ring_l.fill(0.0);
        self.fb_ring_r.fill(0.0);
        self.fb_chain_l.reset();
        self.fb_chain_r.reset();
        self.fb_pos = 0;
        self.freeze_xf = 0.0;
    }

    /// Read-only view of the left grain source ring (test/metering aid:
    /// the Output-only route must keep this identical to the dry input,
    /// and freeze must hold it bit-stable). Sample `n` of the stream
    /// lives at index `n & (ring_len - 1)` while it remains in range.
    pub fn ring_l(&self) -> &[f32] {
        &self.buf_l
    }

    /// Read-only view of the right grain source ring (see [`Self::ring_l`]).
    pub fn ring_r(&self) -> &[f32] {
        &self.buf_r
    }

    /// Absolute write-head position in samples (test/metering aid).
    pub fn write_head(&self) -> u64 {
        self.write_pos
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

        // Damping cutoff: block-rate coefficient update from the
        // smoothed value, sample-rate application (doc #252 §5).
        smoothers.filter_hz.skip(frames as u32);
        let cutoff = smoothers.filter_hz.current().clamp(20.0, 20_000.0);
        self.fb_chain_l.filter.set_cutoff(cutoff, self.sample_rate);
        self.fb_chain_r.filter.set_cutoff(cutoff, self.sample_rate);

        // --- 1. Write into the circular buffer (ba todo #1074): the
        // dry input, plus — on the Wet→Buffer route — the conditioned
        // wet bus of the previous block, so each recirculation is
        // re-granulated. The Output-only route keeps the buffer clean.
        //
        // Freeze (ba todo #1075) gates this whole write — dry *and*
        // feedback, so a frozen buffer cannot run away no matter the
        // loop gain. `freeze_xf` ramps per sample; while in between the
        // write is an equal-power blend of the held content and the
        // incoming signal (the engage ramp thereby morphs the recorded
        // stream into the lap-old material ahead of the stop point, and
        // resume morphs back out of it, so the boundary is always
        // splice-free), and once fully frozen the write head stops
        // (samples are neither written nor consumed for head advance)
        // and the buffer is left bit-untouched. Because the head is
        // static, grain read origins (`write_pos - delay`) become
        // absolute buffer offsets — grains do not chase a stopped head.
        let base = self.write_pos as usize;
        let wet_to_buffer = matches!(
            params.fb_route,
            FbRoute::WetToBuffer | FbRoute::PingPong
        );
        let freeze_target: f32 = if params.freeze { 1.0 } else { 0.0 };
        let freeze_step = 1.0 / (FREEZE_RAMP_SECONDS * self.sample_rate).max(1.0);
        let mut advanced = 0usize;
        for i in 0..frames {
            if self.freeze_xf < freeze_target {
                self.freeze_xf = (self.freeze_xf + freeze_step).min(1.0);
            } else if self.freeze_xf > freeze_target {
                self.freeze_xf = (self.freeze_xf - freeze_step).max(0.0);
            }
            if self.freeze_xf >= 1.0 {
                continue; // fully frozen: hold the buffer, stop the head
            }
            let idx = (base + advanced) & self.mask;
            let (mut in_l, mut in_r) = (left[i], right[i]);
            if wet_to_buffer && i < self.fb_len {
                in_l += self.fb_l[i];
                in_r += self.fb_r[i];
            }
            if self.freeze_xf > 0.0 {
                // Engage/resume ramp: equal-power blend of held content
                // and the incoming stream, click-free at both ends.
                let phase = std::f32::consts::FRAC_PI_2 * self.freeze_xf;
                let (keep_g, write_g) = phase.sin_cos();
                self.buf_l[idx] = self.buf_l[idx] * keep_g + in_l * write_g;
                self.buf_r[idx] = self.buf_r[idx] * keep_g + in_r * write_g;
            } else {
                self.buf_l[idx] = in_l;
                self.buf_r[idx] = in_r;
            }
            advanced += 1;
        }

        // --- 2. Granulate behind the write head into the wet bus. -----
        self.wet_l[..frames].fill(0.0);
        self.wet_r[..frames].fill(0.0);
        self.wet_r_decor[..frames].fill(0.0);
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
            // Streaming: 1.0. Frozen: 0.0 — the engine's static-corpus
            // mode, so spawn guards and read origins track the stopped
            // head (ba todo #1075). Transitional blocks pass the actual
            // fraction the head moved.
            head_advance: advanced as f32 / frames as f32,
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

        // --- 2b. Decorrelated right channel (ba todo #1077): with Pan
        // Spread > 0 an independently seeded engine renders its own
        // cloud over the right buffer, and the right wet bus crossfades
        // (equal-power, smoothed) from the lock-stepped cloud to it.
        // When the spread returns to 0 the engine stops spawning but
        // keeps rendering (density 0) until its live grains finish, so
        // re-engaging never resumes stale grains; once drained and the
        // fade has settled it costs nothing.
        let decor_gate = params.pan_spread > 0.0;
        if decor_gate
            || smoothers.decor.current() > 0.0
            || self.engine_r_decor.active_grains() > 0
        {
            let drain_params;
            let gp = if decor_gate {
                &grain_params
            } else {
                drain_params = GrainParams {
                    density_hz: 0.0,
                    ..grain_params.clone()
                };
                &drain_params
            };
            let (discard, wet_dec) =
                (&mut self.discard[..frames], &mut self.wet_r_decor[..frames]);
            self.engine_r_decor
                .process(&self.buf_r, write_pos, gp, discard, wet_dec);
        }
        if !decor_gate && smoothers.decor.current() == 0.0 {
            // Fully settled in lock-stepped mode: no blend work.
            smoothers.decor.skip(frames as u32);
        } else {
            for i in 0..frames {
                let d = smoothers.decor.next().clamp(0.0, 1.0);
                let phase = std::f32::consts::FRAC_PI_2 * d;
                let (g_decor, g_lock) = phase.sin_cos();
                self.wet_r[i] = self.wet_r[i] * g_lock + self.wet_r_decor[i] * g_decor;
            }
        }

        // --- 3. Feedback conditioning (ba todo #1074): wet × feedback →
        // damping filter → tanh soft clip → DC blocker. The tanh bounds
        // the recirculated signal regardless of loop gain, which is what
        // keeps the over-unity (up to 110 %) range stable; the DC
        // blocker stops offset accumulating across recirculations.
        // The recirc ring runs on its own clock (`fb_pos`): identical
        // to the write head while streaming, but it keeps ticking while
        // frozen so Output-only repeats stay on their own time axis
        // instead of stalling with the stopped head (ba todo #1075).
        let fb_base = self.fb_pos as usize;
        match params.fb_route {
            FbRoute::WetToBuffer | FbRoute::PingPong => {
                // Condition this block's wet bus into the feedback bus
                // consumed at the next block's write point, and keep the
                // recirculation ring warm so a route switch is seamless.
                // Ping-pong (ba todo #1077) swaps the channels right
                // here at the feedback write tap, so every
                // recirculation crosses sides.
                let cross = params.fb_route == FbRoute::PingPong;
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let (src_l, src_r) = if cross {
                        (self.wet_r[i], self.wet_l[i])
                    } else {
                        (self.wet_l[i], self.wet_r[i])
                    };
                    self.fb_l[i] = self
                        .fb_chain_l
                        .process(src_l * g, params.filter_is_highpass);
                    self.fb_r[i] = self
                        .fb_chain_r
                        .process(src_r * g, params.filter_is_highpass);
                    let idx = (fb_base + i) & self.mask;
                    self.fb_ring_l[idx] = self.wet_l[i];
                    self.fb_ring_r[idx] = self.wet_r[i];
                }
                self.fb_len = frames;
            }
            FbRoute::OutputOnly => {
                // Clean repeats: recirculate the wet-path output through
                // a dedicated ring read at the delay time; the grain
                // source buffer never sees wet material. Bounded even
                // while frozen: the loop still passes through the tanh.
                let delay_samples = ((params.delay_seconds * self.sample_rate) as usize)
                    .clamp(1, self.mask);
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let idx = (fb_base + i) & self.mask;
                    let ridx = (fb_base + i).wrapping_sub(delay_samples) & self.mask;
                    let fl = self
                        .fb_chain_l
                        .process(self.fb_ring_l[ridx] * g, params.filter_is_highpass);
                    let fr = self
                        .fb_chain_r
                        .process(self.fb_ring_r[ridx] * g, params.filter_is_highpass);
                    self.fb_ring_l[idx] = self.wet_l[i] + fl;
                    self.fb_ring_r[idx] = self.wet_r[i] + fr;
                    self.wet_l[i] += fl;
                    self.wet_r[i] += fr;
                }
                self.fb_len = 0;
            }
        }

        // --- 4. M/S width on the wet sum only (ba todo #1077, doc #252
        // §5), then the equal-power dry/wet mix (doc #252 §9); the dry
        // path stays bit-exact regardless of the stereo processing.
        // Width sits *after* the feedback tap, so the loop recirculates
        // the un-widened wet and the width control cannot destabilise
        // it. Width 0 collapses the wet to its mid signal (L == R).
        for i in 0..frames {
            let width = smoothers.width.next().clamp(0.0, 1.5);
            let mid = 0.5 * (self.wet_l[i] + self.wet_r[i]);
            let side = 0.5 * (self.wet_l[i] - self.wet_r[i]) * width;
            let mix = smoothers.mix.next().clamp(0.0, 1.0);
            let dry_gain = (1.0 - mix).sqrt();
            let wet_gain = mix.sqrt();
            left[i] = left[i] * dry_gain + (mid + side) * wet_gain;
            right[i] = right[i] * dry_gain + (mid - side) * wet_gain;
        }

        // The write head advances only by the samples actually written
        // (it stops while frozen); the recirc clock always advances.
        self.write_pos += advanced as u64;
        self.fb_pos += frames as u64;
    }
}
