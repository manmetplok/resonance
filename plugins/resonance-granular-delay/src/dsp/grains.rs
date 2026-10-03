//! Stage 2 — the grain bank: the three engine pairs that granulate the
//! source ring (the lock-stepped audible pair, the decorrelated right
//! engine of ba todo #1077 and the un-transposed feedback tap of ba todo
//! #1078), their wet buses, and the per-slice render loop that drives
//! them.

use resonance_dsp::{GrainEngine, GrainParams, InterpQuality, MAX_GRAINS};

use crate::params::GranularSmoothers;
use crate::quantize::{quantize_transpose, PitchQuantize};

use super::modes::LOFI_MAX_GRAINS;
use super::quant::{QuantDraw, QUANT_SLICE};
use super::source::SourceRing;
use super::time::{TimeMachine, TimePlan, REPITCH_RATE_MAX, REPITCH_RATE_MIN};
use super::{BlockParams, QualityTier};

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

/// Half-width of the WSOLA onset-alignment search window, seconds
/// (ba todo #1320). The engine's own hard cap is 10 ms; 5 ms is its
/// default and covers one period of everything above ~200 Hz, which is
/// where splice roughness is audible. It is fixed rather than exposed:
/// the audible decision is "aligned or not", and a second control would
/// cost a knob in the GRAINS group for a difference only very low
/// material can hear. Widen it to a parameter if that ever proves
/// wrong — `GrainParams::align_window_seconds` is already per block.
pub const ALIGN_WINDOW_SECONDS: f32 = 0.005;

/// Shared seed for the lock-stepped feedback-tap engine pair (ba todo
/// #1078): with FB Pitch off, these render the *un-transposed*
/// re-granulation that recirculates, so repeats keep a constant pitch
/// while the audible wet stays transposed.
const FB_TAP_SEED: u64 = 0xFBFB_7A93;

/// Per-block engine dispositions, fixed before the render loop (see
/// [`GrainBank::resolve_gates`]).
pub(super) struct EngineGates {
    /// The un-transposed feedback tap is engaged: FB Pitch off on a
    /// granulated-feedback route while a transpose is engaged
    /// (ba todo #1078).
    pub(super) unity_tap: bool,
    /// The feedback-tap engine pair renders this block (engaged, or
    /// stale grains still draining).
    pub(super) fb_render: bool,
    /// Pan Spread > 0: the decorrelated right engine spawns
    /// (ba todo #1077).
    pub(super) decor_gate: bool,
    /// The decorrelated engine renders this block (engaged, fade tail
    /// audible, or grains still draining).
    pub(super) decor_render: bool,
}

pub(super) struct GrainBank {
    /// Lock-stepped grain engines: same seed, same params, same call
    /// sequence, so their grain clouds are identical and each grain
    /// reads L and R at the same position (see [`ENGINE_SEED`]).
    pub(super) engine_l: GrainEngine,
    engine_r: GrainEngine,
    /// Decorrelated right-channel engine (see [`DECOR_SEED`]): rendered
    /// whenever Pan Spread > 0 (or while its tail drains) and blended
    /// into the right wet bus with an equal-power crossfade.
    pub(super) engine_r_decor: GrainEngine,
    /// Wet accumulation buses (pre-allocated to the max block size).
    pub(super) wet_l: Vec<f32>,
    pub(super) wet_r: Vec<f32>,
    /// Decorrelated right-channel wet bus, blended into `wet_r`.
    wet_r_decor: Vec<f32>,
    /// Lock-stepped feedback-tap engines (ba todo #1078, see
    /// [`FB_TAP_SEED`]): render the un-transposed re-granulation that
    /// recirculates while FB Pitch is off and a transpose is engaged.
    pub(super) fb_engine_l: GrainEngine,
    fb_engine_r: GrainEngine,
    /// Feedback-tap buses for the un-transposed re-granulation.
    pub(super) fbw_l: Vec<f32>,
    pub(super) fbw_r: Vec<f32>,
    /// Sink for the pan-opposite engine outputs (each engine renders a
    /// stereo pair; only its own channel's side is kept).
    discard: Vec<f32>,
}

impl GrainBank {
    pub(super) fn new(sample_rate: f32, max_block: usize) -> Self {
        Self {
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
            discard: vec![0.0; max_block],
        }
    }

    /// Longest block this bank's buses can render.
    pub(super) fn capacity(&self) -> usize {
        self.wet_l.len()
    }

