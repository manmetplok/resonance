use resonance_plugin::*;

pub const PARAM_COUNT: usize = 14;

pub struct DelayParams {
    pub sync: BoolParam,
    pub division: IntParam,
    pub time_ms: FloatParam,
    pub feedback: FloatParam,
    pub mix: FloatParam,
    pub character: IntParam,
    pub routing: IntParam,
    pub stereo_offset: FloatParam,
    pub hi_cut: FloatParam,
    pub lo_cut: FloatParam,
    pub drive: FloatParam,
    pub mod_rate: FloatParam,
    pub mod_depth: FloatParam,
    pub freeze: BoolParam,
}

impl DelayParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.sync,
            1 => &self.division,
            2 => &self.time_ms,
            3 => &self.feedback,
            4 => &self.mix,
            5 => &self.character,
            6 => &self.routing,
            7 => &self.stereo_offset,
            8 => &self.hi_cut,
            9 => &self.lo_cut,
            10 => &self.drive,
            11 => &self.mod_rate,
            12 => &self.mod_depth,
            13 => &self.freeze,
            _ => &self.sync,
        }
    }
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            sync: BoolParam::new("sync", "Sync", true),

            division: IntParam::new(
                "division",
                "Division",
                4,
                IntRange::Linear { min: 0, max: 11 },
            ),

            time_ms: FloatParam::new(
                "time_ms",
                "Time",
                375.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            feedback: FloatParam::new(
                "feedback",
                "Feedback",
                0.45,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new(
                "mix",
                "Mix",
                0.35,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            character: IntParam::new(
                "character",
                "Character",
                0,
                IntRange::Linear { min: 0, max: 1 },
            ),

            routing: IntParam::new("routing", "Routing", 0, IntRange::Linear { min: 0, max: 2 }),

            stereo_offset: FloatParam::new(
                "stereo_offset",
                "Stereo Offset",
                0.0,
                FloatRange::Linear {
                    min: -0.5,
                    max: 0.5,
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            hi_cut: FloatParam::new(
                "hi_cut",
                "Hi Cut",
                8000.0,
                FloatRange::Skewed {
                    min: 400.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            lo_cut: FloatParam::new(
                "lo_cut",
                "Lo Cut",
                120.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 1000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            drive: FloatParam::new(
                "drive",
                "Drive",
                0.15,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mod_rate: FloatParam::new(
                "mod_rate",
                "Mod Rate",
                0.4,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 6.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            mod_depth: FloatParam::new(
                "mod_depth",
                "Mod Depth",
                0.1,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            freeze: BoolParam::new("freeze", "Freeze", false),
        }
    }
}

pub struct DelaySmoothers {
    /// Resolved delay time in *samples*, retargeted once per block from
    /// the sync/division/time/tempo state (see `lib.rs`). Smoothing in
    /// samples-space means a sync toggle or division change glides the
    /// read tap along the delay line instead of relocating it
    /// discontinuously (a click).
    pub delay_samples: Smoother,
    pub feedback: Smoother,
    pub mix: Smoother,
    pub stereo_offset: Smoother,
    pub hi_cut: Smoother,
    pub lo_cut: Smoother,
    pub drive: Smoother,
    pub mod_rate: Smoother,
    pub mod_depth: Smoother,
}

impl Default for DelaySmoothers {
    fn default() -> Self {
        Self::new()
    }
}

impl DelaySmoothers {
    pub fn new() -> Self {
        Self {
            delay_samples: Smoother::new(SmoothingStyle::Linear(100.0)),
            feedback: Smoother::new(SmoothingStyle::Linear(50.0)),
            mix: Smoother::new(SmoothingStyle::Linear(50.0)),
            stereo_offset: Smoother::new(SmoothingStyle::Linear(50.0)),
            hi_cut: Smoother::new(SmoothingStyle::Linear(50.0)),
            lo_cut: Smoother::new(SmoothingStyle::Linear(50.0)),
            drive: Smoother::new(SmoothingStyle::Linear(50.0)),
            mod_rate: Smoother::new(SmoothingStyle::Linear(50.0)),
            mod_depth: Smoother::new(SmoothingStyle::Linear(50.0)),
        }
    }

    pub fn prepare(&mut self, sample_rate: f32, params: &DelayParams) {
        self.delay_samples.set_sample_rate(sample_rate);
        self.feedback.set_sample_rate(sample_rate);
        self.mix.set_sample_rate(sample_rate);
        self.stereo_offset.set_sample_rate(sample_rate);
        self.hi_cut.set_sample_rate(sample_rate);
        self.lo_cut.set_sample_rate(sample_rate);
        self.drive.set_sample_rate(sample_rate);
        self.mod_rate.set_sample_rate(sample_rate);
        self.mod_depth.set_sample_rate(sample_rate);

        // No tempo at prepare time; the unsynced resolution is the best
        // starting point and the first block retargets with tempo anyway.
        self.delay_samples.reset(crate::sync::delay_samples(
            params.sync.value(),
            params.division.value() as usize,
            params.time_ms.value(),
            None,
            sample_rate,
            sample_rate * 4.0 + 256.0,
        ));
        self.feedback.reset(params.feedback.value());
        self.mix.reset(params.mix.value());
        self.stereo_offset.reset(params.stereo_offset.value());
        self.hi_cut.reset(params.hi_cut.value());
        self.lo_cut.reset(params.lo_cut.value());
        self.drive.reset(params.drive.value());
        self.mod_rate.reset(params.mod_rate.value());
        self.mod_depth.reset(params.mod_depth.value());
    }

    /// Retarget all smoothers except `delay_samples`, which needs tempo
    /// context and is retargeted explicitly in `process`.
    pub fn retarget_from(&mut self, params: &DelayParams) {
        self.feedback.set_target(params.feedback.value());
        self.mix.set_target(params.mix.value());
        self.stereo_offset.set_target(params.stereo_offset.value());
        self.hi_cut.set_target(params.hi_cut.value());
        self.lo_cut.set_target(params.lo_cut.value());
        self.drive.set_target(params.drive.value());
        self.mod_rate.set_target(params.mod_rate.value());
        self.mod_depth.set_target(params.mod_depth.value());
    }
}
