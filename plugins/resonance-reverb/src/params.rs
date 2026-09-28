/// Plugin parameters for the algorithmic reverb.
///
/// The params here only store atomic current values — no smoothers.
/// Per-parameter smoothing lives in [`ReverbSmoothers`] below, which
/// the plugin owns directly (not via `Arc`) so the audio thread can
/// mutate smoother state through `&mut self`.
use resonance_plugin::*;

pub const PARAM_COUNT: usize = 22;

pub struct ReverbParams {
    pub predelay: FloatParam,
    pub er_level: FloatParam,
    pub er_time: FloatParam,
    pub size: FloatParam,
    pub decay: FloatParam,
    pub damping: FloatParam,
    pub diffusion: FloatParam,
    pub mod_rate: FloatParam,
    pub mod_depth: FloatParam,
    pub width: FloatParam,
    pub mix: FloatParam,
    pub freeze: BoolParam,
    // -- Return EQ, ducking, depth (warmth-width-depth.md §6.4) ----------
    //
    // Appended after the original twelve so their host order and indices
    // are unchanged. Every one defaults to a no-op: both filters off,
    // duck amount 0, ER/tail balance centred — a project saved before
    // they existed renders bit-identically (`tests/legacy_state.rs`).
    /// High-pass on the reverb input, before the tank (the Abbey Road
    /// return EQ). Off by default.
    pub wet_hpf_on: BoolParam,
    pub wet_hpf_freq: FloatParam,
    /// Low-pass on the reverb input, before the tank. Off by default.
    pub wet_lpf_on: BoolParam,
    pub wet_lpf_freq: FloatParam,
    /// Slope of both wet filters: 0 = 12 dB/oct, 1 = 18 dB/oct.
    pub wet_filter_slope: IntParam,
    /// How far the wet return is pulled down while the key (the external
    /// sidechain, or the dry input when none is connected) is over
    /// `duck_threshold`. 0 disables ducking; 1 is
    /// [`crate::dsp::DUCK_MAX_GR_DB`].
    pub duck_amount: FloatParam,
    pub duck_threshold: FloatParam,
    pub duck_attack: FloatParam,
    pub duck_release: FloatParam,
    /// Depth crossfade between early reflections and the tail. 0 is the
    /// plugin's original balance; toward -1 the tail fades out (close,
    /// "in the room"), toward +1 the early reflections fade out (far,
    /// just the wash).
    pub er_tail_balance: FloatParam,
}

/// Labels of [`ReverbParams::wet_filter_slope`].
pub const WET_SLOPE_LABELS: &[&str] = &["12 dB/oct", "18 dB/oct"];

