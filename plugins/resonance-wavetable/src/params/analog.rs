use resonance_plugin::*;

/// Analog instability (see `crate::dsp::analog`). Both default to 0, which
/// renders bit-identically to the synth before they existed, so no existing
/// preset or project changes sound.
pub struct AnalogParams {
    /// How far a note-on scatters each unison sub-voice's oscillator start
    /// phases: 0 resets every oscillator to phase 0 (every note starts
    /// identically aligned), 1 starts each one at a uniformly random phase —
    /// the free-running feel of an analog poly.
    ///
    /// A continuous amount rather than a Reset / Random / Free-running
    /// switch: a free-running phase in a stealing polysynth is just a phase
    /// nobody can predict, which a full random draw reproduces, while the
    /// in-between amounts keep the attack transient mostly consistent and
    /// still decorrelate a unison stack. It is also automatable.
    pub phase_random: FloatParam,
    /// One "analog" knob: a slow per-sub-voice random pitch walk (up to
    /// ±`DRIFT_MAX_CENTS`), plus a small static per-note offset of filter
    /// cutoff and oscillator level.
    pub drift: FloatParam,
}

impl AnalogParams {
    pub(super) fn new() -> Self {
        Self {
            phase_random: FloatParam::new(
                "osc_phase_random",
                "Osc Phase Random",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            drift: FloatParam::new(
                "analog",
                "Analog",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