    pub(super) fn clear(&mut self) {
        self.engine_l.reset();
        self.engine_r.reset();
        self.engine_r_decor.reset();
        self.wet_r_decor.fill(0.0);
        self.fb_engine_l.reset();
        self.fb_engine_r.reset();
        self.fbw_l.fill(0.0);
        self.fbw_r.fill(0.0);
    }

    /// Per-block engine dispositions, fixed before the render loop.
    ///
    /// Decorrelated engine (ba todo #1077): engaged while Pan Spread >
    /// 0; draining (no new spawns, live grains finish) while the fade
    /// or its tail is still audible; otherwise skipped entirely.
    ///
    /// Feedback tap (ba todo #1078): rendered while the un-transposed
    /// tap is engaged or while stale grains drain (density 0) — a
    /// toggle never resumes stale grains and the settled state costs
    /// nothing.
    pub(super) fn resolve_gates(
        &self,
        params: &BlockParams,
        wet_to_buffer: bool,
        smoothers: &GranularSmoothers,
    ) -> EngineGates {
        let decor_gate = params.pan_spread > 0.0;
        let decor_render = decor_gate
            || smoothers.decor.current() > 0.0
            || self.engine_r_decor.active_grains() > 0;
        let transpose_engaged = params.pitch_semitones != 0.0 || params.detune_spread_cents > 0.0;
        let unity_tap = wet_to_buffer && !params.fb_pitch && transpose_engaged;
        let fb_render = unity_tap
            || self.fb_engine_l.active_grains() > 0
            || self.fb_engine_r.active_grains() > 0;
        EngineGates {
            unity_tap,
            fb_render,
            decor_gate,
            decor_render,
        }
    }

