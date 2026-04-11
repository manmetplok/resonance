//! Plugin parameters for the stereo compressor.

use std::sync::Arc;

use resonance_plugin::*;

pub const PARAM_COUNT: usize = 11;

pub struct CompressorParams {
    /// Threshold in dBFS at which compression begins.
    pub threshold: FloatParam,
    /// Ratio 1..20; displays as `n:1`. Above 20 it's effectively limiting,
    /// but capping at 20 keeps the UI readable.
    pub ratio: FloatParam,
    /// Attack time in milliseconds.
    pub attack: FloatParam,
    /// Release time in milliseconds.
    pub release: FloatParam,
    /// Soft-knee width in dB. 0 = hard knee, 12 = wide transition.
    pub knee: FloatParam,
    /// Manual makeup gain in dB applied after compression.
    pub makeup: FloatParam,
    /// Dry/wet blend for parallel compression. 1.0 = fully compressed.
    pub mix: FloatParam,
    /// Detector mode: 0.0 = peak, 1.0 = RMS, continuous blend in between.
    pub detector_mix: FloatParam,
    /// Sidechain high-pass frequency in Hz, only applied when `sc_hpf_on`.
    pub sc_hpf_freq: FloatParam,
    /// Enable the sidechain high-pass filter on the detector input.
    pub sc_hpf_on: BoolParam,
    /// Automatically apply makeup gain based on threshold + ratio.
    pub auto_makeup: BoolParam,
}

impl CompressorParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.threshold,
            1 => &self.ratio,
            2 => &self.attack,
            3 => &self.release,
            4 => &self.knee,
            5 => &self.makeup,
            6 => &self.mix,
            7 => &self.detector_mix,
            8 => &self.sc_hpf_freq,
            9 => &self.sc_hpf_on,
            10 => &self.auto_makeup,
            _ => unreachable!("invalid compressor param index {index}"),
        }
    }
}

fn format_db(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v: f32| format!("{:.*} dB", decimals, v))
}

fn format_ms(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v: f32| {
        if v >= 100.0 {
            format!("{:.*} ms", decimals.saturating_sub(1), v)
        } else {
            format!("{:.*} ms", decimals, v)
        }
    })
}

fn format_ratio() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v >= 19.5 {
            "∞:1".to_string()
        } else {
            format!("{:.1}:1", v)
        }
    })
}

fn format_percent(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v: f32| format!("{:.*}%", decimals, v * 100.0))
}

fn format_hz() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v >= 1000.0 {
            format!("{:.2} kHz", v / 1000.0)
        } else {
            format!("{:.0} Hz", v)
        }
    })
}

fn format_detector() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v < 0.05 {
            "Peak".to_string()
        } else if v > 0.95 {
            "RMS".to_string()
        } else {
            format!("{:.0}% RMS", v * 100.0)
        }
    })
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold: FloatParam::new(
                "threshold",
                "Threshold",
                -18.0,
                FloatRange::Linear { min: -60.0, max: 0.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(format_db(1)),

            ratio: FloatParam::new(
                "ratio",
                "Ratio",
                4.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 20.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(format_ratio()),

            attack: FloatParam::new(
                "attack",
                "Attack",
                10.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 200.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(format_ms(2)),

            release: FloatParam::new(
                "release",
                "Release",
                120.0,
                FloatRange::Skewed {
                    min: 5.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(format_ms(1)),

            knee: FloatParam::new(
                "knee",
                "Knee",
                6.0,
                FloatRange::Linear { min: 0.0, max: 12.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(format_db(1)),

            makeup: FloatParam::new(
                "makeup",
                "Makeup",
                0.0,
                FloatRange::Linear { min: -12.0, max: 24.0 },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(format_db(1)),

            mix: FloatParam::new(
                "mix",
                "Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit("%")
            .with_value_to_string(format_percent(0)),

            detector_mix: FloatParam::new(
                "detector_mix",
                "Detector",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(format_detector()),

            sc_hpf_freq: FloatParam::new(
                "sc_hpf_freq",
                "SC HPF",
                80.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(format_hz()),

            sc_hpf_on: BoolParam::new("sc_hpf_on", "SC HPF On", false),
            auto_makeup: BoolParam::new("auto_makeup", "Auto Makeup", false),
        }
    }
}
