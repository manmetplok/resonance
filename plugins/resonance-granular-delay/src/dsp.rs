//! Granular-delay DSP core: a power-of-two stereo circular buffer, one
//! write head, and lock-stepped left/right grain engines granulating
//! behind it (doc #252 §1/§5/§8, doc #253), plus the stereo stage
//! (ba todo #1077): a decorrelated right-channel engine that fades in
//! whenever Pan Spread is non-zero, M/S width on the wet sum and a
//! ping-pong feedback route; plus shimmer and pitch quantization
//! (ba todo #1078): FB Pitch selects whether the granulated-feedback
//! tap carries the transposed wet (cumulative octave-climb repeats) or
//! an un-transposed re-granulation (constant-pitch repeats), and the
//! per-grain effective transpose can be quantized to semitones or a
//! scale at spawn (see `crate::quantize`).
//!
//! Everything is pre-allocated at construction; `process_block` performs
//! no allocation and takes no locks.

use resonance_dsp::{
    read_hermite_wrapped, DcBlocker, GrainEngine, GrainParams, OnePole, SchedulerMode, SimpleRng,
    SwapFader,
};
use resonance_music_theory::Scale;

use crate::params::GranularSmoothers;
use crate::pitch_sync::{PitchSyncGranulator, SpawnMode};
use crate::quantize::{quantize_transpose, PitchQuantize};

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

/// Shared seed for the lock-stepped feedback-tap engine pair (ba todo
/// #1078): with FB Pitch off, these render the *un-transposed*
/// re-granulation that recirculates, so repeats keep a constant pitch
/// while the audible wet stays transposed.
const FB_TAP_SEED: u64 = 0xFBFB_7A93;

/// Seed for the plugin-side quantized-transpose draws (ba todo #1078).
const QUANT_SEED: u64 = 0x0AB5_C41E;

/// Engine-render slice length while pitch quantization is active, in
/// samples: the block is processed in slices no longer than this, each
/// with its own independently drawn, quantized effective transpose, so
/// grains latch (near-)independent quantized values at spawn. The
/// slice is far shorter than the minimum inter-onset time at maximum
/// density (480 samples at 100 grains/s, 48 kHz), so two grains almost
/// never share a draw. Engine output is slice-invariant (onset
/// scheduling carries across process calls), so quantize-off behaviour
/// is untouched.
const QUANT_SLICE: usize = 64;

/// Fade time mode (ba todo #1076): length of each `SwapFader` leg,
/// seconds. The full transition — wet fades out, the tap swaps on the
/// silent sample, the new origin fades back in — takes twice this,
/// ~20 ms (doc #252 §5: too-long fades color, too-short ones glitch).
pub const FADE_LEG_SECONDS: f32 = 0.010;

/// Repitch time mode (ba todo #1076): one-pole slew time constant of
/// the effective delay, seconds (tape/BBD-style glide).
pub const REPITCH_TAU_SECONDS: f32 = 0.100;

/// Delay-target changes below this are ignored by the Fade swap
/// trigger, seconds — keeps host tempo jitter from re-arming fades.
const TIME_EPSILON_SECONDS: f32 = 1.0e-4;

/// Once the Repitch slew is within this of the target it snaps,
/// seconds (sub-sample at any supported rate).
const REPITCH_SNAP_SECONDS: f32 = 1.0e-5;

/// Clamp on the Repitch playback-rate multiplier `1 − d(delay)/dt`
/// (± two octaves), bounding the swoop on extreme jumps.
const REPITCH_RATE_MIN: f64 = 0.25;
const REPITCH_RATE_MAX: f64 = 4.0;

/// Equal-power crossfade length between the async grain cloud and the
/// pitch-synchronous PSOLA bus (ba todo #1082), seconds. Long enough
/// for the fallback cloud to rebuild some overlap, short enough that
/// voiced/unvoiced handovers feel immediate.
pub const VOICE_FADE_SECONDS: f32 = 0.05;

