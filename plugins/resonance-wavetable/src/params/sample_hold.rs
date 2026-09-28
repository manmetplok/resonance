use resonance_plugin::*;

use crate::dsp::lfo::SyncDivision;

/// Parameters for the global `ModSource::SampleHold` generator (see
/// `dsp::lfo::SampleHoldGen`). Same rate/sync/division shape as
/// [`super::LfoParams`], minus the fields that only make sense per-voice
/// (`shape`, `depth`, `retrigger`) -- this source has one clock, shared by
/// every voice, and its own bipolar amount lives on whichever matrix slot
/// routes it, same as every other source.
pub struct SampleHoldParams {
    /// Free-running rate in Hz. Ignored while [`Self::sync`] is on.
    pub rate: FloatParam,
    /// 0 = classic stepped random; higher values slew toward each new
    /// target, turning it into a slowly drifting source.
    pub slew: FloatParam,
    /// Lock the phase to the host transport at [`Self::division`] instead of
    /// free-running at [`Self::rate`].
    pub sync: BoolParam,
    /// Musical division a synced clock cycles over. Values are
    /// [`SyncDivision`] discriminants -- declared via `with_choices` so the
    /// editor's `Div` knob needs no formatter of its own (unlike the LFOs'
    /// own division knob, still on the `int_knob_fmt` seam ba todo #1356
    /// will retire).
    pub division: IntParam,
}

impl SampleHoldParams {
    pub(super) fn new() -> Self {
        Self {
            rate: FloatParam::new(
                "mod_sh_rate",
                "S&H Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 50.0,
                    factor: -2.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            slew: FloatParam::new(
                "mod_sh_slew",
                "S&H Slew",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            sync: BoolParam::new("mod_sh_sync", "S&H Sync", false),
            division: IntParam::new(
                "mod_sh_division",
                "S&H Division",
                SyncDivision::DEFAULT as i32,
                IntRange::Linear {
                    min: 0,
                    max: (SyncDivision::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&SyncDivision::LABELS),
        }
    }
}
