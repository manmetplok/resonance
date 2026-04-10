/// State-variable filter (Cytomic/Simper topology).
/// Provides simultaneous LP, HP, BP, Notch outputs.
/// Stable under rapid cutoff modulation.

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum FilterType {
    Lowpass = 0,
    Highpass = 1,
    Bandpass = 2,
    Notch = 3,
}

impl FilterType {
    pub fn from_int(v: i32) -> Self {
        match v {
            0 => Self::Lowpass,
            1 => Self::Highpass,
            2 => Self::Bandpass,
            3 => Self::Notch,
            _ => Self::Lowpass,
        }
    }
}

#[derive(Clone)]
pub struct StateVariableFilter {
    ic1eq: f32,
    ic2eq: f32,
}

impl StateVariableFilter {
    pub fn new() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    pub fn clear(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Process one sample through the filter.
    ///
    /// - `cutoff_hz`: filter cutoff frequency (20..20000)
    /// - `resonance`: 0..1 (0 = no resonance, 1 = self-oscillation)
    /// - `sample_rate`: audio sample rate
    /// - `filter_type`: which output to use
    /// - `drive`: 0..1 input drive (soft-clip)
    pub fn process(
        &mut self,
        input: f32,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
        filter_type: FilterType,
        drive: f32,
    ) -> f32 {
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.49);
        let g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        // Damping: k=2 means no resonance, k->0 means self-oscillation
        let k = 2.0 - 2.0 * resonance.clamp(0.0, 0.99);

        // Optional soft-clip drive on input
        let input = if drive > 0.001 {
            let driven = input * (1.0 + drive * 5.0);
            soft_clip(driven)
        } else {
            input
        };

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        match filter_type {
            FilterType::Lowpass => v2,
            FilterType::Highpass => input - k * v1 - v2,
            FilterType::Bandpass => v1,
            FilterType::Notch => input - k * v1,
        }
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    x.tanh()
}
