//! Full parameter set for the granular delay (doc #252 §9).
//!
//! Every parameter from the §9 table is declared here so the CLAP id
//! space is stable from the first release; parameters owned by later
//! todos in epic #196 are *inert* (declared, saved/restored, but not yet
//! read by the DSP) and are marked `TODO(epic-196 #...)` below.

use resonance_plugin::*;

pub const PARAM_COUNT: usize = 29;

pub struct GranularDelayParams {
    // --- Time -----------------------------------------------------------
    pub sync: BoolParam,
    pub division: IntParam,
    pub time_ms: FloatParam,
    /// 0 = Fade, 1 = Repitch, 2 = Per-Grain. TODO(epic-196 #1076): only
    /// the granular-native Per-Grain behaviour exists today (new grains
    /// latch the new time at spawn); Fade/Repitch are inert.
    pub time_mode: IntParam,

    // --- Feedback -------------------------------------------------------
    /// Loop gain, 0–110 %: wet × feedback → damping filter → tanh soft
    /// clip → DC blocker (doc #252 §1/§5; the soft clip keeps the
    /// over-unity range bounded).
    pub feedback: FloatParam,
    /// 0 = Wet->Buffer (recirculations are re-granulated), 1 =
    /// Output-only (clean repeats; the buffer keeps the dry input),
    /// 2 = Ping-pong (Wet->Buffer with the channels crossed at the
    /// feedback write tap, ba todo #1077).
    pub fb_route: IntParam,
    /// Shimmer (ba todo #1078, doc #252 §3): on = the transposed
    /// granulated wet is what recirculates, so each pass transposes
    /// cumulatively (+12 st climbs octaves); off = the feedback tap
    /// carries an un-transposed re-granulation, so recirculations keep
    /// a constant pitch. Only meaningful on the granulated-feedback
    /// routes (Wet→Buffer / Ping-pong); Output-only recirculates the
    /// once-transposed wet unchanged either way.
    pub fb_pitch: BoolParam,

    // --- Grains ---------------------------------------------------------
    pub grain_size_ms: FloatParam,
    pub density_hz: FloatParam,
    /// Tempo-synced density (grains per beat division). TODO(epic-196).
    pub density_sync: BoolParam,
    /// 0 = Sync, 1 = Async, 2 = Pitch-Sync. Pitch-Sync (PSOLA-style
    /// voice/mono mode) is TODO(epic-196 #1082) and falls back to Async.
    pub scheduler: IntParam,

    // --- Pitch ----------------------------------------------------------
    pub pitch: FloatParam,
    /// 0 = Off, 1 = Semitones, 2 = Scale (ba todo #1078): the per-grain
    /// effective transpose (base Pitch + random Spread) is quantized at
    /// spawn; Scale mode snaps to degrees of `root`/`scale` via
    /// resonance-music-theory (see `crate::quantize`).
    pub pitch_quantize: IntParam,
    pub spread_cents: FloatParam,
    /// Scale root for Pitch Quantize = Scale: 0–11 = C..B.
    pub root: IntParam,
    /// Scale mode for Pitch Quantize = Scale: indexes
    /// `resonance_music_theory::Mode::ALL` (0 = Chromatic, 1 = Major,
    /// 2 = Minor, ... 9 = Melodic Minor).
    pub scale: IntParam,

    // --- Texture / randomization ----------------------------------------
    pub texture: FloatParam,
    pub spray_ms: FloatParam,
    pub size_jitter: FloatParam,
    pub level_jitter: FloatParam,
    pub reverse_prob: FloatParam,

    // --- Buffer ---------------------------------------------------------
    /// Freeze/hold (latching; usable momentarily via host automation):
    /// stops the write head and holds the buffer while grains keep
    /// reading it; engage/resume crossfade the write gain over a few ms
    /// so there is no splice click (doc #252 §1).
    pub freeze: BoolParam,

