//! A vocal lane's expression curves — Dynamics, Tension, Breathiness and
//! Pitch bend — as they are saved with the lane config
//! (vocal-mcp-control-and-quality.md §10).
//!
//! Points are **section-relative**: `beat` counts quarter notes from the
//! start of the section the lane belongs to, so every placement of the
//! section sings the same curve. `value` is the normalized `0..=1` lane
//! value; what it means (an envelope, or cents of pitch bend) is the app's
//! business. Pure data — sampling and editing live in the app.

use serde::{Deserialize, Serialize};

/// One point of a curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VocalCurvePoint {
    /// Quarter notes from the section start.
    pub beat: f64,
    /// Normalized `0..=1` value.
    pub value: f32,
    /// Hold this value until the next point instead of ramping to it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stepped: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The four curves of one vocal lane, each sorted by `beat`. An empty
/// curve leaves the render on its own derivation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VocalCurves {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamics: Vec<VocalCurvePoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tension: Vec<VocalCurvePoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breathiness: Vec<VocalCurvePoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pitch_bend: Vec<VocalCurvePoint>,
}

impl VocalCurves {
    /// True when no curve has a point.
    pub fn is_empty(&self) -> bool {
        self.dynamics.is_empty()
            && self.tension.is_empty()
            && self.breathiness.is_empty()
            && self.pitch_bend.is_empty()
    }

    /// The curve at `index` in the order dynamics, tension, breathiness,
    /// pitch bend (`None` past 3).
    pub fn get(&self, index: usize) -> Option<&Vec<VocalCurvePoint>> {
        match index {
            0 => Some(&self.dynamics),
            1 => Some(&self.tension),
            2 => Some(&self.breathiness),
            3 => Some(&self.pitch_bend),
            _ => None,
        }
    }

    /// Mutable [`Self::get`].
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Vec<VocalCurvePoint>> {
        match index {
            0 => Some(&mut self.dynamics),
            1 => Some(&mut self.tension),
            2 => Some(&mut self.breathiness),
            3 => Some(&mut self.pitch_bend),
            _ => None,
        }
    }
}