impl ReverbParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.predelay,
            1 => &self.er_level,
            2 => &self.er_time,
            3 => &self.size,
            4 => &self.decay,
            5 => &self.damping,
            6 => &self.diffusion,
            7 => &self.mod_rate,
            8 => &self.mod_depth,
            9 => &self.width,
            10 => &self.mix,
            11 => &self.freeze,
            12 => &self.wet_hpf_on,
            13 => &self.wet_hpf_freq,
            14 => &self.wet_lpf_on,
            15 => &self.wet_lpf_freq,
            16 => &self.wet_filter_slope,
            17 => &self.duck_amount,
            18 => &self.duck_threshold,
            19 => &self.duck_attack,
            20 => &self.duck_release,
            21 => &self.er_tail_balance,
            _ => &self.predelay,
        }
    }
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            predelay: FloatParam::new(
                "predelay",
                "Pre-delay",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 250.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            er_level: FloatParam::new(
                "er_level",
                "ER Level",
                0.4,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            er_time: FloatParam::new(
                "er_time",
                "ER Time",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            size: FloatParam::new(
                "size",
                "Size",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            decay: FloatParam::new(
                "decay",
                "Decay",
                2.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            damping: FloatParam::new(
                "damping",
                "Damping",
                8000.0,
                FloatRange::Skewed {
                    min: 200.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            diffusion: FloatParam::new(
                "diffusion",
                "Diffusion",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mod_rate: FloatParam::new(
                "mod_rate",
                "Mod Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            mod_depth: FloatParam::new(
                "mod_depth",
                "Mod Depth",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            width: FloatParam::new(
                "width",
                "Width",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            freeze: BoolParam::new("freeze", "Freeze", false),

            wet_hpf_on: BoolParam::new("wet_hpf_on", "Wet HPF", false),
            wet_hpf_freq: FloatParam::new(
                "wet_hpf_freq",
                "Wet HPF Freq",
                600.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            wet_lpf_on: BoolParam::new("wet_lpf_on", "Wet LPF", false),
            wet_lpf_freq: FloatParam::new(
                "wet_lpf_freq",
                "Wet LPF Freq",
                10000.0,
                FloatRange::Skewed {
                    min: 1000.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            wet_filter_slope: IntParam::new(
                "wet_filter_slope",
                "Wet Filter Slope",
                0,
                IntRange::Linear {
                    min: 0,
                    max: WET_SLOPE_LABELS.len() as i32 - 1,
                },
            )
            .with_choices(WET_SLOPE_LABELS),

            duck_amount: FloatParam::new(
                "duck_amount",
                "Duck",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            duck_threshold: FloatParam::new(
                "duck_threshold",
                "Duck Threshold",
                -30.0,
                FloatRange::Linear {
                    min: -60.0,
                    max: 0.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            duck_attack: FloatParam::new(
                "duck_attack",
                "Duck Attack",
                15.0,
                FloatRange::Skewed {
                    min: 0.5,
                    max: 200.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            duck_release: FloatParam::new(
                "duck_release",
                "Duck Release",
                200.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            er_tail_balance: FloatParam::new(
                "er_tail_balance",
                "ER / Tail",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(format_balance()),
        }
    }
}

/// Readout of [`ReverbParams::er_tail_balance`]: which side it leans to.
fn format_balance() -> std::sync::Arc<dyn Fn(f32) -> String + Send + Sync> {
    std::sync::Arc::new(|v: f32| {
        if v.abs() < 0.005 {
            "Even".to_string()
        } else if v < 0.0 {
            format!("ER {:.0}%", -v * 100.0)
        } else {
            format!("Tail {:.0}%", v * 100.0)
        }
    })
}

/// Audio-thread-only smoothers, one per FloatParam. Lives outside the
/// shared `Arc<ReverbParams>` so the audio thread can mutate smoother
/// state through `&mut self` without fighting the editor's shared
/// reference.
pub struct ReverbSmoothers {
    pub predelay: Smoother,
    pub er_level: Smoother,
    pub er_time: Smoother,
    pub size: Smoother,
    pub decay: Smoother,
    pub damping: Smoother,
    pub diffusion: Smoother,
    pub mod_rate: Smoother,
    pub mod_depth: Smoother,
    pub width: Smoother,
    pub mix: Smoother,
    pub wet_hpf_freq: Smoother,
    pub wet_lpf_freq: Smoother,
    pub er_tail_balance: Smoother,
}

impl Default for ReverbSmoothers {
    fn default() -> Self {
        Self::new()
    }
}

impl ReverbSmoothers {
    pub fn new() -> Self {
        Self {
            predelay: Smoother::new(SmoothingStyle::Linear(50.0)),
            er_level: Smoother::new(SmoothingStyle::Linear(50.0)),
            er_time: Smoother::new(SmoothingStyle::Linear(50.0)),
            size: Smoother::new(SmoothingStyle::Linear(100.0)),
            decay: Smoother::new(SmoothingStyle::Linear(100.0)),
            damping: Smoother::new(SmoothingStyle::Linear(50.0)),
            diffusion: Smoother::new(SmoothingStyle::Linear(50.0)),
            mod_rate: Smoother::new(SmoothingStyle::Linear(50.0)),
            mod_depth: Smoother::new(SmoothingStyle::Linear(50.0)),
            width: Smoother::new(SmoothingStyle::Linear(50.0)),
            mix: Smoother::new(SmoothingStyle::Linear(50.0)),
            wet_hpf_freq: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            wet_lpf_freq: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            er_tail_balance: Smoother::new(SmoothingStyle::Linear(50.0)),
        }
    }

    /// Call once on `initialize` — updates every smoother's sample rate
    /// and resets them to the current param values so the first block
    /// doesn't ramp from zero.
    pub fn prepare(&mut self, sample_rate: f32, params: &ReverbParams) {
        self.predelay.set_sample_rate(sample_rate);
        self.er_level.set_sample_rate(sample_rate);
        self.er_time.set_sample_rate(sample_rate);
        self.size.set_sample_rate(sample_rate);
        self.decay.set_sample_rate(sample_rate);
        self.damping.set_sample_rate(sample_rate);
        self.diffusion.set_sample_rate(sample_rate);
        self.mod_rate.set_sample_rate(sample_rate);
        self.mod_depth.set_sample_rate(sample_rate);
        self.width.set_sample_rate(sample_rate);
        self.mix.set_sample_rate(sample_rate);
        self.wet_hpf_freq.set_sample_rate(sample_rate);
        self.wet_lpf_freq.set_sample_rate(sample_rate);
        self.er_tail_balance.set_sample_rate(sample_rate);

        self.predelay.reset(params.predelay.value());
        self.er_level.reset(params.er_level.value());
        self.er_time.reset(params.er_time.value());
        self.size.reset(params.size.value());
        self.decay.reset(params.decay.value());
        self.damping.reset(params.damping.value());
        self.diffusion.reset(params.diffusion.value());
        self.mod_rate.reset(params.mod_rate.value());
        self.mod_depth.reset(params.mod_depth.value());
        self.width.reset(params.width.value());
        self.mix.reset(params.mix.value());
        self.wet_hpf_freq.reset(params.wet_hpf_freq.value());
        self.wet_lpf_freq.reset(params.wet_lpf_freq.value());
        self.er_tail_balance.reset(params.er_tail_balance.value());
    }

    /// Push the current atomic param values as smoother targets at
    /// the start of each block.
    pub fn retarget_from(&mut self, params: &ReverbParams) {
        self.predelay.set_target(params.predelay.value());
        self.er_level.set_target(params.er_level.value());
        self.er_time.set_target(params.er_time.value());
        self.size.set_target(params.size.value());
        self.decay.set_target(params.decay.value());
        self.damping.set_target(params.damping.value());
        self.diffusion.set_target(params.diffusion.value());
        self.mod_rate.set_target(params.mod_rate.value());
        self.mod_depth.set_target(params.mod_depth.value());
        self.width.set_target(params.width.value());
        self.mix.set_target(params.mix.value());
        self.wet_hpf_freq.set_target(params.wet_hpf_freq.value());
        self.wet_lpf_freq.set_target(params.wet_lpf_freq.value());
        self.er_tail_balance.set_target(params.er_tail_balance.value());
    }
}