    // --- Wet path -------------------------------------------------------
    /// Damping filter type in the feedback loop: 0 = LP, 1 = HP.
    pub filter_type: IntParam,
    /// Damping filter cutoff in the feedback loop (smoothed; the
    /// coefficient updates at block rate).
    pub filter_hz: FloatParam,
    /// Allpass smear of the wet path. TODO(epic-196): follow-up; not in
    /// #1074's feedback DoD.
    pub diffusion: FloatParam,
    pub pan_spread: FloatParam,
    /// M/S width on the wet sum, 0–150 % (smoothed; ba todo #1077).
    pub width: FloatParam,
    pub mix: FloatParam,
    /// 0 = Lo-fi, 1 = Normal, 2 = HQ. The HQ tier already engages the
    /// grain engine's rate-tracked anti-alias lowpass; the full tier
    /// treatment (interp order, lo-fi µ-law) is TODO(epic-196 #1083).
    pub quality: IntParam,
}

impl GranularDelayParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.sync,
            1 => &self.division,
            2 => &self.time_ms,
            3 => &self.time_mode,
            4 => &self.feedback,
            5 => &self.fb_route,
            6 => &self.fb_pitch,
            7 => &self.grain_size_ms,
            8 => &self.density_hz,
            9 => &self.density_sync,
            10 => &self.scheduler,
            11 => &self.pitch,
            12 => &self.pitch_quantize,
            13 => &self.spread_cents,
            14 => &self.texture,
            15 => &self.spray_ms,
            16 => &self.size_jitter,
            17 => &self.level_jitter,
            18 => &self.reverse_prob,
            19 => &self.freeze,
            20 => &self.filter_type,
            21 => &self.filter_hz,
            22 => &self.diffusion,
            23 => &self.pan_spread,
            24 => &self.width,
            25 => &self.mix,
            26 => &self.quality,
            // Appended after the initial 27 so the P1 CLAP index space
            // stays stable (ba todo #1078).
            27 => &self.root,
            28 => &self.scale,
            _ => &self.sync,
        }
    }
}

