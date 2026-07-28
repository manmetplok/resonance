//! Config-driven activation registry for NAM models.
//!
//! Mirrors `NAM/activations.h`/`.cpp` from NeuralAmpModelerCore (MIT).
//! A `.nam` config's activation — either a plain string (`"Tanh"`) or an
//! A2-style object (`{"type": "LeakyReLU", "negative_slope": 0.2}`) — is
//! parsed into an [`ActivationConfig`], then resolved once at
//! model-construction time into a runtime [`Activation`] enum. `apply`
//! dispatches with a single `match` per call and never allocates, so it is
//! safe in the per-sample audio path.

use serde_json::Value;

use super::{fast_tanh, sigmoid as fast_sigmoid};

// -- Scalar formulas (per NAM/activations.h) ---------------------------------

#[inline(always)]
pub fn relu(x: f32) -> f32 {
    if x > 0.0 {
        x
    } else {
        0.0
    }
}

#[inline(always)]
pub fn leaky_relu(x: f32, negative_slope: f32) -> f32 {
    if x > 0.0 {
        x
    } else {
        negative_slope * x
    }
}

#[inline(always)]
pub fn hard_tanh(x: f32) -> f32 {
    let t = if x < -1.0 { -1.0 } else { x };
    if t > 1.0 {
        1.0
    } else {
        t
    }
}

#[inline(always)]
pub fn leaky_hardtanh(x: f32, min_val: f32, max_val: f32, min_slope: f32, max_slope: f32) -> f32 {
    if x < min_val {
        (x - min_val) * min_slope + min_val
    } else if x > max_val {
        (x - max_val) * max_slope + max_val
    } else {
        x
    }
}

/// Exact logistic sigmoid, as used by the reference `ActivationSigmoid`.
/// (Distinct from [`super::sigmoid`], the fast-tanh-derived approximation
/// used by the A1 gated path and the LSTM gates.)
#[inline(always)]
pub fn exact_sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// SiLU, aka Swish: `x * sigmoid(x)`.
#[inline(always)]
pub fn swish(x: f32) -> f32 {
    x * exact_sigmoid(x)
}

/// Hardswish: `x * clamp(x + 3, 0, 6) / 6`.
#[inline(always)]
pub fn hardswish(x: f32) -> f32 {
    let t = x + 3.0;
    x * t.clamp(0.0, 6.0) * (1.0 / 6.0)
}

#[inline(always)]
pub fn softsign(x: f32) -> f32 {
    x / (1.0 + x.abs())
}

// -- Parsed configuration ----------------------------------------------------

/// The activation kinds understood by the registry, matching the reference
/// `ActivationType` enum (plus `Identity`, the reference's do-nothing
/// `ActivationIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationKind {
    Identity,
    Tanh,
    FastTanh,
    HardTanh,
    LeakyHardTanh,
    Relu,
    LeakyRelu,
    PRelu,
    Sigmoid,
    Silu,
    Hardswish,
    Softsign,
}

impl ActivationKind {
    /// Map a NAM activation name to a kind, per the reference `type_map`
    /// (both `"LeakyHardtanh"` casings are accepted).
    pub fn from_name(name: &str) -> Result<Self, String> {
        Ok(match name {
            "Identity" => Self::Identity,
            "Tanh" => Self::Tanh,
            "Fasttanh" => Self::FastTanh,
            "Hardtanh" => Self::HardTanh,
            "LeakyHardtanh" | "LeakyHardTanh" => Self::LeakyHardTanh,
            "ReLU" => Self::Relu,
            "LeakyReLU" => Self::LeakyRelu,
            "PReLU" => Self::PRelu,
            "Sigmoid" => Self::Sigmoid,
            "SiLU" => Self::Silu,
            "Hardswish" => Self::Hardswish,
            "Softsign" => Self::Softsign,
            other => return Err(format!("Unknown activation type: {other}")),
        })
    }
}

/// Parsed activation configuration: a kind plus the optional parameters used
/// by specific kinds. Mirrors the reference `ActivationConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivationConfig {
    pub kind: ActivationKind,
    /// LeakyReLU / PReLU (single slope).
    pub negative_slope: Option<f32>,
    /// PReLU (per-channel slopes).
    pub negative_slopes: Option<Vec<f32>>,
    /// LeakyHardtanh.
    pub min_val: Option<f32>,
    pub max_val: Option<f32>,
    pub min_slope: Option<f32>,
    pub max_slope: Option<f32>,
}

impl ActivationConfig {
    /// A config with no parameters set.
    pub fn simple(kind: ActivationKind) -> Self {
        Self {
            kind,
            negative_slope: None,
            negative_slopes: None,
            min_val: None,
            max_val: None,
            min_slope: None,
            max_slope: None,
        }
    }

