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
    // Delay-line state (per channel instance).
    ic1eq: f32,
    ic2eq: f32,

    // Cached coefficients — computed once by `set_coeffs` and reused across
    // many per-sample `process` calls. The `tan()` and three multiplies are
    // the expensive part; avoiding per-sample recomputation is a large win
    // when cutoff modulation is slow (voice envelopes, LFOs) relative to
    // the sample rate.
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,

    // Input drive: 0 disables the soft-clip entirely.
    drive: f32,
    drive_gain: f32,
}

impl StateVariableFilter {
    pub fn new() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
            k: 2.0,
            a1: 1.0,
            a2: 0.0,
            a3: 0.0,
            drive: 0.0,
            drive_gain: 1.0,
        }
    }

    pub fn clear(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Precompute coefficients. Call this at control rate (once per block, or
    /// every N samples when modulation is fast) — *not* per sample.
    ///
    /// - `cutoff_hz`: filter cutoff frequency (20..sr/2)
    /// - `resonance`: 0..1 (0 = no resonance, 1 = self-oscillation)
    /// - `sample_rate`: audio sample rate
    /// - `drive`: 0..1 input drive (soft-clip)
    pub fn set_coeffs(&mut self, cutoff_hz: f32, resonance: f32, sample_rate: f32, drive: f32) {
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.49);
        let g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        // Damping: k=2 means no resonance, k->0 means self-oscillation.
        self.k = 2.0 - 2.0 * resonance.clamp(0.0, 0.99);

        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;

        self.drive = drive;
        self.drive_gain = 1.0 + drive * 5.0;
    }

    /// Process one sample using the most recently set coefficients.
    #[inline]
    pub fn process(&mut self, input: f32, filter_type: FilterType) -> f32 {
        // Optional soft-clip drive on input. The comparison is against a
        // fixed threshold matching the old per-sample API so behaviour is
        // unchanged.
        let input = if self.drive > 0.001 {
            soft_clip(input * self.drive_gain)
        } else {
            input
        };

        let v3 = input - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.a2 * self.ic1eq + self.a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        match filter_type {
            FilterType::Lowpass => v2,
            FilterType::Highpass => input - self.k * v1 - v2,
            FilterType::Bandpass => v1,
            FilterType::Notch => input - self.k * v1,
        }
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

impl Default for StateVariableFilter {
    fn default() -> Self {
        Self::new()
    }
}
