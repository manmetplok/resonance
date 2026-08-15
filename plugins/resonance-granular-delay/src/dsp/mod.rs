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
//!
//! # Module map (ba todo #1264)
//!
//! [`GranularDsp`] owns one struct per sub-system and this file holds
//! only their wiring — the render stages live next to the state they
//! touch, in render order:
//!
//! | module         | sub-system                                            |
//! |----------------|-------------------------------------------------------|
//! | [`modes`]      | the `TimeMode` / `FbRoute` / `QualityTier` choices      |
//! | [`source`]     | source ring, write head, freeze crossfade, peak mip     |
//! | [`time`]       | Fade swap machine + Repitch slew, per-sample delay/gain |
//! | [`voice`]      | pitch tracker, PSOLA voices, voice crossfade            |
//! | [`grains`]     | the three engine pairs and their wet buses              |
//! | [`quant`]      | the plugin-side quantized-transpose draw                |
//! | [`feedback`]   | conditioning chains, wet bus, recirc rings and clock    |
//! | [`mix`]        | M/S width and the equal-power dry/wet mix               |
//! | [`viz_publish`]| editor presentation only — never read by the DSP        |

mod feedback;
mod grains;
mod mix;
mod modes;
mod quant;
mod source;
mod time;
mod viz_publish;
mod voice;

use resonance_dsp::SchedulerMode;
use resonance_music_theory::Scale;

use crate::params::GranularSmoothers;
use crate::quantize::PitchQuantize;
use crate::viz::GranularViz;

use feedback::FeedbackStage;
use grains::GrainBank;
use quant::QuantDraw;
use source::SourceRing;
use time::TimeMachine;
use voice::VoiceStage;

pub use grains::{ALIGN_WINDOW_SECONDS, DECOR_FADE_MS};
pub use modes::{DampingFilter, FbRoute, QualityTier, Scheduler, TimeMode, LOFI_MAX_GRAINS};
pub use source::{FREEZE_RAMP_SECONDS, MAX_DELAY_SECONDS};
pub use time::{FADE_LEG_SECONDS, REPITCH_TAU_SECONDS};
pub use voice::VOICE_FADE_SECONDS;

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
    /// WSOLA-style correlation-aligned grain onsets (ba todo #1320):
    /// every grain engine snaps each onset to the lag within
    /// [`ALIGN_WINDOW_SECONDS`] that best continues the previously
    /// spawned grain. Off is the unaligned engine, bit for bit.
    pub align: bool,
    pub pitch_semitones: f32,
    pub detune_spread_cents: f32,
    pub texture: f32,
    /// Position jitter ("spray"), seconds.
    pub spray_seconds: f32,
    pub size_jitter: f32,
    pub level_jitter: f32,
    pub reverse_probability: f32,
    pub pan_spread: f32,
    /// Quality tier (ba todo #1083): resolves to the per-grain
    /// interpolation kernel, µ-law lo-fi quantization, pool cap and
    /// anti-alias engagement of every grain engine.
    pub quality: QualityTier,
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

/// The granular delay's realtime core: one struct per sub-system, wired
/// together by [`GranularDsp::process_block`].
pub struct GranularDsp {
    sample_rate: f32,
    /// Source ring, write head, freeze crossfade and peak mip.
    source: SourceRing,
    /// Fade/Repitch time-mode machine (ba todo #1076).
    time: TimeMachine,
    /// Pitch-synchronous voice path and its crossfade (ba todo #1082).
    voice: VoiceStage,
    /// Grain engines and their wet buses.
    grains: GrainBank,
    /// Plugin-side quantized-transpose draw (ba todo #1078).
    quant: QuantDraw,
    /// Feedback topology (ba todo #1074/#1075).
    feedback: FeedbackStage,
}

impl GranularDsp {
    pub fn new(sample_rate: f32, max_block: usize) -> Self {
        let ring_len = ((MAX_DELAY_SECONDS * sample_rate) as usize + 1).next_power_of_two();
        let max_block = max_block.max(1);
        Self {
            sample_rate,
            source: SourceRing::new(ring_len),
            time: TimeMachine::new(sample_rate, max_block),
            voice: VoiceStage::new(sample_rate, max_block),
            grains: GrainBank::new(sample_rate, max_block),
            quant: QuantDraw::new(),
            feedback: FeedbackStage::new(ring_len, max_block),
        }
    }