/// Delay-time change behaviour (doc #252 §5, ba todo #1076).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeMode {
    /// Dual-tap swap via [`SwapFader`]: on a time change the whole
    /// granulated wet fades out over [`FADE_LEG_SECONDS`], the tap
    /// jumps on the silent sample (every in-flight grain is retired
    /// there, click-free by construction) and the new origin fades
    /// back in.
    Fade,
    /// Tape/BBD: the effective delay slews toward the target (one-pole,
    /// [`REPITCH_TAU_SECONDS`]) and grains spawned during the slew take
    /// the matching playback-rate offset `1 − d(delay)/dt` — the
    /// tape-style momentary pitch swoop, settling back to unity.
    Repitch,
    /// Default, uniquely granular: a time change affects only newly
    /// spawned grains; in-flight grains finish at their old origin —
    /// artifact-free by construction.
    PerGrain,
}

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
    /// How delay-time changes are realized (ba todo #1076).
    pub time_mode: TimeMode,
    /// Scheduler = Pitch-Sync (ba todo #1082): run the pitch tracker on
    /// the written input and, while it is voiced, granulate PSOLA-style
    /// (marker-snapped two-period Hann voices, transposition by onset
    /// spacing). Unvoiced/unlocked spans fall back to the async cloud.
    pub pitch_sync: bool,
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
    /// Shimmer switch (ba todo #1078): true = the transposed wet
    /// recirculates (each pass compounds the transpose); false = the
    /// feedback tap re-granulates without transpose (constant-pitch
    /// repeats).
    pub fb_pitch: bool,
    /// Per-grain transpose quantization at spawn (ba todo #1078).
    pub quantize: PitchQuantize,
    /// Root/mode for [`PitchQuantize::Scale`].
    pub scale: Scale,
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
    /// Lock-stepped feedback-tap engines (ba todo #1078, see
    /// [`FB_TAP_SEED`]): render the un-transposed re-granulation that
    /// recirculates while FB Pitch is off and a transpose is engaged.
    fb_engine_l: GrainEngine,
    fb_engine_r: GrainEngine,
    /// Feedback-tap buses for the un-transposed re-granulation.
    fbw_l: Vec<f32>,
    fbw_r: Vec<f32>,
    /// RNG for the plugin-side quantized-transpose draws (ba todo
    /// #1078): while quantization is on, the engines' own detune draw
    /// is bypassed (spread passed as 0) and the effective transpose is
    /// drawn and quantized here, per render slice.
    quant_rng: SimpleRng,
    /// Sign of the next quantized-spread draw; alternates like the
    /// engine's detune sign so the quantized cloud stays symmetric.
    quant_sign: f32,
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
    /// Pitch-synchronous Voice/Mono scheduler (ba todo #1082): tracker,
    /// marker ring and PSOLA voice pool.
    psola: PitchSyncGranulator,
    /// PSOLA wet buses, blended into `wet_l`/`wet_r`.
    psola_l: Vec<f32>,
    psola_r: Vec<f32>,
    /// Mono (mid) scratch of the dry input samples actually written
    /// this block — what the tracker is fed, so marker positions map
    /// 1:1 onto write-stream/buffer positions.
    track_in: Vec<f32>,
    /// Equal-power crossfade position between the async cloud (0) and
    /// the PSOLA bus (1); ramps per sample over [`VOICE_FADE_SECONDS`].
    voice_xf: f32,
    /// Last block's engage decision (tracker voiced + usable marker
    /// near the tap) — metering/test aid.
    psola_engaged: bool,
    /// Fade time mode (ba todo #1076): the dual-tap swap machine. The
    /// payload is the delay value (seconds) the wet path is committed
    /// to; a target change swaps toward the new value through silence.
    time_fade: SwapFader<f32>,
    /// Most recent value handed to the fader (active or pending) — the
    /// swap re-trigger reference.
    fade_goal: f32,
    /// True while a Fade transition (either leg) is still in flight.
    fade_busy: bool,
    /// Per-sample effective delay for the block, seconds (filled by the
    /// Fade/Repitch pre-pass; unused in Per-Grain mode).
    time_delay_buf: Vec<f32>,
    /// Per-sample wet gain from the Fade swap (1.0 outside fades).
    time_gain_buf: Vec<f32>,
    /// Current effective delay, seconds: the resolved target in
    /// Per-Grain mode, the slewed value in Repitch, the fader's active
    /// payload in Fade. `f64` so the one-pole slew increment never
    /// stalls below the mantissa step of the value itself.
    eff_delay: f64,
    /// False until the first block latches the initial delay target
    /// (so activation never fades/slews from an arbitrary value).
    time_primed: bool,
    /// Per-sample one-pole coefficient of the Repitch slew.
    repitch_coeff: f64,
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
            fb_engine_l: GrainEngine::new(sample_rate, FB_TAP_SEED),
            fb_engine_r: GrainEngine::new(sample_rate, FB_TAP_SEED),
            fbw_l: vec![0.0; max_block],
            fbw_r: vec![0.0; max_block],
            quant_rng: SimpleRng::new(QUANT_SEED),
            quant_sign: 1.0,
            discard: vec![0.0; max_block],
            fb_l: vec![0.0; max_block],
            fb_r: vec![0.0; max_block],
            fb_len: 0,
            fb_ring_l: vec![0.0; ring_len],
            fb_ring_r: vec![0.0; ring_len],
            fb_chain_l: FeedbackChain::new(),
            fb_chain_r: FeedbackChain::new(),
            fb_pos: 0,
            psola: PitchSyncGranulator::new(sample_rate),
            psola_l: vec![0.0; max_block],
            psola_r: vec![0.0; max_block],
            track_in: vec![0.0; max_block],
            voice_xf: 0.0,
            psola_engaged: false,
            time_fade: SwapFader::new(Self::fade_leg_samples(sample_rate)),
            fade_goal: 0.0,
            fade_busy: false,
            time_delay_buf: vec![0.0; max_block],
            time_gain_buf: vec![1.0; max_block],
            eff_delay: 0.0,
            time_primed: false,
            repitch_coeff: 1.0 - (-1.0 / f64::from(REPITCH_TAU_SECONDS * sample_rate)).exp(),
            freeze_xf: 0.0,
        }
    }

    /// Samples per Fade leg at `sample_rate` (never zero).
    fn fade_leg_samples(sample_rate: f32) -> u32 {
        ((FADE_LEG_SECONDS * sample_rate) as u32).max(1)
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

    /// Playback rates of the currently sounding audible grains (left
    /// lock-stepped engine; test/metering aid). With pitch quantization
    /// on, every rate sits on the quantized semitone/scale lattice
    /// (ba todo #1078).
    pub fn active_rates(&self) -> impl Iterator<Item = f64> + '_ {
        self.engine_l.active_rates()
    }

    /// Playback rates of the decorrelated right engine's grains
    /// (test/metering aid; ba todos #1077/#1078).
    pub fn active_rates_decor(&self) -> impl Iterator<Item = f64> + '_ {
        self.engine_r_decor.active_rates()
    }

    /// Playback rates of the feedback-tap engine's grains — all ±1
    /// while the un-transposed tap is active (test/metering aid,
    /// ba todo #1078).
    pub fn active_rates_fb(&self) -> impl Iterator<Item = f64> + '_ {
        self.fb_engine_l.active_rates()
    }

    /// Uniform draw in `[0, 1)` for the plugin-side quantized-spread
    /// magnitude (same mapping as the engine's own RNG draws).
    fn quant_unit(&mut self) -> f32 {
        (self.quant_rng.next_u32() >> 8) as f32 * (1.0 / (1 << 24) as f32)
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
        self.fb_engine_l.reset();
        self.fb_engine_r.reset();
        self.fbw_l.fill(0.0);
        self.fbw_r.fill(0.0);
        self.quant_rng = SimpleRng::new(QUANT_SEED);
        self.quant_sign = 1.0;
        self.fb_l.fill(0.0);
        self.fb_r.fill(0.0);
        self.fb_len = 0;
        self.fb_ring_l.fill(0.0);
        self.fb_ring_r.fill(0.0);
        self.fb_chain_l.reset();
        self.fb_chain_r.reset();
        self.fb_pos = 0;
        self.psola.reset();
        self.psola_l.fill(0.0);
        self.psola_r.fill(0.0);
        self.voice_xf = 0.0;
        self.psola_engaged = false;
        // `SwapFader::new` is allocation-free, so rebuilding it here is
        // the cheapest full reset (it has no reset method).
        self.time_fade = SwapFader::new(Self::fade_leg_samples(self.sample_rate));
        self.fade_goal = 0.0;
        self.fade_busy = false;
        self.eff_delay = 0.0;
        self.time_primed = false;
        self.freeze_xf = 0.0;
    }

    /// Current effective delay-tap position, seconds (test/metering
    /// aid, ba todo #1076): equals the resolved target in Per-Grain
    /// mode, glides monotonically toward it in Repitch mode and steps
    /// on the (silent) swap sample in Fade mode.
    pub fn effective_delay_seconds(&self) -> f32 {
        self.eff_delay as f32
    }

    /// Whether the pitch-synchronous scheduler was engaged (tracker
    /// voiced + usable pitch mark near the tap) on the last block
    /// (test/metering aid, ba todo #1082).
    pub fn pitch_sync_engaged(&self) -> bool {
        self.psola_engaged
    }

    /// Total PSOLA onsets spawned (test/metering aid).
    pub fn psola_onsets(&self) -> u64 {
        self.psola.onsets()
    }

    /// Currently sounding PSOLA voices (test/metering aid).
    pub fn psola_active_voices(&self) -> usize {
        self.psola.active_voices()
    }

    /// The most recent PSOLA onset times in absolute output samples,
    /// oldest first (up to 64). Test aid — allocates, keep off the
    /// audio path.
    pub fn psola_recent_onsets(&self) -> Vec<f64> {
        self.psola.recent_onsets()
    }

    /// Last known tracked fundamental period in full-rate samples
    /// (0 before the first voiced lock; held across unvoiced spans and
    /// freeze). Test/metering aid.
    pub fn tracked_period_samples(&self) -> f32 {
        self.psola.period_samples() as f32
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
        let sr = f64::from(self.sample_rate);

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
            if params.pitch_sync {
                // The tracker is fed the *dry* mid of exactly the
                // samples written, so its markers map 1:1 onto
                // write-stream positions (ba todo #1082).
                self.track_in[advanced] = 0.5 * (left[i] + right[i]);
            }
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

        // --- 1b. Time-mode resolution (ba todo #1076, doc #252 §5):
        // decide the per-sample effective delay (and, in Fade mode, the
        // per-sample wet gain) this block renders with. Per-Grain
        // passes the target straight through — bit-identical to the
        // pre-#1076 behaviour.
        let target = params.delay_seconds;
        let target64 = f64::from(target);
        if !self.time_primed {
            self.time_primed = true;
            self.eff_delay = target64;
            self.time_fade.install(target);
            self.fade_goal = target;
            self.fade_busy = false;
        }
        // Sample index where a Fade swap lands this block (the silent
        // sample; every engine is hard-reset there so the old tap's
        // in-flight grains are retired without a click).
        let mut swap_at = usize::MAX;
        let mut fade_varying = false;
        let mut repitch_slewing = false;
        match params.time_mode {
            TimeMode::PerGrain => {
                // A time change affects only newly spawned grains;
                // in-flight grains finish at their old origin. Keep the
                // fader in sync so entering Fade mode later starts from
                // the current value instead of a stale one.
                self.eff_delay = target64;
                self.time_fade.install(target);
                self.fade_goal = target;
                self.fade_busy = false;
            }
            TimeMode::Repitch => {
                if (self.eff_delay - target64).abs() > f64::from(REPITCH_SNAP_SECONDS) {
                    repitch_slewing = true;
                    for slot in self.time_delay_buf[..frames].iter_mut() {
                        self.eff_delay += (target64 - self.eff_delay) * self.repitch_coeff;
                        *slot = self.eff_delay as f32;
                    }
                } else {
                    self.eff_delay = target64;
                }
                let eff = self.eff_delay as f32;
                self.time_fade.install(eff);
                self.fade_goal = eff;
                self.fade_busy = false;
            }
            TimeMode::Fade => {
                if (target - self.fade_goal).abs() > TIME_EPSILON_SECONDS {
                    self.time_fade.begin_swap(target);
                    self.fade_goal = target;
                    self.fade_busy = true;
                }
                if self.fade_busy {
                    fade_varying = true;
                    let mut active = self.eff_delay as f32;
                    let mut settled = true;
                    for i in 0..frames {
                        let (g, value) = self.time_fade.next();
                        let v = value.map_or(target, |v| *v);
                        if v != active {
                            swap_at = i;
                            active = v;
                        }
                        if g < 1.0 {
                            settled = false;
                        }
                        self.time_gain_buf[i] = g;
                        self.time_delay_buf[i] = v;
                    }
                    self.eff_delay = f64::from(active);
                    self.fade_busy = !settled;
                } else {
                    self.eff_delay = target64;
                }
            }
        }
        let time_varying = fade_varying || repitch_slewing;

        // --- 1c. Pitch-sync scheduler state (ba todo #1082, doc #252
        // §4): feed the tracker the written input, then decide whether
        // the Voice/Mono path is engaged this block — the tracker must
        // be voiced AND a stored pitch mark must lie near the delay
        // tap (so a freshly engaged mode without marker history simply
        // stays on the async cloud until the buffer has been analysed).
        // While frozen nothing is fed: the marker ring holds, the last
        // known period stands, and spawning continues from it against
        // the stalled head — the frozen drone keeps its pitch lattice.
        let mut engaged = false;
        if params.pitch_sync {
            self.psola.sync_to(self.write_pos);
            if advanced > 0 {
                self.psola.feed(&self.track_in[..advanced]);
            }
            let tap_seconds = if time_varying {
                self.time_delay_buf[0]
            } else {
                self.eff_delay as f32
            };
            let tap_target = self.write_pos as f64 - f64::from(tap_seconds) * sr;
            engaged = self.psola.voiced() && self.psola.has_marker_near(tap_target);
        }
        self.psola_engaged = engaged;
        let psola_render =
            params.pitch_sync || self.voice_xf > 0.0 || self.psola.active_voices() > 0;

        // --- 2. Granulate behind the write head into the wet bus. -----
        self.wet_l[..frames].fill(0.0);
        self.wet_r[..frames].fill(0.0);
        self.wet_r_decor[..frames].fill(0.0);
        self.discard[..frames].fill(0.0);

        let grain_params = GrainParams {
            // Voice/Mono engaged (ba todo #1082): the async cloud — and
            // its decorrelated and feedback-tap companions, which
            // inherit this density — drains until silent (no new
            // spawns, live grains finish) while the PSOLA bus takes
            // over, so a scheduler handover never resumes stale grains.
            density_hz: if engaged { 0.0 } else { params.density_hz },
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
        let head_adv = grain_params.head_advance as f64;

        // Decorrelated-engine disposition (ba todo #1077), fixed before
        // rendering: engaged while Pan Spread > 0; draining (no new
        // spawns, live grains finish) while the fade or its tail is
        // still audible; otherwise skipped entirely.
        let decor_gate = params.pan_spread > 0.0;
        let decor_render = decor_gate
            || smoothers.decor.current() > 0.0
            || self.engine_r_decor.active_grains() > 0;

        // Per-grain transpose quantization (ba todo #1078): with
        // quantization on, the engines' own detune draw is bypassed
        // (spread passed as 0) and the block renders in short slices,
        // each with its own plugin-side drawn and quantized effective
        // transpose (base Pitch + alternating-sign random Spread), so
        // grains latch (near-)independent quantized values at spawn.
        // The Repitch slew (ba todo #1076) uses the same short slices
        // so the gliding origin and rate offset stay smooth; a Fade
        // swap splits the block at the (silent) swap sample. Otherwise
        // the whole block is one slice; engine output is
        // slice-invariant (onset scheduling carries across process
        // calls), so behaviour is then unchanged.
        let quantize_on = params.quantize != PitchQuantize::Off;
        let slice_len = if quantize_on || repitch_slewing {
            QUANT_SLICE
        } else {
            frames
        };

        // Feedback-tap disposition (ba todo #1078), fixed before the
        // loop like the decorrelated engine's: rendered while the
        // un-transposed tap is engaged or while stale grains drain
        // (density 0) — a toggle never resumes stale grains and the
        // settled state costs nothing.
        let transpose_engaged =
            params.pitch_semitones != 0.0 || params.detune_spread_cents > 0.0;
        let unity_tap = wet_to_buffer && !params.fb_pitch && transpose_engaged;
        let fb_render = unity_tap
            || self.fb_engine_l.active_grains() > 0
            || self.fb_engine_r.active_grains() > 0;
        if fb_render {
            self.fbw_l[..frames].fill(0.0);
            self.fbw_r[..frames].fill(0.0);
        }

        let mut off = 0usize;
        while off < frames {
            if off == swap_at {
                // The Fade swap lands here, on the silent sample: hard-
                // retire the old tap's in-flight grains (click-free —
                // the wet gain is exactly 0 at this sample) so the new
                // origin fades in from a clean pool. The lock-stepped
                // pairs reset together, so they stay in lockstep.
                self.engine_l.reset();
                self.engine_r.reset();
                self.engine_r_decor.reset();
                self.fb_engine_l.reset();
                self.fb_engine_r.reset();
            }
            let mut n = (frames - off).min(slice_len);
            if swap_at > off && swap_at < off + n {
                n = swap_at - off;
            }
            let slice_delay = if time_varying {
                self.time_delay_buf[off]
            } else {
                self.eff_delay as f32
            };
            // Repitch (ba todo #1076): grains spawned during the slew
            // glide with the moving read origin, so they take the
            // matching playback-rate multiplier `1 − d(delay)/dt`
            // (delay growing ⇒ rate < 1 ⇒ pitch down, and vice versa).
            let repitch_semis = if repitch_slewing {
                let step_samples =
                    (target64 - f64::from(slice_delay)) * self.repitch_coeff * sr;
                let m = (1.0 - step_samples).clamp(REPITCH_RATE_MIN, REPITCH_RATE_MAX);
                (12.0 * m.log2()) as f32
            } else {
                0.0
            };
            let mut gp = grain_params.clone();
            gp.position_seconds = slice_delay;
            gp.position_jitter_seconds = params.spray_seconds.min(slice_delay);
            if quantize_on {
                let mut semitones = params.pitch_semitones;
                if params.detune_spread_cents > 0.0 {
                    let magnitude = self.quant_unit() * params.detune_spread_cents;
                    semitones += self.quant_sign * magnitude * (1.0 / 100.0);
                    self.quant_sign = -self.quant_sign;
                }
                gp.pitch_semitones =
                    quantize_transpose(semitones, params.quantize, params.scale);
                gp.detune_spread_cents = 0.0;
            }
            // The physical tape glide sits outside the musical
            // transpose, so it applies after quantization.
            gp.pitch_semitones += repitch_semis;
            let wp = write_pos + off as f64 * head_adv;
            // Each engine renders the full stereo pan pair for its
            // channel; keeping engine L's left and engine R's right
            // applies the per-grain constant-power pan as a stereo
            // balance.
            {
                let (wet_l, discard) =
                    (&mut self.wet_l[off..off + n], &mut self.discard[off..off + n]);
                self.engine_l.process(&self.buf_l, wp, &gp, wet_l, discard);
            }
            {
                let (discard, wet_r) =
                    (&mut self.discard[off..off + n], &mut self.wet_r[off..off + n]);
                self.engine_r.process(&self.buf_r, wp, &gp, discard, wet_r);
            }
            // --- 2b. Decorrelated right channel (ba todo #1077): with
            // Pan Spread > 0 an independently seeded engine renders its
            // own cloud over the right buffer, and the right wet bus
            // crossfades (equal-power, smoothed) from the lock-stepped
            // cloud to it. Once drained and the fade has settled it
            // costs nothing.
            if decor_render {
                let drain_params;
                let dgp = if decor_gate {
                    &gp
                } else {
                    drain_params = GrainParams {
                        density_hz: 0.0,
                        ..gp.clone()
                    };
                    &drain_params
                };
                let (discard, wet_dec) = (
                    &mut self.discard[off..off + n],
                    &mut self.wet_r_decor[off..off + n],
                );
                self.engine_r_decor
                    .process(&self.buf_r, wp, dgp, discard, wet_dec);
            }
            // --- 2c. Feedback-tap re-granulation (ba todo #1078, doc
            // #252 §3): with FB Pitch off on a granulated-feedback
            // route while a transpose is engaged, the signal written
            // back is a separate, *un-transposed* granulation of the
            // same buffer, so recirculations keep a constant pitch and
            // the transpose is heard exactly once. With FB Pitch on the
            // transposed wet bus itself is the tap. Rendered inside the
            // slice loop (slice-invariant) so the tap follows the
            // per-slice time-mode position and the Fade swap reset
            // (ba todo #1076).
            if fb_render {
                let mut fgp = gp.clone();
                // Zero the musical transpose (and any quantized draw)
                // but keep the physical Repitch glide — the tap rides
                // the same moving origin as the audible cloud.
                fgp.pitch_semitones = repitch_semis;
                fgp.detune_spread_cents = 0.0;
                if !unity_tap {
                    fgp.density_hz = 0.0; // drain, output unused
                }
                {
                    let (fbw_l, discard) =
                        (&mut self.fbw_l[off..off + n], &mut self.discard[off..off + n]);
                    self.fb_engine_l.process(&self.buf_l, wp, &fgp, fbw_l, discard);
                }
                {
                    let (discard, fbw_r) =
                        (&mut self.discard[off..off + n], &mut self.fbw_r[off..off + n]);
                    self.fb_engine_r.process(&self.buf_r, wp, &fgp, discard, fbw_r);
                }
            }
            off += n;
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

        // --- 2c-v. Pitch-synchronous PSOLA bus (ba todo #1082, doc
        // #252 §4). Engaged: onsets snap to the pitch mark nearest the
        // tap, voices are two-period Hann segments at unity rate, and
        // transposition is onset spacing (period / α) — formants
        // preserved, no AM beating. Disengaging: the granulator keeps
        // spawning at the nominal tap with the last known period while
        // the crossfade drains, so the fallback handover never gaps.
        // Idle with no live voices it costs nothing.
        if psola_render {
            self.psola_l[..frames].fill(0.0);
            self.psola_r[..frames].fill(0.0);
            let spawn = if engaged {
                SpawnMode::Marker
            } else if self.voice_xf > 0.0 {
                SpawnMode::Nominal
            } else {
                SpawnMode::None
            };
            // The musical transpose (quantized like the async cloud's)
            // sets the output marker density; per-grain detune spread
            // does not apply to the mono voice lattice.
            let mut semis = params.pitch_semitones;
            if params.quantize != PitchQuantize::Off {
                semis = quantize_transpose(semis, params.quantize, params.scale);
            }
            let alpha = f64::from(semis / 12.0).exp2();
            let tap_seconds = if time_varying {
                self.time_delay_buf[0]
            } else {
                self.eff_delay as f32
            };
            let delay_samples = f64::from(tap_seconds) * sr;
            let (psl, psr) = (&mut self.psola_l[..frames], &mut self.psola_r[..frames]);
            self.psola.render(
                &self.buf_l,
                &self.buf_r,
                self.write_pos,
                head_adv,
                delay_samples,
                alpha,
                spawn,
                psl,
                psr,
            );
        }

        // --- 2d. Fade time mode (ba todo #1076): the SwapFader's
        // per-sample gain windows the whole granulated wet — audible
        // cloud and feedback tap alike — through the tap swap: out over
        // ~10 ms, swap on the silent sample (where the engines were
        // hard-reset), back in over ~10 ms at the new origin. Applied
        // before the feedback stage so recirculations carry the faded
        // wet coherently.
        if fade_varying {
            for i in 0..frames {
                let g = self.time_gain_buf[i];
                self.wet_l[i] *= g;
                self.wet_r[i] *= g;
            }
            if fb_render {
                for i in 0..frames {
                    let g = self.time_gain_buf[i];
                    self.fbw_l[i] *= g;
                    self.fbw_r[i] *= g;
                }
            }
            if psola_render {
                for i in 0..frames {
                    let g = self.time_gain_buf[i];
                    self.psola_l[i] *= g;
                    self.psola_r[i] *= g;
                }
            }
        }

        // --- 2e. Voice/Mono blend (ba todo #1082): equal-power
        // crossfade between the async grain cloud and the PSOLA bus,
        // ramped per sample over [`VOICE_FADE_SECONDS`], so
        // voiced/unvoiced handovers (and enabling/disabling the mode)
        // are transparent. Applied before the feedback stage so
        // recirculations carry the blended wet.
        if psola_render {
            let step = 1.0 / (VOICE_FADE_SECONDS * self.sample_rate).max(1.0);
            let target_xf: f32 = if engaged { 1.0 } else { 0.0 };
            for i in 0..frames {
                if self.voice_xf < target_xf {
                    self.voice_xf = (self.voice_xf + step).min(1.0);
                } else if self.voice_xf > target_xf {
                    self.voice_xf = (self.voice_xf - step).max(0.0);
                }
                if self.voice_xf > 0.0 {
                    let phase = std::f32::consts::FRAC_PI_2 * self.voice_xf;
                    let (g_voice, g_async) = phase.sin_cos();
                    self.wet_l[i] = self.wet_l[i] * g_async + self.psola_l[i] * g_voice;
                    self.wet_r[i] = self.wet_r[i] * g_async + self.psola_r[i] * g_voice;
                }
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
                // recirculation crosses sides. With FB Pitch off the
                // tap carries the un-transposed re-granulation instead
                // of the transposed wet (ba todo #1078).
                let cross = params.fb_route == FbRoute::PingPong;
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let (tap_l, tap_r) = if unity_tap {
                        (self.fbw_l[i], self.fbw_r[i])
                    } else {
                        (self.wet_l[i], self.wet_r[i])
                    };
                    let (src_l, src_r) = if cross { (tap_r, tap_l) } else { (tap_l, tap_r) };
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
            FbRoute::OutputOnly if time_varying => {
                // Clean repeats with the recirc read tap following the
                // time mode too (ba todo #1076): the tap reads the ring
                // at the per-sample effective delay with a fractional
                // Hermite read. Repitch thereby glides — the tape swoop
                // also repitches the repeats — and Fade jumps on the
                // silent sample with the read masked by the swap gain,
                // so the tap jump cannot click (in the output or in
                // what recirculates).
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let idx = (fb_base + i) & self.mask;
                    let d = (f64::from(self.time_delay_buf[i]) * sr).max(1.0);
                    let pos = (fb_base + i) as f64 - d;
                    let tap_gain = if fade_varying { self.time_gain_buf[i] } else { 1.0 };
                    let fl = self.fb_chain_l.process(
                        read_hermite_wrapped(&self.fb_ring_l, pos) * tap_gain * g,
                        params.filter_is_highpass,
                    );
                    let fr = self.fb_chain_r.process(
                        read_hermite_wrapped(&self.fb_ring_r, pos) * tap_gain * g,
                        params.filter_is_highpass,
                    );
                    self.fb_ring_l[idx] = self.wet_l[i] + fl;
                    self.fb_ring_r[idx] = self.wet_r[i] + fr;
                    self.wet_l[i] += fl;
                    self.wet_r[i] += fr;
                }
                self.fb_len = 0;
            }
            FbRoute::OutputOnly => {
                // Clean repeats: recirculate the wet-path output through
                // a dedicated ring read at the delay time; the grain
                // source buffer never sees wet material. Bounded even
                // while frozen: the loop still passes through the tanh.
                let delay_samples = ((self.eff_delay as f32 * self.sample_rate) as usize)
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
