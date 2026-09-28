//! Real ↔ normalized lane values (automation-control-api.md §4.2).
//!
//! A lane stores `0..=1`; the wire speaks the target's real units. Every
//! conversion goes through `resonance_common`'s [`real_to_lane_value`] /
//! [`lane_value_to_real`] (mixer lanes) and [`plugin_param_to_lane_value`]
//! / [`lane_value_to_plugin_param`] (plugin lanes) — the same functions
//! the engine and the GUI labels use — so nothing here duplicates a
//! mapping; it only adds the wire's vocabulary (`"-inf"`, bools, choice
//! labels) and its range checks.

use crate::update::control::track::resolve_param_value;
use resonance_common::{
    lane_value_to_plugin_param, lane_value_to_real, plugin_param_to_lane_value, real_to_lane_value,
    AutomationTarget, CurveKind, GAIN_MAX_DB, GAIN_MIN_DB,
};
use resonance_control::methods::automation::AutomationValue;
use resonance_control::methods::track::{ParamValue, PluginParamView};
use resonance_control::RpcError;

/// What a lane's values mean: which real range a normalized value maps to.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::update::control) enum ValueDomain {
    /// A fader, dB in `GAIN_MIN_DB..=GAIN_MAX_DB`; the floor is silence.
    Gain,
    /// Stereo balance, `-1..=1`.
    Pan,
    /// A bool.
    Mute,
    /// A plugin parameter, linear in its own `min..=max`.
    Plugin(PluginParamView),
    /// The range is unknown — a device lane, an orphan, a plugin that is
    /// missing or still initializing. Values read back normalized.
    Normalized,
}

impl ValueDomain {
    /// The domain of a mixer (gain / pan / mute) target; `Normalized` for
    /// plugin and device targets, whose range lives outside the target.
    pub(in crate::update::control) fn for_mixer(target: &AutomationTarget) -> Self {
        match target {
            AutomationTarget::TrackGain(_)
            | AutomationTarget::BusGain(_)
            | AutomationTarget::MasterGain => Self::Gain,
            AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => Self::Pan,
            AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => Self::Mute,
            AutomationTarget::PluginParam { .. } | AutomationTarget::DeviceParam { .. } => {
                Self::Normalized
            }
        }
    }

    /// The real unit's name on the wire.
    pub(in crate::update::control) fn unit(&self) -> String {
        match self {
            Self::Gain => "dB".to_owned(),
            Self::Mute => "bool".to_owned(),
            Self::Plugin(p) => p.unit.clone(),
            Self::Pan | Self::Normalized => String::new(),
        }
    }

    /// The real value at normalized 0, when known.
    pub(in crate::update::control) fn min(&self) -> Option<f64> {
        match self {
            Self::Gain => Some(f64::from(GAIN_MIN_DB)),
            Self::Pan => Some(-1.0),
            Self::Mute => Some(0.0),
            Self::Plugin(p) => Some(p.min),
            Self::Normalized => None,
        }
    }

    /// The real value at normalized 1, when known.
    pub(in crate::update::control) fn max(&self) -> Option<f64> {
        match self {
            Self::Gain => Some(f64::from(GAIN_MAX_DB)),
            Self::Pan | Self::Mute => Some(1.0),
            Self::Plugin(p) => Some(p.max),
            Self::Normalized => None,
        }
    }

    /// The curve a point gets when the caller names none: `stepped` for
    /// the discrete targets (mute, a stepped plugin parameter), where a
    /// ramp between two values means nothing, `linear` otherwise.
    pub(in crate::update::control) fn default_curve(&self) -> CurveKind {
        match self {
            Self::Mute => CurveKind::Stepped,
            Self::Plugin(p) if p.stepped => CurveKind::Stepped,
            _ => CurveKind::Linear,
        }
    }

    /// The stored `0..=1` value `value` denotes on `target`, or why it
    /// denotes none. `normalized` is the call's `normalized` flag.
    pub(in crate::update::control) fn normalize(
        &self,
        target: &AutomationTarget,
        value: &AutomationValue,
        normalized: bool,
    ) -> Result<f32, RpcError> {
        if normalized {
            return normalized_input(value);
        }
        match self {
            Self::Gain => {
                if is_minus_inf(value) {
                    return Ok(0.0);
                }
                let db = number(value).ok_or_else(|| gain_error(value))?;
                if !db.is_finite() || db < f64::from(GAIN_MIN_DB) || db > f64::from(GAIN_MAX_DB) {
                    return Err(gain_error(value));
                }
                Ok(real_to_lane_value(target, db as f32))
            }
            Self::Pan => {
                let pan = number(value)
                    .filter(|p| p.is_finite() && (-1.0..=1.0).contains(p))
                    .ok_or_else(|| {
                        RpcError::invalid_params(format!(
                            "pan must be within -1..=1 (got {})",
                            describe(value)
                        ))
                    })?;
                Ok(real_to_lane_value(target, pan as f32))
            }
            Self::Mute => {
                let muted = match value {
                    AutomationValue::Bool(b) => Some(*b),
                    AutomationValue::Number(n) if *n == 0.0 => Some(false),
                    AutomationValue::Number(n) if *n == 1.0 => Some(true),
                    _ => None,
                }
                .ok_or_else(|| {
                    RpcError::invalid_params(format!(
                        "mute takes true / false (or 0 / 1), got {}",
                        describe(value)
                    ))
                })?;
                Ok(real_to_lane_value(target, if muted { 1.0 } else { 0.0 }))
            }
            Self::Plugin(param) => {
                // The same resolution `*.set_plugin_param` applies: choice
                // labels, finiteness, the f32-declared-bounds tolerance.
                let requested = match value {
                    AutomationValue::Number(n) => ParamValue::Number(*n),
                    AutomationValue::Bool(b) => ParamValue::Number(if *b { 1.0 } else { 0.0 }),
                    AutomationValue::Text(t) => ParamValue::Label(t.clone()),
                };
                let real = resolve_param_value(param, &requested)?;
                Ok(plugin_param_to_lane_value(real, param.min, param.max))
            }
            Self::Normalized => Err(RpcError::invalid_params(
                "this lane's real range is unknown (its plugin is missing or still \
                 initializing); pass normalized: true with values in 0..=1",
            )),
        }
    }

