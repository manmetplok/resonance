//! Plugin parameters for the 8-band EQ.

use std::sync::Arc;

use resonance_plugin::*;

use crate::band::{BandKind, BandMs, BandSlope, KIND_LABELS, MS_LABELS};

pub const NUM_BANDS: usize = 8;
/// The original per-band block (enabled, freq, gain, q, kind, slope),
/// laid out band-major from index 0.
pub const PARAMS_PER_BAND: usize = 6;
/// Index of `output_gain`, right after the original band blocks.
pub const OUTPUT_GAIN_INDEX: usize = NUM_BANDS * PARAMS_PER_BAND;
/// Index of `auto_gain`.
pub const AUTO_GAIN_INDEX: usize = OUTPUT_GAIN_INDEX + 1;
/// Per-band parameters added after the original layout (warmth-width-
/// depth.md §6.4). They live in their own band-major block *after*
/// `auto_gain`, so every original parameter keeps its index.
pub const EXTRA_PER_BAND: usize = 1;
/// First index of the extra per-band block.
pub const EXTRA_BASE: usize = AUTO_GAIN_INDEX + 1;
pub const PARAM_COUNT: usize = EXTRA_BASE + NUM_BANDS * EXTRA_PER_BAND;

pub struct BandParams {
    pub enabled: BoolParam,
    pub freq: FloatParam,
    pub gain: FloatParam,
    pub q: FloatParam,
    /// 0=Bell, 1=LowShelf, 2=HighShelf, 3=LowCut, 4=HighCut, 5=Tilt,
    /// 6=LF Lift+Dip, 7=Air — mirror of `BandKind`.
    pub kind: IntParam,
    /// 0=12 dB/oct, 1=24 dB/oct, 2=48 dB/oct — only meaningful for LowCut/HighCut.
    pub slope: IntParam,
    /// 0=Stereo, 1=Mid, 2=Side — mirror of `BandMs`. Stereo by default.
    pub ms: IntParam,
}

pub struct EqParams {
    pub bands: [BandParams; NUM_BANDS],
    pub output_gain: FloatParam,
    /// Loudness-compensated output: trims the output by the static gain
    /// estimate of the band curve (see `crate::dsp::static_gain_db`), so
    /// an EQ move can be judged at matched level. Off by default.
    pub auto_gain: BoolParam,
}

impl BandParams {
    /// Read a strongly-typed snapshot of the current band settings.
    pub fn snapshot(&self) -> BandSnapshot {
        BandSnapshot {
            enabled: self.enabled.value(),
            freq: self.freq.value(),
            gain_db: self.gain.value(),
            q: self.q.value(),
            kind: BandKind::from_index(self.kind.value()),
            slope: BandSlope::from_index(self.slope.value()),
            ms: BandMs::from_index(self.ms.value()),
        }
    }

    /// The extra per-band parameter at offset `within` of the band's
    /// block in the extra region (see [`EXTRA_BASE`]).
    fn extra_at(&self, within: usize) -> &dyn Param {
        match within {
            0 => &self.ms,
            _ => &self.ms,
        }
    }
}

/// Plain-old-data snapshot of a band's current values. Used by the DSP to
/// detect changes and recompute coefficients, and by the editor to render
/// the response curve.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BandSnapshot {
    pub enabled: bool,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    pub kind: BandKind,
    pub slope: BandSlope,
    pub ms: BandMs,
}

impl EqParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        if index == OUTPUT_GAIN_INDEX {
            return &self.output_gain;
        }
        if index == AUTO_GAIN_INDEX {
            return &self.auto_gain;
        }
        if index >= EXTRA_BASE {
            let off = (index - EXTRA_BASE).min(NUM_BANDS * EXTRA_PER_BAND - 1);
            // One extra per band for now, so the modulo is trivially 0.
            #[allow(clippy::modulo_one)]
            let (band, within) = (off / EXTRA_PER_BAND, off % EXTRA_PER_BAND);
            return self.bands[band].extra_at(within);
        }
        let band = index / PARAMS_PER_BAND;
        let within = index % PARAMS_PER_BAND;
        let b = &self.bands[band];
        match within {
            0 => &b.enabled,
            1 => &b.freq,
            2 => &b.gain,
            3 => &b.q,
            4 => &b.kind,
            5 => &b.slope,
            _ => &b.enabled,
        }
    }
}

