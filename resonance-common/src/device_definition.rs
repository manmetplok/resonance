//! Device-definition data model for external hardware synths (architecture doc
//! #201, epic #40).
//!
//! One definition per synth *model* names the synth's parameters and maps each
//! to a MIDI binding (a CC or NRPN/RPN address) with an integer value range and
//! a response curve, plus a named patch/bank list. The definition is shared by
//! the realtime engine (`resonance-audio`), the app (`resonance-app`) and
//! project I/O so they all agree on what a parameter is, which MIDI message it
//! emits, and how a normalized automation-lane value maps onto the binding's
//! integer domain.
//!
//! Like the automation model (doc #162, [`crate::automation`]) the mapping math
//! lives here as the single source of truth ([`lane_value_to_binding_value`] /
//! [`binding_value_to_lane`]) so UI read-outs and the engine never disagree. A
//! lane value is always **normalized** `0.0..=1.0`; the binding value is the raw
//! integer actually sent on the wire (`0..=127` for CC / 7-bit NRPN, `0..=16383`
//! for 14-bit NRPN/RPN).
//!
//! Note: this module's [`MidiBinding`] (a CC/NRPN address for a device
//! parameter) is a distinct concept from [`crate::midi_map::MidiBinding`] (a
//! hardware-control → mixer-target mapping for MIDI Learn); both keep their
//! domain name and are reached through their module path.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Failure serializing or parsing a [`DeviceDefinition`]'s on-disk JSON.
#[derive(Debug, Error)]
pub enum DeviceJsonError {
    #[error("serialize device definition: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("parse device definition: {0}")]
    Parse(#[source] serde_json::Error),
}

/// On-disk schema version stamped into every [`DeviceDefinition`]. Bump when the
/// serialized shape changes incompatibly so loaders can migrate or reject.
pub const SCHEMA_VERSION: u32 = 1;

/// Shaping exponent for the non-linear curves. [`ParamCurve::Exponential`]
/// raises the normalized value to this power (slow start, fast finish, more
/// resolution near the minimum); [`ParamCurve::Logarithmic`] uses its reciprocal
/// (fast start, slow finish). The two are exact inverses, so a value mapped out
/// through one and back round-trips.
const CURVE_EXP: f32 = 2.0;

/// Which MIDI message a device parameter is bound to.
///
/// `Cc` is a 7-bit Control Change. `Nrpn`/`Rpn` carry a 14-bit parameter address
/// (`msb`/`lsb`, each `0..=127`); when `fourteen_bit` the *value* uses data-entry
/// MSB+LSB for `0..=16383` resolution, otherwise only the data-entry MSB is sent
/// for `0..=127`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MidiBinding {
    /// 7-bit Control Change on the given CC number (`0..=127`).
    Cc { cc: u8 },
    /// Non-Registered Parameter Number at address `msb`/`lsb`.
    Nrpn { msb: u8, lsb: u8, fourteen_bit: bool },
    /// Registered Parameter Number at address `msb`/`lsb` (same shape as NRPN).
    Rpn { msb: u8, lsb: u8, fourteen_bit: bool },
}

impl MidiBinding {
    /// Largest raw value this binding can carry: `127` for CC and for 7-bit
    /// NRPN/RPN (data-entry MSB only), `16383` for 14-bit NRPN/RPN.
    pub fn max_value(&self) -> u16 {
        match self {
            MidiBinding::Cc { .. } => 127,
            MidiBinding::Nrpn { fourteen_bit, .. } | MidiBinding::Rpn { fourteen_bit, .. } => {
                if *fourteen_bit {
                    16_383
                } else {
                    127
                }
            }
        }
    }
}

/// How a normalized `0.0..=1.0` lane value maps onto the binding's integer
/// range. Defaults to [`ParamCurve::Linear`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ParamCurve {
    /// Straight proportional map across `min..=max`.
    #[default]
    Linear,
    /// Slow start / fast finish — more resolution near the minimum.
    Exponential,
    /// Fast start / slow finish — more resolution near the maximum.
    Logarithmic,
}

/// One named, automatable parameter of a device.
///
/// `min`/`max`/`default` are in the binding's **integer** domain (not
/// normalized), so they must fit the binding's range (see
/// [`MidiBinding::max_value`]). `group` lets the picker cluster params (Filter,
/// Envelope, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceParam {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub binding: MidiBinding,
    pub min: u16,
    pub max: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<u16>,
    #[serde(default)]
    pub curve: ParamCurve,
}

