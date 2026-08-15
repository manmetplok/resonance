//! Parameter range types for linear and skewed mappings.
//!
//! [`FloatRange::normalize`] and [`FloatRange::denormalize`] are the two
//! halves of one mapping: plain parameter value <-> the 0..1 travel of
//! the control that edits it. Since ba todo #1281 every knob and slider
//! in every editor moves through them, so this file decides where each
//! value sits on each dial.

/// Range for float parameters.
#[derive(Clone)]
pub enum FloatRange {
    Linear {
        min: f32,
        max: f32,
    },
    Skewed {
        min: f32,
        max: f32,
        /// Skew factor. **Negative** gives the low end of the range more
        /// of the control's travel (a frequency, a time, a gain — the
        /// usual case); positive gives the high end more.
        factor: f32,
    },
}

impl FloatRange {
    /// Compute a skew factor for use with `Skewed`.
    ///
    /// Negative values give the values near the minimum more of the
    /// control's travel; positive values favour the maximum.
    pub fn skew_factor(factor: f32) -> f32 {
        factor
    }

    /// Compute a skew factor appropriate for gain parameters (dB scale).
    pub fn gain_skew_factor(min_db: f32, max_db: f32) -> f32 {
        // Log-based skew that makes the middle of the slider correspond
        // to a more perceptually useful range for gain.
        let range = max_db - min_db;
        if range.abs() < f32::EPSILON {
            return 0.0;
        }
        // Approximate: negative skew to bunch values toward lower gains
        -2.0 * min_db.abs() / range
    }

    /// Normalize a plain value to 0..1 — where it sits on the travel of
    /// the control that edits it.
    ///
    /// # The skew exponent's sign (ba todo #1281)
    ///
    /// The curve is `travel = linear^(2^factor)`, so a **negative**
    /// factor is a fractional exponent and stretches the low end of the
    /// range across more of the control. That is the direction every
    /// declaration in the fleet asks for: gain, attack/release times,
    /// filter cutoffs and the gate's key HPF all pass a negative
    /// `skew_factor`, and [`FloatRange::gain_skew_factor`] returns one.
    ///
    /// This exponent used to be `2^(-factor)`, which did the exact
    /// opposite — a `skew_factor(-3.0)` cutoff would have put 98 % of
    /// the dial above 2 kHz, and the gate's 1 ms attack (audit finding
    /// C5) would have become *less* dialable, not more. It went
    /// unnoticed because nothing outside the tests ever called this:
    /// the editors hardcoded their own linear/logarithmic arcs and threw
    /// the declared skew away, which is the finding (F4/W4/C5) #1281
    /// exists to fix. Correcting the sign here changes no shipped
    /// behaviour; it decides what the newly param-bound controls do.
    pub fn normalize(&self, value: f32) -> f32 {
        match self {
            FloatRange::Linear { min, max } => {
                if (max - min).abs() < f32::EPSILON {
                    return 0.0;
                }
                ((value - min) / (max - min)).clamp(0.0, 1.0)
            }
            FloatRange::Skewed { min, max, factor } => {
                if (max - min).abs() < f32::EPSILON {
                    return 0.0;
                }
                let linear = ((value - min) / (max - min)).clamp(0.0, 1.0);
                if factor.abs() < f32::EPSILON {
                    linear
                } else {
                    // Apply power curve: travel = linear^(2^factor).
                    linear.powf(2.0_f32.powf(*factor))
                }
            }
        }
    }

    /// Map a normalized 0..1 position back to a plain value — the exact
    /// inverse of [`FloatRange::normalize`].
    ///
    /// This is what turns knob/slider travel into a parameter value, so
    /// the declared skew is the *only* curve a control ever follows. Note
    /// that it is a power law, not a logarithm: a `min` of exactly 0.0
    /// (the gate's `key_hpf`) maps cleanly here, where a log mapping has
    /// to clamp the low end away from zero and silently changes the
    /// parameter's own bounds.
    ///
    /// Out-of-range input is clamped, so the result always lies inside
    /// `min..=max`.
    pub fn denormalize(&self, normalized: f32) -> f32 {
        let t = if normalized.is_nan() {
            0.0
        } else {
            normalized.clamp(0.0, 1.0)
        };
        match self {
            FloatRange::Linear { min, max } => min + t * (max - min),
            FloatRange::Skewed { min, max, factor } => {
                let linear = if factor.abs() < f32::EPSILON {
                    t
                } else {
                    // Inverse of normalize's linear^(2^factor).
                    t.powf(2.0_f32.powf(-*factor))
                };
                min + linear * (max - min)
            }
        }
    }

    pub fn min(&self) -> f32 {
        match self {
            FloatRange::Linear { min, .. } | FloatRange::Skewed { min, .. } => *min,
        }
    }

    pub fn max(&self) -> f32 {
        match self {
            FloatRange::Linear { max, .. } | FloatRange::Skewed { max, .. } => *max,
        }
    }
}

/// Range for integer parameters.
#[derive(Clone)]
pub enum IntRange {
    Linear { min: i32, max: i32 },
}

impl IntRange {
    pub fn normalize(&self, value: i32) -> f64 {
        match self {
            IntRange::Linear { min, max } => {
                if max == min {
                    return 0.0;
                }
                ((value - min) as f64 / (max - min) as f64).clamp(0.0, 1.0)
            }
        }
    }


    pub fn min(&self) -> i32 {
        match self {
            IntRange::Linear { min, .. } => *min,
        }
    }

    pub fn max(&self) -> i32 {
        match self {
            IntRange::Linear { max, .. } => *max,
        }
    }
}