    /// Parse from a plain activation name string.
    pub fn from_name(name: &str) -> Result<Self, String> {
        Ok(Self::simple(ActivationKind::from_name(name)?))
    }

    /// Parse from a `.nam` config value: either a plain string
    /// (`"Tanh"`) or an A2-style object (`{"type": "...", ...params}`),
    /// per the reference `ActivationConfig::from_json`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(name) => Self::from_name(name),
            Value::Object(obj) => {
                let type_str = obj
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("Activation config object is missing a string \"type\" field")?;
                let mut config = Self::from_name(type_str)?;
                match config.kind {
                    ActivationKind::PRelu => {
                        if let Some(v) = obj.get("negative_slope") {
                            config.negative_slope = Some(json_f32(v, "negative_slope")?);
                        } else if let Some(v) = obj.get("negative_slopes") {
                            let arr = v
                                .as_array()
                                .ok_or("PReLU negative_slopes must be an array of numbers")?;
                            let mut slopes = Vec::with_capacity(arr.len());
                            for item in arr {
                                slopes.push(json_f32(item, "negative_slopes")?);
                            }
                            if slopes.is_empty() {
                                return Err("PReLU negative_slopes must not be empty".into());
                            }
                            config.negative_slopes = Some(slopes);
                        }
                    }
                    ActivationKind::LeakyRelu => {
                        config.negative_slope =
                            Some(json_f32_or(obj.get("negative_slope"), 0.01)?);
                    }
                    ActivationKind::LeakyHardTanh => {
                        config.min_val = Some(json_f32_or(obj.get("min_val"), -1.0)?);
                        config.max_val = Some(json_f32_or(obj.get("max_val"), 1.0)?);
                        config.min_slope = Some(json_f32_or(obj.get("min_slope"), 0.01)?);
                        config.max_slope = Some(json_f32_or(obj.get("max_slope"), 0.01)?);
                    }
                    _ => {}
                }
                Ok(config)
            }
            _ => Err("Invalid activation config: expected string or object".into()),
        }
    }
}

fn json_f32(value: &Value, field: &str) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| format!("Activation config field {field} must be a number"))
}

fn json_f32_or(value: Option<&Value>, default: f32) -> Result<f32, String> {
    match value {
        Some(Value::Null) | None => Ok(default),
        Some(v) => json_f32(v, "parameter"),
    }
}

// -- Runtime activation ------------------------------------------------------

/// A runtime activation function, fully resolved at model-construction time.
/// Dispatch is one `match` per `apply` call — no string lookups, trait
/// objects, or allocations in the audio path.
#[derive(Debug, Clone, PartialEq)]
pub enum Activation {
    Identity,
    /// Exact `tanh`.
    Tanh,
    /// NAM fast tanh approximation ([`super::fast_tanh`]) — what A1 models
    /// use for their `"Tanh"` activation (reference `enable_fast_tanh()`).
    FastTanh,
    HardTanh,
    LeakyHardTanh {
        min_val: f32,
        max_val: f32,
        min_slope: f32,
        max_slope: f32,
    },
    Relu,
    LeakyRelu {
        negative_slope: f32,
    },
    /// Per-channel negative slopes; channel = `pos % negative_slopes.len()`
    /// when applied to a slice (one value per channel).
    PRelu {
        negative_slopes: Vec<f32>,
    },
    /// Exact logistic sigmoid (reference `ActivationSigmoid`).
    Sigmoid,
    /// Fast-tanh-derived sigmoid ([`super::sigmoid`]). What a `Sigmoid`
    /// secondary (gate/blend) activation resolves to in fast-tanh mode (see
    /// [`Activation::secondary_from_config`]), so A1 gated output stays
    /// bit-identical to the previously hardcoded fast_tanh * fast_sigmoid
    /// path.
    FastSigmoid,
    Silu,
    Hardswish,
    Softsign,
}

impl Activation {
    /// Resolve a parsed config into a runtime activation.
    ///
    /// `fast_tanh_mode` mirrors the reference core's `enable_fast_tanh()`
    /// (always on in the NAM plugin): `Tanh` resolves to [`Activation::FastTanh`].
    /// Model construction passes `true`, which keeps A1 models (config
    /// activation `"Tanh"`) on today's bit-identical fast_tanh path.
    pub fn from_config(config: &ActivationConfig, fast_tanh_mode: bool) -> Self {
        match config.kind {
            ActivationKind::Identity => Self::Identity,
            ActivationKind::Tanh => {
                if fast_tanh_mode {
                    Self::FastTanh
                } else {
                    Self::Tanh
                }
            }
            ActivationKind::FastTanh => Self::FastTanh,
            ActivationKind::HardTanh => Self::HardTanh,
            ActivationKind::LeakyHardTanh => Self::LeakyHardTanh {
                min_val: config.min_val.unwrap_or(-1.0),
                max_val: config.max_val.unwrap_or(1.0),
                min_slope: config.min_slope.unwrap_or(0.01),
                max_slope: config.max_slope.unwrap_or(0.01),
            },
            ActivationKind::Relu => Self::Relu,
            ActivationKind::LeakyRelu => Self::LeakyRelu {
                negative_slope: config.negative_slope.unwrap_or(0.01),
            },
            ActivationKind::PRelu => Self::PRelu {
                negative_slopes: config
                    .negative_slopes
                    .clone()
                    .unwrap_or_else(|| vec![config.negative_slope.unwrap_or(0.01)]),
            },
            ActivationKind::Sigmoid => Self::Sigmoid,
            ActivationKind::Silu => Self::Silu,
            ActivationKind::Hardswish => Self::Hardswish,
            ActivationKind::Softsign => Self::Softsign,
        }
    }

