use resonance_plugin::*;

pub const PARAM_COUNT: usize = 8;

pub struct GateParams {
    pub threshold: FloatParam,
    pub ratio: FloatParam,
    pub attack: FloatParam,
    pub hold: FloatParam,
    pub release: FloatParam,
    pub range: FloatParam,
    pub hysteresis: FloatParam,
    pub key_hpf: FloatParam,
}

impl GateParams {
    /// The parameter at a host/editor index, as the concrete
    /// [`FloatParam`] — every parameter the gate declares is one.
    ///
    /// The editor needs the concrete type, not `&dyn Param`, because
    /// `editor_widgets::float_knob` reads the range, the default *and*
    /// the declared skew off the parameter it is handed (ba todos
    /// #1281/#1286). Going through `&dyn Param` is what cost the gate
    /// its skew: the trait exposes `min_plain`/`max_plain` but not the
    /// `FloatRange`, so the old strip could only ask the widget for a
    /// linear arc.
    ///
    /// Out-of-range indices fall back to `threshold`, matching
    /// [`GateParams::param_at`], which the CLAP bridge relies on.
    pub fn float_at(&self, index: usize) -> &FloatParam {
        match index {
            0 => &self.threshold,
            1 => &self.ratio,
            2 => &self.attack,
            3 => &self.hold,
            4 => &self.release,
            5 => &self.range,
            6 => &self.hysteresis,
            7 => &self.key_hpf,
            _ => &self.threshold,
        }
    }

    /// The same parameter as [`GateParams::float_at`], type-erased for
    /// the CLAP bridge. Delegating keeps one index table rather than
    /// two that can disagree.
    pub fn param_at(&self, index: usize) -> &dyn Param {
        self.float_at(index)
    }
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            threshold: FloatParam::new(
                "threshold",
                "Threshold",
                -40.0,
                FloatRange::Linear {
                    min: -80.0,
                    max: 0.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            // 1.0 is a pass-through; the default is a hard-ish gate.
            // Lower ratios turn it into a gentle downward expander.
            ratio: FloatParam::new(
                "ratio",
                "Ratio",
                8.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 20.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            attack: FloatParam::new(
                "attack",
                "Attack",
                1.0,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 100.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            hold: FloatParam::new(
                "hold",
                "Hold",
                20.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            release: FloatParam::new(
                "release",
                "Release",
                100.0,
                FloatRange::Skewed {
                    min: 5.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            // How far the closed state attenuates. 80 dB is effectively
            // mute; smaller values duck instead of gating, which is what
            // you want on a drum bus you don't want holes in.
            range: FloatParam::new(
                "range",
                "Range",
                60.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 80.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            hysteresis: FloatParam::new(
                "hysteresis",
                "Hysteresis",
                6.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 24.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            // Detector-only high-pass; 0 disables it.
            key_hpf: FloatParam::new(
                "key_hpf",
                "Key HPF",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
        }
    }
}