/// One factory/user patch: a Bank Select (`bank_msb`/`bank_lsb`) plus
/// Program Change `program`, with a display name and optional category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchEntry {
    pub bank_msb: u8,
    pub bank_lsb: u8,
    pub program: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// A device definition: the named parameters and patches of one synth model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDefinition {
    /// Stable identifier, last-wins key in the registry (e.g. `"moog-muse"`).
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub schema_version: u32,
    #[serde(default)]
    pub params: Vec<DeviceParam>,
    #[serde(default)]
    pub patches: Vec<PatchEntry>,
}

/// A reason a [`DeviceDefinition`] failed [`DeviceDefinition::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceDefinitionError {
    /// The definition's `id` is empty/whitespace.
    EmptyDeviceId,
    /// A parameter's `id` is empty/whitespace.
    EmptyParamId,
    /// Two parameters share the same `id`.
    DuplicateParamId(String),
    /// `min > max` for the named parameter.
    MinGreaterThanMax { param_id: String, min: u16, max: u16 },
    /// `max` exceeds what the binding can carry (its [`MidiBinding::max_value`]).
    BindingValueOutOfRange {
        param_id: String,
        value: u16,
        limit: u16,
    },
    /// `default` lies outside `min..=max`.
    DefaultOutOfRange {
        param_id: String,
        default: u16,
        min: u16,
        max: u16,
    },
    /// A CC number outside `0..=127`.
    InvalidCc { param_id: String, cc: u8 },
    /// An NRPN/RPN address byte outside `0..=127`.
    InvalidParameterAddress { param_id: String, msb: u8, lsb: u8 },
}

impl fmt::Display for DeviceDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceDefinitionError::EmptyDeviceId => write!(f, "device id is empty"),
            DeviceDefinitionError::EmptyParamId => write!(f, "a parameter id is empty"),
            DeviceDefinitionError::DuplicateParamId(id) => {
                write!(f, "duplicate parameter id `{id}`")
            }
            DeviceDefinitionError::MinGreaterThanMax { param_id, min, max } => {
                write!(f, "parameter `{param_id}`: min {min} > max {max}")
            }
            DeviceDefinitionError::BindingValueOutOfRange {
                param_id,
                value,
                limit,
            } => write!(
                f,
                "parameter `{param_id}`: value {value} exceeds binding range 0..={limit}"
            ),
            DeviceDefinitionError::DefaultOutOfRange {
                param_id,
                default,
                min,
                max,
            } => write!(
                f,
                "parameter `{param_id}`: default {default} outside {min}..={max}"
            ),
            DeviceDefinitionError::InvalidCc { param_id, cc } => {
                write!(f, "parameter `{param_id}`: CC {cc} outside 0..=127")
            }
            DeviceDefinitionError::InvalidParameterAddress { param_id, msb, lsb } => write!(
                f,
                "parameter `{param_id}`: NRPN/RPN address {msb}/{lsb} outside 0..=127"
            ),
        }
    }
}

impl std::error::Error for DeviceDefinitionError {}