impl Default for GranularDelayParams {
    fn default() -> Self {
        Self {
            sync: BoolParam::new("sync", "Sync", true),

            division: IntParam::new(
                "division",
                "Division",
                4, // 1/4 (doc #252 §9 default)
                IntRange::Linear { min: 0, max: 11 },
            ),

            time_ms: FloatParam::new(
                "time_ms",
                "Time",
                500.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 4000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            time_mode: IntParam::new(
                "time_mode",
                "Time Mode",
                2, // Per-Grain
                IntRange::Linear { min: 0, max: 2 },
            ),

            feedback: FloatParam::new(
                "feedback",
                "Feedback",
                0.35,
                FloatRange::Linear { min: 0.0, max: 1.1 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            fb_route: IntParam::new(
                "fb_route",
                "FB Route",
                0, // Wet -> Buffer
                IntRange::Linear { min: 0, max: 2 },
            ),

            fb_pitch: BoolParam::new("fb_pitch", "FB Pitch", false),

            grain_size_ms: FloatParam::new(
                "grain_size_ms",
                "Grain Size",
                90.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            // Default ≈ overlap 2x at the 90 ms default grain size
            // (doc #252 §9: density default "overlap 2x").
            density_hz: FloatParam::new(
                "density_hz",
                "Density",
                22.0,
                FloatRange::Skewed {
                    min: 0.5,
                    max: 100.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" /s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            density_sync: BoolParam::new("density_sync", "Density Sync", false),

            scheduler: IntParam::new(
                "scheduler",
                "Scheduler",
                1, // Async (doc #252 §9 default)
                IntRange::Linear { min: 0, max: 2 },
            ),

            pitch: FloatParam::new(
                "pitch",
                "Pitch",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            pitch_quantize: IntParam::new(
                "pitch_quantize",
                "Pitch Quantize",
                0, // Off
                IntRange::Linear { min: 0, max: 2 },
            ),

            root: IntParam::new(
                "root",
                "Root",
                0, // C
                IntRange::Linear { min: 0, max: 11 },
            ),

            scale: IntParam::new(
                "scale",
                "Scale",
                1, // Major (Mode::ALL[1])
                IntRange::Linear { min: 0, max: 9 },
            ),

            spread_cents: FloatParam::new(
                "spread_cents",
                "Spread",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            texture: FloatParam::new(
                "texture",
                "Texture",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            spray_ms: FloatParam::new(
                "spray_ms",
                "Spray",
                20.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            size_jitter: FloatParam::new(
                "size_jitter",
                "Size Jitter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            level_jitter: FloatParam::new(
                "level_jitter",
                "Level Jitter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            reverse_prob: FloatParam::new(
                "reverse_prob",
                "Reverse",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            freeze: BoolParam::new("freeze", "Freeze", false),

            filter_type: IntParam::new(
                "filter_type",
                "Filter Type",
                0, // LP
                IntRange::Linear { min: 0, max: 1 },
            ),

            filter_hz: FloatParam::new(
                "filter_hz",
                "Filter",
                8000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            diffusion: FloatParam::new(
                "diffusion",
                "Diffusion",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            pan_spread: FloatParam::new(
                "pan_spread",
                "Pan Spread",
                0.4,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            width: FloatParam::new(
                "width",
                "Width",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.5 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new(
                "mix",
                "Mix",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            quality: IntParam::new(
                "quality",
                "Quality",
                1, // Normal
                IntRange::Linear { min: 0, max: 2 },
            ),
        }
    }
}

/// Block-rate smoothers for the global continuous parameters that are
/// *not* grain-latched. Grain-level parameters (size, pitch, spread,
/// texture, jitters, pan) need no smoothing: they are latched per grain
/// at spawn and the grain cloud itself interpolates (doc #252 §5).
/// Delay time is likewise latched per grain (Per-Grain time mode).
/// Smoothed: wet/dry mix, feedback amount, damping cutoff (ba todo
/// #1074), M/S width and the lock-stepped ↔ decorrelated crossfade
/// (ba todo #1077).
pub struct GranularSmoothers {
    pub mix: Smoother,
    /// Loop gain (per-sample application in the feedback stage).
    pub feedback: Smoother,
    /// Damping cutoff in Hz; consumed at block rate (`skip` + `current`)
    /// because the one-pole coefficient update costs an `exp`.
    pub filter_hz: Smoother,
    /// M/S width on the wet sum (per-sample application).
    pub width: Smoother,
    /// Equal-power crossfade position between the lock-stepped right
    /// engine (0) and the decorrelated one (1); the target is the
    /// binary gate `pan_spread > 0`, smoothed so toggling the spread
    /// across zero never clicks.
    pub decor: Smoother,
}

impl Default for GranularSmoothers {
    fn default() -> Self {
        Self::new()
    }
}

impl GranularSmoothers {
    pub fn new() -> Self {
        Self {
            mix: Smoother::new(SmoothingStyle::Linear(50.0)),
            feedback: Smoother::new(SmoothingStyle::Linear(50.0)),
            filter_hz: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            width: Smoother::new(SmoothingStyle::Linear(50.0)),
            decor: Smoother::new(SmoothingStyle::Linear(crate::dsp::DECOR_FADE_MS)),
        }
    }

    /// Crossfade gate for the decorrelated right engine: fully engaged
    /// whenever the pan spread is non-zero.
    fn decor_gate(params: &GranularDelayParams) -> f32 {
        if params.pan_spread.value() > 0.0 {
            1.0
        } else {
            0.0
        }
    }

    pub fn prepare(&mut self, sample_rate: f32, params: &GranularDelayParams) {
        self.mix.set_sample_rate(sample_rate);
        self.mix.reset(params.mix.value());
        self.feedback.set_sample_rate(sample_rate);
        self.feedback.reset(params.feedback.value());
        self.filter_hz.set_sample_rate(sample_rate);
        self.filter_hz.reset(params.filter_hz.value());
        self.width.set_sample_rate(sample_rate);
        self.width.reset(params.width.value());
        self.decor.set_sample_rate(sample_rate);
        self.decor.reset(Self::decor_gate(params));
    }

    pub fn retarget_from(&mut self, params: &GranularDelayParams) {
        self.mix.set_target(params.mix.value());
        self.feedback.set_target(params.feedback.value());
        self.filter_hz.set_target(params.filter_hz.value());
        self.width.set_target(params.width.value());
        self.decor.set_target(Self::decor_gate(params));
    }
}