    /// Ring-buffer length in seconds (>= [`MAX_DELAY_SECONDS`]).
    pub fn buffer_seconds(&self) -> f32 {
        self.source.buffer_seconds(self.sample_rate)
    }

    /// Total grains spawned since construction (metering aid for tests
    /// and the future editor, ba todo #1079). The engines are in
    /// lockstep, so either count is authoritative.
    pub fn grains_spawned(&self) -> u64 {
        self.grains.engine_l.grains_spawned()
    }

    /// Currently sounding grains (metering aid).
    pub fn active_grains(&self) -> usize {
        self.grains.engine_l.active_grains()
    }

    /// Grain onsets the WSOLA aligner has moved off their nominal
    /// position since construction (ba todo #1320; audible-path
    /// metering aid, left *internal* — it drives tests and any future
    /// diagnostic readout, and the editor deliberately draws no lag
    /// meter: the alignment decision is audible, a per-spawn lag count
    /// is not something a user acts on).
    pub fn aligned_spawns(&self) -> u64 {
        self.grains.engine_l.aligned_spawns()
    }

    /// Largest onset-alignment lag magnitude applied so far, in samples
    /// (metering aid; bounded by [`ALIGN_WINDOW_SECONDS`]).
    pub fn max_align_lag_samples(&self) -> f64 {
        self.grains.engine_l.max_abs_align_lag_samples()
    }