    /// Resolve a parsed secondary (gate/blend) activation config.
    ///
    /// Same as [`Activation::from_config`], except that in fast-tanh mode a
    /// `Sigmoid` secondary resolves to the fast sigmoid — the engine-wide
    /// fast-mode convention (mirroring `Tanh` -> fast tanh). The default
    /// secondary of gated/blended layers is `Sigmoid` (reference
    /// backward-compat), so this keeps A1 gated models bit-identical to the
    /// historical hardcoded `fast_tanh(z) * fast_sigmoid(g)` path.
    pub fn secondary_from_config(config: &ActivationConfig, fast_tanh_mode: bool) -> Self {
        match config.kind {
            ActivationKind::Sigmoid if fast_tanh_mode => Self::FastSigmoid,
            _ => Self::from_config(config, fast_tanh_mode),
        }
    }

    /// Apply to a single value. For [`Activation::PRelu`] this uses the first
    /// channel's slope (use [`Activation::apply`] for per-channel behavior).
    #[inline]
    pub fn scalar(&self, x: f32) -> f32 {
        match self {
            Self::Identity => x,
            Self::Tanh => x.tanh(),
            Self::FastTanh => fast_tanh(x),
            Self::HardTanh => hard_tanh(x),
            Self::LeakyHardTanh {
                min_val,
                max_val,
                min_slope,
                max_slope,
            } => leaky_hardtanh(x, *min_val, *max_val, *min_slope, *max_slope),
            Self::Relu => relu(x),
            Self::LeakyRelu { negative_slope } => leaky_relu(x, *negative_slope),
            Self::PRelu { negative_slopes } => {
                leaky_relu(x, negative_slopes.first().copied().unwrap_or(0.01))
            }
            Self::Sigmoid => exact_sigmoid(x),
            Self::FastSigmoid => fast_sigmoid(x),
            Self::Silu => swish(x),
            Self::Hardswish => hardswish(x),
            Self::Softsign => softsign(x),
        }
    }

    /// Apply in place to a contiguous slice (one value per channel for a
    /// single time step). One `match`, no allocation.
    #[inline]
    pub fn apply(&self, data: &mut [f32]) {
        match self {
            Self::Identity => {}
            Self::Tanh => {
                for v in data.iter_mut() {
                    *v = v.tanh();
                }
            }
            Self::FastTanh => {
                for v in data.iter_mut() {
                    *v = fast_tanh(*v);
                }
            }
            Self::HardTanh => {
                for v in data.iter_mut() {
                    *v = hard_tanh(*v);
                }
            }
            Self::LeakyHardTanh {
                min_val,
                max_val,
                min_slope,
                max_slope,
            } => {
                for v in data.iter_mut() {
                    *v = leaky_hardtanh(*v, *min_val, *max_val, *min_slope, *max_slope);
                }
            }
            Self::Relu => {
                for v in data.iter_mut() {
                    *v = relu(*v);
                }
            }
            Self::LeakyRelu { negative_slope } => {
                for v in data.iter_mut() {
                    *v = leaky_relu(*v, *negative_slope);
                }
            }
            Self::PRelu { negative_slopes } => {
                let n = negative_slopes.len();
                if n == 0 {
                    return;
                }
                for (pos, v) in data.iter_mut().enumerate() {
                    *v = leaky_relu(*v, negative_slopes[pos % n]);
                }
            }
            Self::Sigmoid => {
                for v in data.iter_mut() {
                    *v = exact_sigmoid(*v);
                }
            }
            Self::FastSigmoid => {
                for v in data.iter_mut() {
                    *v = fast_sigmoid(*v);
                }
            }
            Self::Silu => {
                for v in data.iter_mut() {
                    *v = swish(*v);
                }
            }
            Self::Hardswish => {
                for v in data.iter_mut() {
                    *v = hardswish(*v);
                }
            }
            Self::Softsign => {
                for v in data.iter_mut() {
                    *v = softsign(*v);
                }
            }
        }
    }
}
