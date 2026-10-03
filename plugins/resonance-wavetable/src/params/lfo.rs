use resonance_plugin::*;

use crate::dsp::lfo::SyncDivision;

pub struct LfoParams {
    pub shape: IntParam,
    pub rate: FloatParam,
    pub depth: FloatParam,
    /// Per-voice phase reset on note-on. Ignored when [`Self::sync`] is on —
    /// see [`crate::dsp::lfo::LfoMode`].
    pub retrigger: BoolParam,
    /// Lock the phase to the host transport at [`Self::division`] instead of
    /// free-running at [`Self::rate`].
    pub sync: BoolParam,
    /// Musical division a synced LFO cycles over. Values are
    /// [`SyncDivision`] discriminants.
    pub division: IntParam,
}

impl LfoParams {
    pub(super) fn new(
        num: usize,
        default_rate: f32,
        default_depth: f32,
        default_retrigger: bool,
    ) -> Self {
        let sh_id: &'static str = super::intern(format!("lfo{}_shape", num));
        let sh_name: &'static str = super::intern(format!("LFO {} Shape", num));
        let rt_id: &'static str = super::intern(format!("lfo{}_rate", num));
        let rt_name: &'static str = super::intern(format!("LFO {} Rate", num));
        let dp_id: &'static str = super::intern(format!("lfo{}_depth", num));
        let dp_name: &'static str = super::intern(format!("LFO {} Depth", num));
        let rtr_id: &'static str = super::intern(format!("lfo{}_retrigger", num));
        let rtr_name: &'static str = super::intern(format!("LFO {} Retrigger", num));
        let sync_id: &'static str = super::intern(format!("lfo{}_sync", num));
        let sync_name: &'static str = super::intern(format!("LFO {} Sync", num));
        let div_id: &'static str = super::intern(format!("lfo{}_division", num));
        let div_name: &'static str = super::intern(format!("LFO {} Division", num));

        Self {
            shape: IntParam::new(sh_id, sh_name, 0, IntRange::Linear { min: 0, max: 4 }),
            rate: FloatParam::new(
                rt_id,
                rt_name,
                default_rate,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 50.0,
                    factor: -2.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                dp_id,
                dp_name,
                default_depth,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            retrigger: BoolParam::new(rtr_id, rtr_name, default_retrigger),
            sync: BoolParam::new(sync_id, sync_name, false),
            division: IntParam::new(
                div_id,
                div_name,
                SyncDivision::DEFAULT as i32,
                IntRange::Linear {
                    min: 0,
                    max: (SyncDivision::LABELS.len() - 1) as i32,
                },
            ),
        }
    }
}