    /// Granulate behind the write head into the wet buses: the
    /// lock-stepped audible pair, the decorrelated right engine
    /// (ba todo #1077) and the un-transposed feedback tap (ba todo
    /// #1078), sliced for pitch quantization and the Repitch slew.
    ///
    /// Per-grain transpose quantization (ba todo #1078): with
    /// quantization on, the engines' own detune draw is bypassed
    /// (spread passed as 0) and the block renders in short slices,
    /// each with its own plugin-side drawn and quantized effective
    /// transpose (base Pitch + alternating-sign random Spread), so
    /// grains latch (near-)independent quantized values at spawn.
    /// The Repitch slew (ba todo #1076) uses the same short slices
    /// so the gliding origin and rate offset stay smooth; a Fade
    /// swap splits the block at the (silent) swap sample. Otherwise
    /// the whole block is one slice; engine output is
    /// slice-invariant (onset scheduling carries across process
    /// calls), so behaviour is then unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &mut self,
        source: &SourceRing,
        time: &TimeMachine,
        quant: &mut QuantDraw,
        sample_rate: f32,
        frames: usize,
        params: &BlockParams,
        grain_params: &GrainParams,
        plan: &TimePlan,
        gates: &EngineGates,
    ) {
        let sr = f64::from(sample_rate);
        let target64 = f64::from(params.delay_seconds);
        let time_varying = plan.time_varying();

        self.wet_l[..frames].fill(0.0);
        self.wet_r[..frames].fill(0.0);
        self.wet_r_decor[..frames].fill(0.0);
        self.discard[..frames].fill(0.0);

        let write_pos = source.write_pos as f64;
        let head_adv = grain_params.head_advance as f64;

        let quantize_on = params.quantize != PitchQuantize::Off;
        let slice_len = if quantize_on || plan.repitch_slewing {
            QUANT_SLICE
        } else {
            frames
        };

        if gates.fb_render {
            self.fbw_l[..frames].fill(0.0);
            self.fbw_r[..frames].fill(0.0);
        }

        let mut off = 0usize;
        while off < frames {
            if off == plan.swap_at {
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
            if plan.swap_at > off && plan.swap_at < off + n {
                n = plan.swap_at - off;
            }
            let slice_delay = if time_varying {
                time.delay_buf[off]
            } else {
                time.eff_delay as f32
            };
            // Repitch (ba todo #1076): grains spawned during the slew
            // glide with the moving read origin, so they take the
            // matching playback-rate multiplier `1 − d(delay)/dt`
            // (delay growing ⇒ rate < 1 ⇒ pitch down, and vice versa).
            let repitch_semis = if plan.repitch_slewing {
                let step_samples = (target64 - f64::from(slice_delay)) * time.repitch_coeff * sr;
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
                    semitones += quant.next_spread_semitones(params.detune_spread_cents);
                }
                gp.pitch_semitones = quantize_transpose(semitones, params.quantize, params.scale);
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
                self.engine_l.process(&source.buf_l, wp, &gp, wet_l, discard);
            }
            {
                let (discard, wet_r) =
                    (&mut self.discard[off..off + n], &mut self.wet_r[off..off + n]);
                self.engine_r.process(&source.buf_r, wp, &gp, discard, wet_r);
            }
            // --- 2b. Decorrelated right channel (ba todo #1077): with
            // Pan Spread > 0 an independently seeded engine renders its
            // own cloud over the right buffer, and the right wet bus
            // crossfades (equal-power, smoothed) from the lock-stepped
            // cloud to it. Once drained and the fade has settled it
            // costs nothing.
            if gates.decor_render {
                let drain_params;
                let dgp = if gates.decor_gate {
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
                    .process(&source.buf_r, wp, dgp, discard, wet_dec);
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
            if gates.fb_render {
                let mut fgp = gp.clone();
                // Zero the musical transpose (and any quantized draw)
                // but keep the physical Repitch glide — the tap rides
                // the same moving origin as the audible cloud.
                fgp.pitch_semitones = repitch_semis;
                fgp.detune_spread_cents = 0.0;
                if !gates.unity_tap {
                    fgp.density_hz = 0.0; // drain, output unused
                }
                {
                    let (fbw_l, discard) =
                        (&mut self.fbw_l[off..off + n], &mut self.discard[off..off + n]);
                    self.fb_engine_l
                        .process(&source.buf_l, wp, &fgp, fbw_l, discard);
                }
                {
                    let (discard, fbw_r) =
                        (&mut self.discard[off..off + n], &mut self.fbw_r[off..off + n]);
                    self.fb_engine_r
                        .process(&source.buf_r, wp, &fgp, discard, fbw_r);
                }
            }
            off += n;
        }
    }

    /// Stage 2b (blend) — fold the decorrelated right engine's bus into
    /// the right wet bus with the smoothed equal-power crossfade
    /// (ba todo #1077).
    pub(super) fn blend_decor(
        &mut self,
        frames: usize,
        decor_gate: bool,
        smoothers: &mut GranularSmoothers,
    ) {
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
    }
}

/// The block-invariant grain parameters shared by every engine.
///
/// Quality-tier resolution (ba todo #1083): all four ingredients
/// are engine-side and grain-latched (or click-free by
/// construction), so a tier switch only affects grains spawned
/// from here on. Every engine — audible pair, decorrelated
/// right, feedback tap — inherits the tier, so the whole cloud
/// shares one character; the PSOLA voice pool deliberately does
/// not (unity-rate marker-snapped reads neither alias nor
/// resample, so tiers have nothing to improve there).
pub(super) fn base_grain_params(
    params: &BlockParams,
    engaged: bool,
    advanced: usize,
    frames: usize,
    buffer_seconds: f32,
) -> GrainParams {
    let (interp, lofi_quantize, max_polyphony, anti_alias) = match params.quality {
        QualityTier::LoFi => (InterpQuality::Linear, true, LOFI_MAX_GRAINS, false),
        QualityTier::Normal => (InterpQuality::Hermite4, false, MAX_GRAINS, false),
        QualityTier::Hq => (InterpQuality::Lagrange6, false, MAX_GRAINS, true),
    };

    GrainParams {
        // Voice/Mono engaged (ba todo #1082): the async cloud — and
        // its decorrelated and feedback-tap companions, which
        // inherit this density — drains until silent (no new
        // spawns, live grains finish) while the PSOLA bus takes
        // over, so a scheduler handover never resumes stale grains.
        density_hz: if engaged { 0.0 } else { params.density_hz },
        // Bound grain length by the buffer so slow/reversed grains
        // can never be lapped by the write head (doc #252 §5); the
        // engine enforces the exact per-grain collision guard.
        grain_seconds: params.grain_seconds.min(buffer_seconds * 0.5),
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
        // WSOLA onset alignment (ba todo #1320). Every engine inherits
        // it, so the whole cloud shares one splice character. Each
        // engine correlates over the buffer it reads, so on strongly
        // decorrelated stereo input the lock-stepped pair may snap L
        // and R to lags up to ±[`ALIGN_WINDOW_SECONDS`] apart — a mild
        // extra widening; on mono or correlated material the two agree
        // and the pair stays in exact lockstep.
        align: params.align,
        align_window_seconds: ALIGN_WINDOW_SECONDS,
        anti_alias,
        interp,
        lofi_quantize,
        max_polyphony,
        // Later epic-196 todos grow `GrainParams` (e.g. #1080's
        // alignment fields); default the rest so this literal stays
        // source-compatible as the engine evolves.
        ..GrainParams::default()
    }
}