impl DeviceDefinition {
    /// Check the definition is internally consistent, returning the first
    /// problem found. Rejects empty ids, duplicate parameter ids, out-of-range
    /// bindings (CC/address bytes, `min`/`max` beyond the binding's resolution,
    /// `min > max`) and defaults outside `min..=max`.
    pub fn validate(&self) -> Result<(), DeviceDefinitionError> {
        if self.id.trim().is_empty() {
            return Err(DeviceDefinitionError::EmptyDeviceId);
        }
        let mut seen: HashSet<&str> = HashSet::with_capacity(self.params.len());
        for p in &self.params {
            if p.id.trim().is_empty() {
                return Err(DeviceDefinitionError::EmptyParamId);
            }
            if !seen.insert(p.id.as_str()) {
                return Err(DeviceDefinitionError::DuplicateParamId(p.id.clone()));
            }
            match p.binding {
                MidiBinding::Cc { cc } if cc > 127 => {
                    return Err(DeviceDefinitionError::InvalidCc {
                        param_id: p.id.clone(),
                        cc,
                    });
                }
                MidiBinding::Nrpn { msb, lsb, .. } | MidiBinding::Rpn { msb, lsb, .. }
                    if msb > 127 || lsb > 127 =>
                {
                    return Err(DeviceDefinitionError::InvalidParameterAddress {
                        param_id: p.id.clone(),
                        msb,
                        lsb,
                    });
                }
                _ => {}
            }
            if p.min > p.max {
                return Err(DeviceDefinitionError::MinGreaterThanMax {
                    param_id: p.id.clone(),
                    min: p.min,
                    max: p.max,
                });
            }
            let limit = p.binding.max_value();
            if p.max > limit {
                return Err(DeviceDefinitionError::BindingValueOutOfRange {
                    param_id: p.id.clone(),
                    value: p.max,
                    limit,
                });
            }
            if let Some(default) = p.default {
                if default < p.min || default > p.max {
                    return Err(DeviceDefinitionError::DefaultOutOfRange {
                        param_id: p.id.clone(),
                        default,
                        min: p.min,
                        max: p.max,
                    });
                }
            }
        }
        Ok(())
    }

    /// Find a parameter by its `id`.
    pub fn param(&self, id: &str) -> Option<&DeviceParam> {
        self.params.iter().find(|p| p.id == id)
    }

    /// Serialize to the stable on-disk JSON form (pretty-printed). JSON mirrors
    /// the rest of the crate's persistence ([`crate::midi_map`],
    /// [`crate::registry`]); `schema_version` rides along for forward-compat.
    pub fn to_json(&self) -> Result<String, DeviceJsonError> {
        serde_json::to_string_pretty(self).map_err(DeviceJsonError::Serialize)
    }

    /// Parse a definition from its on-disk JSON form. Does **not** validate —
    /// call [`DeviceDefinition::validate`] on the result before trusting it.
    pub fn from_json(bytes: &[u8]) -> Result<Self, DeviceJsonError> {
        serde_json::from_slice(bytes).map_err(DeviceJsonError::Parse)
    }
}

/// Apply a curve to a normalized `0.0..=1.0` value, returning the shaped
/// fraction (also `0.0..=1.0`). `0.0` and `1.0` are fixed points of every curve.
fn shape(curve: ParamCurve, norm: f32) -> f32 {
    match curve {
        ParamCurve::Linear => norm,
        ParamCurve::Exponential => norm.powf(CURVE_EXP),
        ParamCurve::Logarithmic => norm.powf(1.0 / CURVE_EXP),
    }
}

/// Inverse of [`shape`]: recover the normalized value from a shaped fraction.
fn unshape(curve: ParamCurve, frac: f32) -> f32 {
    match curve {
        ParamCurve::Linear => frac,
        ParamCurve::Exponential => frac.powf(1.0 / CURVE_EXP),
        ParamCurve::Logarithmic => frac.powf(CURVE_EXP),
    }
}

/// Map a normalized `0.0..=1.0` lane value onto the parameter's integer binding
/// value, applying its [`ParamCurve`] and clamping into `min..=max`. The result
/// is rounded to the nearest integer in the binding's domain.
pub fn lane_value_to_binding_value(param: &DeviceParam, norm: f32) -> u16 {
    let (lo, hi) = (param.min.min(param.max), param.min.max(param.max));
    let frac = shape(param.curve, norm.clamp(0.0, 1.0));
    let raw = lo as f32 + frac * (hi - lo) as f32;
    raw.round().clamp(lo as f32, hi as f32) as u16
}

/// Inverse of [`lane_value_to_binding_value`]: map a raw binding value back to a
/// normalized `0.0..=1.0` lane value, undoing the curve. A degenerate
/// `min == max` parameter maps everything to `0.0`. Round-trips with
/// `lane_value_to_binding_value` for every integer in `min..=max`.
pub fn binding_value_to_lane(param: &DeviceParam, raw: u16) -> f32 {
    let (lo, hi) = (param.min.min(param.max), param.min.max(param.max));
    if hi == lo {
        return 0.0;
    }
    let clamped = raw.clamp(lo, hi);
    let frac = (clamped - lo) as f32 / (hi - lo) as f32;
    unshape(param.curve, frac).clamp(0.0, 1.0)
}