    /// Playback rates of the currently sounding audible grains (left
    /// lock-stepped engine; test/metering aid). With pitch quantization
    /// on, every rate sits on the quantized semitone/scale lattice
    /// (ba todo #1078).
    pub fn active_rates(&self) -> impl Iterator<Item = f64> + '_ {
        self.grains.engine_l.active_rates()
    }

    /// Playback rates of the decorrelated right engine's grains
    /// (test/metering aid; ba todos #1077/#1078).
    pub fn active_rates_decor(&self) -> impl Iterator<Item = f64> + '_ {
        self.grains.engine_r_decor.active_rates()
    }

    /// Playback rates of the feedback-tap engine's grains — all ±1
    /// while the un-transposed tap is active (test/metering aid,
    /// ba todo #1078).
    pub fn active_rates_fb(&self) -> impl Iterator<Item = f64> + '_ {
        self.grains.fb_engine_l.active_rates()
    }

    /// Silence the buffer and all grains; allocation-free apart from the
    /// buffer zeroing (called from `reset`, off the steady-state path).
    pub fn clear(&mut self) {
        self.source.clear();
        self.grains.clear();
        self.quant.clear();
        self.feedback.clear();
        self.voice.clear();
        self.time.clear(self.sample_rate);
    }

    /// Current effective delay-tap position, seconds (test/metering
    /// aid, ba todo #1076): equals the resolved target in Per-Grain
    /// mode, glides monotonically toward it in Repitch mode and steps
    /// on the (silent) swap sample in Fade mode.
    pub fn effective_delay_seconds(&self) -> f32 {
        self.time.eff_delay as f32
    }

    /// Whether the pitch-synchronous scheduler was engaged (tracker
    /// voiced + usable pitch mark near the tap) on the last block
    /// (test/metering aid, ba todo #1082).
    pub fn pitch_sync_engaged(&self) -> bool {
        self.voice.engaged
    }

    /// Total PSOLA onsets spawned (test/metering aid).
    pub fn psola_onsets(&self) -> u64 {
        self.voice.psola.onsets()
    }

    /// Currently sounding PSOLA voices (test/metering aid).
    pub fn psola_active_voices(&self) -> usize {
        self.voice.psola.active_voices()
    }

    /// The most recent PSOLA onset times in absolute output samples,
    /// oldest first (up to 64). Test aid — allocates, keep off the
    /// audio path.
    pub fn psola_recent_onsets(&self) -> Vec<f64> {
        self.voice.psola.recent_onsets()
    }

    /// Last known tracked fundamental period in full-rate samples
    /// (0 before the first voiced lock; held across unvoiced spans and
    /// freeze). Test/metering aid.
    pub fn tracked_period_samples(&self) -> f32 {
        self.voice.psola.period_samples() as f32
    }

    /// Read-only view of the left grain source ring (test/metering aid:
    /// the Output-only route must keep this identical to the dry input,
    /// and freeze must hold it bit-stable). Sample `n` of the stream
    /// lives at index `n & (ring_len - 1)` while it remains in range.
    pub fn ring_l(&self) -> &[f32] {
        &self.source.buf_l
    }

    /// Read-only view of the right grain source ring (see [`Self::ring_l`]).
    pub fn ring_r(&self) -> &[f32] {
        &self.source.buf_r
    }

    /// Absolute write-head position in samples (test/metering aid).
    pub fn write_head(&self) -> u64 {
        self.source.write_pos
    }

    /// Viz publisher (ba todo #1135, design doc #264 req-1): pack the
    /// currently sounding grains and the coarse buffer peaks into the
    /// shared [`GranularViz`] atomics, at block rate next to
    /// `store_block`. Allocation-free and lock-free.
    ///
    /// Pure presentation: the snapshot building, the synthesized
    /// recirculation ghosts and the peak publication all live in
    /// [`viz_publish`], and nothing they compute re-enters the render
    /// path.
    pub fn publish_viz(&self, viz: &GranularViz, params: &BlockParams, feedback_gain: f32) {
        viz_publish::publish(self, viz, params, feedback_gain);
    }

    /// Render one block in place. `left`/`right` arrive carrying the dry
    /// input and leave carrying the equal-power dry/wet mix.
    ///
    /// Orchestration only (ba todos #1132/#1264): each stage lives in
    /// its sub-system's module, called here in render order — buffer
    /// write, time-mode resolution, pitch-sync state, grain rendering,
    /// the decor/PSOLA blends, feedback and the output mix. Everything
    /// remains allocation-free and lock-free on this path.
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        smoothers: &mut GranularSmoothers,
        params: &BlockParams,
    ) {
        let frames = frames
            .min(left.len())
            .min(right.len())
            .min(self.grains.capacity());
        if frames == 0 {
            return;
        }

        // Damping cutoff: block-rate coefficient update from the
        // smoothed value, sample-rate application (doc #252 §5).
        smoothers.filter_hz.skip(frames as u32);
        let cutoff = smoothers.filter_hz.current().clamp(20.0, 20_000.0);
        self.feedback.set_damping(cutoff, self.sample_rate);

        let wet_to_buffer = params.fb_route.feeds_buffer();

        let advanced = self.source.write_input(
            left,
            right,
            frames,
            self.sample_rate,
            wet_to_buffer,
            params,
            &self.feedback,
            &mut self.voice.track_in,
        );
        let plan = self.time.resolve(frames, params);
        let (engaged, psola_render) =
            self.voice
                .resolve(&self.source, &self.time, self.sample_rate, params, advanced, &plan);
        let gates = self.grains.resolve_gates(params, wet_to_buffer, smoothers);
        let grain_params = grains::base_grain_params(
            params,
            engaged,
            advanced,
            frames,
            self.source.buffer_seconds(self.sample_rate),
        );
        let head_adv = grain_params.head_advance as f64;

        self.grains.render(
            &self.source,
            &self.time,
            &mut self.quant,
            self.sample_rate,
            frames,
            params,
            &grain_params,
            &plan,
            &gates,
        );
        self.grains.blend_decor(frames, gates.decor_gate, smoothers);
        if psola_render {
            self.voice.render(
                &self.source,
                &self.time,
                self.sample_rate,
                frames,
                engaged,
                &plan,
                params,
                head_adv,
            );
        }
        time::apply_fade_gain(
            &self.time,
            frames,
            &plan,
            &mut self.grains,
            &mut self.voice,
            gates.fb_render,
            psola_render,
        );
        if psola_render {
            self.voice
                .blend(&mut self.grains, self.sample_rate, frames, engaged);
        }
        self.feedback.run(
            &mut self.grains,
            &self.time,
            self.sample_rate,
            frames,
            params,
            gates.unity_tap,
            &plan,
            smoothers,
        );
        mix::mix_output(&self.grains, left, right, frames, smoothers);

        // The write head advances only by the samples actually written
        // (it stops while frozen); the recirc clock always advances.
        self.source.write_pos += advanced as u64;
        self.feedback.advance(frames);
    }
}