    /// The real value a stored `norm` reads back as, plus its text.
    ///
    /// Real numbers are rounded to the range's own precision (six
    /// significant digits of its span) so an f32 lane value reads back as
    /// the decimal the caller wrote — `300`, not `300.00001` — and sending
    /// it back stores the identical normalized value.
    pub(in crate::update::control) fn real(
        &self,
        target: &AutomationTarget,
        norm: f32,
    ) -> (AutomationValue, String) {
        match self {
            Self::Gain => {
                if norm <= 0.0 {
                    return (
                        AutomationValue::Text("-inf".to_owned()),
                        "-inf dB".to_owned(),
                    );
                }
                let db = round_to_span(
                    f64::from(lane_value_to_real(target, norm)),
                    f64::from(GAIN_MAX_DB - GAIN_MIN_DB),
                );
                (AutomationValue::Number(db), format!("{db} dB"))
            }
            Self::Pan => {
                let pan = round_to_span(f64::from(lane_value_to_real(target, norm)), 2.0);
                (AutomationValue::Number(pan), format!("{pan}"))
            }
            Self::Mute => {
                let muted = lane_value_to_real(target, norm) >= 0.5;
                (
                    AutomationValue::Bool(muted),
                    if muted { "muted" } else { "unmuted" }.to_owned(),
                )
            }
            Self::Plugin(param) => {
                let raw = lane_value_to_plugin_param(norm, param.min, param.max);
                if param.stepped {
                    let step = (raw - param.min).round();
                    let value = param.min + step;
                    let text = usize::try_from(step as i64)
                        .ok()
                        .and_then(|i| param.choices.get(i))
                        .cloned()
                        .unwrap_or_else(|| with_unit(value, &param.unit));
                    return (AutomationValue::Number(value), text);
                }
                let value = round_to_span(raw, param.max - param.min);
                (
                    AutomationValue::Number(value),
                    with_unit(value, &param.unit),
                )
            }
            Self::Normalized => {
                let n = f64::from(norm);
                (AutomationValue::Number(n), format!("{n}"))
            }
        }
    }
}

/// A `normalized: true` value: a number in `0..=1` (a bool reads as 0/1).
fn normalized_input(value: &AutomationValue) -> Result<f32, RpcError> {
    let n = match value {
        AutomationValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        other => number(other),
    };
    match n {
        Some(n) if n.is_finite() && (0.0..=1.0).contains(&n) => Ok(n as f32),
        _ => Err(RpcError::invalid_params(format!(
            "with normalized: true every value must be within 0..=1 (got {})",
            describe(value)
        ))),
    }
}

/// A number, including one that arrived as a numeric string.
fn number(value: &AutomationValue) -> Option<f64> {
    match value {
        AutomationValue::Number(n) => Some(*n),
        AutomationValue::Text(t) => t.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        AutomationValue::Bool(_) => None,
    }
}

/// `"-inf"` in any of the spellings a client plausibly sends.
fn is_minus_inf(value: &AutomationValue) -> bool {
    matches!(value, AutomationValue::Text(t)
        if matches!(t.trim().to_ascii_lowercase().as_str(), "-inf" | "-infinity"))
}

fn gain_error(value: &AutomationValue) -> RpcError {
    RpcError::invalid_params(format!(
        "volume must be within {GAIN_MIN_DB}..={GAIN_MAX_DB} dB, or \"-inf\" for silence \
         (got {})",
        describe(value)
    ))
}

/// A value as an error message quotes it.
fn describe(value: &AutomationValue) -> String {
    match value {
        AutomationValue::Number(n) => format!("{n}"),
        AutomationValue::Bool(b) => format!("{b}"),
        AutomationValue::Text(t) => format!("{t:?}"),
    }
}

fn with_unit(value: f64, unit: &str) -> String {
    if unit.is_empty() {
        format!("{value}")
    } else {
        format!("{value} {unit}")
    }
}

/// `value` rounded to six significant digits of `span` — the precision
/// an f32 lane value actually carries over that range.
pub(in crate::update::control) fn round_to_span(value: f64, span: f64) -> f64 {
    if !value.is_finite() || !span.is_finite() || span <= 0.0 {
        return value;
    }
    let decimals = (5 - span.log10().floor() as i32).clamp(0, 12) as usize;
    let rounded: f64 = format!("{value:.decimals$}").parse().unwrap_or(value);
    // Never report "-0".
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}