/// Frequency formatter: Hz below 1 kHz, kHz above.
fn format_hz() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v >= 1000.0 {
            format!("{:.2} kHz", v / 1000.0)
        } else {
            format!("{:.0} Hz", v)
        }
    })
}

/// Host-facing labels of the slope parameter, by index.
const SLOPE_LABELS: &[&str] = &["12 dB/oct", "24 dB/oct", "48 dB/oct"];

fn format_db(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v: f32| format!("{:.*} dB", decimals, v))
}

fn format_q() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| format!("{:.2}", v))
}

macro_rules! make_band {
    (
        $ix:tt,
        freq_default: $freq_default:expr,
        kind_default: $kind_default:expr,
        enabled_default: $enabled_default:expr
    ) => {
        BandParams {
            enabled: BoolParam::new(
                concat!("band", $ix, "_enabled"),
                concat!("Band ", $ix, " Enabled"),
                $enabled_default,
            ),
            freq: FloatParam::new(
                concat!("band", $ix, "_freq"),
                concat!("Band ", $ix, " Freq"),
                $freq_default,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(format_hz()),
            gain: FloatParam::new(
                concat!("band", $ix, "_gain"),
                concat!("Band ", $ix, " Gain"),
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(format_db(1)),
            q: FloatParam::new(
                concat!("band", $ix, "_q"),
                concat!("Band ", $ix, " Q"),
                0.707,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(format_q()),
            kind: IntParam::new(
                concat!("band", $ix, "_kind"),
                concat!("Band ", $ix, " Kind"),
                $kind_default,
                IntRange::Linear {
                    min: 0,
                    max: KIND_LABELS.len() as i32 - 1,
                },
            )
            .with_choices(KIND_LABELS),
            slope: IntParam::new(
                concat!("band", $ix, "_slope"),
                concat!("Band ", $ix, " Slope"),
                1,
                IntRange::Linear { min: 0, max: 2 },
            )
            .with_choices(SLOPE_LABELS),
            ms: IntParam::new(
                concat!("band", $ix, "_ms"),
                concat!("Band ", $ix, " M/S"),
                0,
                IntRange::Linear {
                    min: 0,
                    max: MS_LABELS.len() as i32 - 1,
                },
            )
            .with_choices(MS_LABELS),
        }
    };
}

impl Default for EqParams {
    fn default() -> Self {
        // Kind defaults: 0=Bell 1=LowShelf 2=HighShelf 3=LowCut 4=HighCut.
        // Spread the 8 bands across the spectrum with sensible starting
        // points — bookends are cuts, interior bands are bells/shelves.
        Self {
            bands: [
                make_band!("0", freq_default: 40.0,   kind_default: 3, enabled_default: false),
                make_band!("1", freq_default: 120.0,  kind_default: 1, enabled_default: false),
                make_band!("2", freq_default: 250.0,  kind_default: 0, enabled_default: false),
                make_band!("3", freq_default: 600.0,  kind_default: 0, enabled_default: false),
                make_band!("4", freq_default: 1500.0, kind_default: 0, enabled_default: false),
                make_band!("5", freq_default: 4000.0, kind_default: 0, enabled_default: false),
                make_band!("6", freq_default: 9000.0, kind_default: 2, enabled_default: false),
                make_band!("7", freq_default: 16000.0, kind_default: 4, enabled_default: false),
            ],
            output_gain: FloatParam::new(
                "output_gain",
                "Output",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(format_db(1)),
            auto_gain: BoolParam::new("auto_gain", "Auto Gain", false),
        }
    }
}
