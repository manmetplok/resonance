//! Parameter range types for linear and skewed mappings.

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
        /// Skew factor: negative values bunch values toward the low end,
        /// positive toward the high end.
        factor: f32,
    },
}

impl FloatRange {
    /// Compute a skew factor for use with `Skewed`.
    /// Negative values bunch toward the minimum, positive toward the maximum.
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

    /// Normalize a plain value to 0..1.
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
                    // Apply power curve: normalized = linear^(2^(-factor))
                    linear.powf(2.0_f32.powf(-*factor))
                }
            }
        }
    }

    /// Unnormalize a 0..1 value to the plain range.
    pub fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match self {
            FloatRange::Linear { min, max } => min + normalized * (max - min),
            FloatRange::Skewed { min, max, factor } => {
                let linear = if factor.abs() < f32::EPSILON {
                    normalized
                } else {
                    // Inverse of normalize: linear = normalized^(2^factor)
                    normalized.powf(2.0_f32.powf(*factor))
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

    pub fn unnormalize(&self, normalized: f64) -> i32 {
        let normalized = normalized.clamp(0.0, 1.0);
        match self {
            IntRange::Linear { min, max } => {
                ((*min as f64 + normalized * (*max - *min) as f64).round() as i32).clamp(*min, *max)
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
