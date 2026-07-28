//! Typed A2 WaveNet configuration surface.
//!
//! Mirrors `NAM/wavenet/params.h` and the JSON handling in
//! `NAM/wavenet/model.cpp` (`parse_config_json`) plus the slimmable
//! descriptor extraction in `NAM/wavenet/slimmable.cpp` from
//! NeuralAmpModelerCore (MIT). Every field defaults to its A1-equivalent
//! value, so a plain A1 layer-array config parses to the same effective
//! configuration the engine uses today.
//!
//! Model construction does not consume [`WaveNetFullConfig`] wholesale yet;
//! the A2 inference todos (#1105..#1113) wire it into `WaveNetModel`
//! construction area by area. The gating surface ([`GatingMode`] + secondary
//! activations, via `parse_gating_config`) is shared with the engine's
//! legacy config path in `parse.rs` and drives inference already.

use serde_json::Value;

use crate::nam::activations::{ActivationConfig, ActivationKind};

/// Gating mode for a WaveNet layer (reference `GatingMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatingMode {
    /// No gating — standard activation.
    None,
    /// Traditional gating: element-wise product of the two bottleneck halves.
    Gated,
    /// Blending: weighted average between activated and pre-activated values.
    Blended,
}

impl GatingMode {
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "none" => Ok(Self::None),
            "gated" => Ok(Self::Gated),
            "blended" => Ok(Self::Blended),
            other => Err(format!("Invalid gating_mode: {other}")),
        }
    }
}

/// Optional 1x1 convolution feeding the residual path (reference
/// `Layer1x1Params`). A1 default: active with a single group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer1x1Params {
    pub active: bool,
    pub groups: usize,
}

impl Default for Layer1x1Params {
    fn default() -> Self {
        Self {
            active: true,
            groups: 1,
        }
    }
}

/// Optional 1x1 convolution feeding the head/skip path (reference
/// `Head1x1Params`). A1 default: inactive; `out_channels` defaults to the
/// layer array's `channels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head1x1Params {
    pub active: bool,
    pub out_channels: usize,
    pub groups: usize,
}

impl Head1x1Params {
    /// The A1 default: no head1x1 conv, with `out_channels` mirroring the
    /// reference's absent-object fallback (`head1x1_out_channels = channels`
    /// in NAM/wavenet/model.cpp `parse_config_json`).
    pub fn inactive(out_channels: usize) -> Self {
        Self {
            active: false,
            out_channels,
            groups: 1,
        }
    }
}

/// FiLM (feature-wise linear modulation) insertion-point configuration
/// (reference `_FiLMParams`). A1 default: inactive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmParams {
    pub active: bool,
    /// Whether the conditioning applies scale + shift (`true`) or scale only.
    pub shift: bool,
    /// Groups of the condition-to-scale/shift submodule convolution.
    pub groups: usize,
}

impl Default for FilmParams {
    fn default() -> Self {
        Self {
            active: false,
            shift: false,
            groups: 1,
        }
    }
}

impl FilmParams {
    /// Parse one FiLM insertion-point block from its raw JSON value (absent
    /// or `null` passed as `None`): `None` or literal `false` means
    /// inactive; an object defaults to `{active: true, shift: true,
    /// groups: 1}` (reference `parse_film_params` in
    /// NAM/wavenet/model.cpp). Shared between the typed A2 parse and the
    /// engine config path in `parse.rs` (single source of FiLM-block
    /// semantics).
    pub(crate) fn from_json(value: Option<&Value>, ctx: &str, key: &str) -> Result<Self, String> {
        match value {
            None | Some(Value::Bool(false)) => Ok(Self::default()),
            Some(Value::Object(f)) => {
                let fctx = format!("{ctx}: {key}");
                Ok(Self {
                    active: opt_bool_or(f, "active", true, &fctx)?,
                    shift: opt_bool_or(f, "shift", true, &fctx)?,
                    groups: opt_usize_or(f, "groups", 1, &fctx)?,
                })
            }
            Some(_) => Err(format!("{ctx}: {key} must be a JSON object or false")),
        }
    }
}

/// The 8 per-layer FiLM insertion points of one layer array, in reference
/// weight-consumption order (`Layer::set_weights_` in
/// NAM/wavenet/model.cpp: conv_pre, conv_post, input_mixin_pre,
/// input_mixin_post, activation_pre, activation_post, layer1x1_post,
/// head1x1_post — all after the layer's conv/input_mixin/layer1x1/head1x1
/// tensors). Default: all inactive (A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerFilms {
    pub conv_pre: FilmParams,
    pub conv_post: FilmParams,
    pub input_mixin_pre: FilmParams,
    pub input_mixin_post: FilmParams,
    pub activation_pre: FilmParams,
    pub activation_post: FilmParams,
    pub layer1x1_post: FilmParams,
    pub head1x1_post: FilmParams,
}

/// Slimmable packed-weight descriptor for one layer array (reference
/// `NAM/wavenet/slimmable.cpp`). Only the `slice_channels_uniform` method
/// exists; unknown methods are a parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlimmableParams {
    /// Allowed channel counts, sorted ascending; the last entry equals the
    /// layer array's full channel count.
    pub allowed_channels: Vec<usize>,
}

/// Top-level output head (reference `HeadParams` / legacy MLP head export).
#[derive(Debug, Clone, PartialEq)]
pub enum HeadParams {
    /// Legacy `.nam` export: an MLP with `num_layers` hidden layers of
    /// `channels` units each (what the engine's head supports today).
    Mlp {
        channels: usize,
        num_layers: usize,
        out_channels: usize,
    },
    /// A2 windowed/convolutional head with per-layer kernel sizes and its
    /// own activation (reference `HeadParams`).
    Windowed {
        /// Input channels; implied by the last layer array's `head_size`
        /// (validated against an explicit `in_channels` when present).
        in_channels: usize,
        channels: usize,
        out_channels: usize,
        kernel_sizes: Vec<usize>,
        activation: ActivationConfig,
    },
}

/// Full typed configuration of one layer array (reference
/// `LayerArrayParams`). The per-layer vectors (`kernel_sizes`, `dilations`,
/// `activations`, `gating_modes`, `secondary_activations`) all have the same
/// length: one entry per dilated layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerArrayParams {
    pub input_size: usize,
    pub condition_size: usize,
    pub channels: usize,
    /// Internal channel count (A2). Defaults to `channels` (A1).
    pub bottleneck: usize,
    /// Head/skip output size of this array (`head.out_channels`, or legacy
    /// `head_size`).
    pub head_size: usize,
    /// Kernel size of the head rechannel conv (`head.kernel_size`, A1: 1).
    pub head_kernel_size: usize,
    /// Dilation of the head rechannel conv (`head.head_dilation`, A1: 1).
    pub head_dilation: usize,
    /// Bias on the head rechannel conv (`head.bias`, or legacy `head_bias`).
    pub head_bias: bool,
    pub dilations: Vec<usize>,
    pub kernel_sizes: Vec<usize>,
    /// Primary activation per layer.
    pub activations: Vec<ActivationConfig>,
    /// Gating mode per layer (mixed per-layer modes are valid in A2).
    pub gating_modes: Vec<GatingMode>,
    /// Secondary (gate) activation per layer. `None` exactly when the
    /// layer's gating mode is [`GatingMode::None`]; a gated/blended layer
    /// without an explicit config defaults to `Sigmoid` (reference
    /// backward-compat).
    pub secondary_activations: Vec<Option<ActivationConfig>>,
    /// Groups of the dilated input convolution. A1: 1.
    pub groups_input: usize,
    /// Groups of the condition input-mixin convolution. A1: 1.
    pub groups_input_mixin: usize,
    pub layer1x1: Layer1x1Params,
    pub head1x1: Head1x1Params,
    pub conv_pre_film: FilmParams,
    pub conv_post_film: FilmParams,
    pub input_mixin_pre_film: FilmParams,
    pub input_mixin_post_film: FilmParams,
    pub activation_pre_film: FilmParams,
    pub activation_post_film: FilmParams,
    pub layer1x1_post_film: FilmParams,
    pub head1x1_post_film: FilmParams,
    /// Slimmable packed-weight descriptor, when this array is slimmable.
    pub slimmable: Option<SlimmableParams>,
}

/// Full typed A2 WaveNet config (reference `WaveNetConfig` in model.cpp).
#[derive(Debug, Clone, PartialEq)]
pub struct WaveNetFullConfig {
    pub layer_arrays: Vec<LayerArrayParams>,
    pub head: Option<HeadParams>,
    /// Output scale. Kept optional: the engine does not consume it yet and
    /// some historical exports omit it.
    pub head_scale: Option<f32>,
    /// Input channel count (A2 multi-channel). A1: 1.
    pub in_channels: usize,
    /// Raw `condition_dsp` sub-model JSON (a nested .nam-style model
    /// object). Kept untyped until the condition_dsp inference todo.
    pub condition_dsp: Option<Value>,
}

// -- JSON helpers -------------------------------------------------------------

/// Non-null JSON value for `key`, if present. `null` counts as absent,
/// matching the reference's `is_null()` guards.
fn non_null<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.get(key).filter(|v| !v.is_null())
}

fn value_usize(v: &Value, what: &str) -> Result<usize, String> {
    v.as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| format!("{what} must be a non-negative integer"))
}

fn req_usize(obj: &serde_json::Map<String, Value>, key: &str, ctx: &str) -> Result<usize, String> {
    let v = non_null(obj, key).ok_or_else(|| format!("{ctx}: missing required field {key}"))?;
    value_usize(v, &format!("{ctx}: {key}"))
}

fn opt_usize_or(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    default: usize,
    ctx: &str,
) -> Result<usize, String> {
    match non_null(obj, key) {
        Some(v) => value_usize(v, &format!("{ctx}: {key}")),
        None => Ok(default),
    }
}

fn req_bool(obj: &serde_json::Map<String, Value>, key: &str, ctx: &str) -> Result<bool, String> {
    non_null(obj, key)
        .ok_or_else(|| format!("{ctx}: missing required field {key}"))?
        .as_bool()
        .ok_or_else(|| format!("{ctx}: {key} must be a boolean"))
}

fn opt_bool_or(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    default: bool,
    ctx: &str,
) -> Result<bool, String> {
    match non_null(obj, key) {
        Some(v) => v
            .as_bool()
            .ok_or_else(|| format!("{ctx}: {key} must be a boolean")),
        None => Ok(default),
    }
}

fn usize_array(v: &Value, what: &str) -> Result<Vec<usize>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what} must be an array"))?
        .iter()
        .map(|item| value_usize(item, what))
        .collect()
}

// -- Parsing ------------------------------------------------------------------

impl WaveNetFullConfig {
    /// Parse a WaveNet `config` JSON value covering the full A2 surface.
    ///
    /// Accepts both plain A1 layer-array configs (all A2 fields fall back to
    /// A1-equivalent defaults) and every A2 extension: activation config
    /// objects/arrays, `bottleneck`, `gating_mode` incl. `blended` +
    /// `secondary_activation`, grouped convolutions, `layer1x1`/`head1x1`
    /// objects, the 8 FiLM insertion points, windowed heads, `condition_dsp`,
    /// and `slimmable` packed-weight descriptors.
    pub fn from_json(config: &Value) -> Result<Self, String> {
        let obj = config
            .as_object()
            .ok_or("WaveNet config must be a JSON object")?;

        let condition_dsp = non_null(obj, "condition_dsp").cloned();

        let layers_json = non_null(obj, "layers")
            .and_then(Value::as_array)
            .ok_or("WaveNet config requires a \"layers\" array")?;
        if layers_json.is_empty() {
            return Err("WaveNet config requires at least one layer array".into());
        }
        let layer_arrays: Vec<LayerArrayParams> = layers_json
            .iter()
            .enumerate()
            .map(|(i, l)| LayerArrayParams::from_json(i, l))
            .collect::<Result<_, String>>()?;

        let head = match non_null(obj, "head") {
            Some(v) => {
                // Implied input size: the last layer array's head output.
                let implied_in = layer_arrays
                    .last()
                    .expect("layer_arrays is non-empty")
                    .head_size;
                Some(HeadParams::from_json(v, implied_in)?)
            }
            None => None,
        };

        let head_scale = match non_null(obj, "head_scale") {
            Some(v) => Some(
                v.as_f64()
                    .map(|f| f as f32)
                    .ok_or("WaveNet config: head_scale must be a number")?,
            ),
            None => None,
        };

        let in_channels = opt_usize_or(obj, "in_channels", 1, "WaveNet config")?;

        Ok(Self {
            layer_arrays,
            head,
            head_scale,
            in_channels,
            condition_dsp,
        })
    }
}

impl HeadParams {
    fn from_json(value: &Value, implied_in_channels: usize) -> Result<Self, String> {
        let ctx = "WaveNet config: head";
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{ctx} must be a JSON object"))?;

        // Legacy MLP export is identified by its `num_layers` field; the A2
        // windowed head has `kernel_sizes` + `activation` instead.
        if non_null(obj, "num_layers").is_some() {
            return Ok(Self::Mlp {
                channels: req_usize(obj, "channels", ctx)?,
                num_layers: req_usize(obj, "num_layers", ctx)?,
                out_channels: req_usize(obj, "out_channels", ctx)?,
            });
        }

        // The new trainer export omits in_channels (implied by the last
        // layer array's head_size); legacy files that include it must agree.
        if let Some(v) = non_null(obj, "in_channels") {
            let explicit = value_usize(v, &format!("{ctx}: in_channels"))?;
            if explicit != implied_in_channels {
                return Err(format!(
                    "{ctx}: in_channels ({explicit}) must equal last layer array's head_size ({implied_in_channels})"
                ));
            }
        }
        let kernel_sizes = usize_array(
            non_null(obj, "kernel_sizes")
                .ok_or_else(|| format!("{ctx}: missing required field kernel_sizes"))?,
            &format!("{ctx}: kernel_sizes"),
        )?;
        if kernel_sizes.is_empty() {
            return Err(format!("{ctx}: kernel_sizes must be non-empty"));
        }
        let activation = ActivationConfig::from_json(
            non_null(obj, "activation")
                .ok_or_else(|| format!("{ctx}: missing required field activation"))?,
        )?;
        Ok(Self::Windowed {
            in_channels: implied_in_channels,
            channels: req_usize(obj, "channels", ctx)?,
            out_channels: req_usize(obj, "out_channels", ctx)?,
            kernel_sizes,
            activation,
        })
    }
}

impl LayerArrayParams {
    fn from_json(index: usize, value: &Value) -> Result<Self, String> {
        let ctx = format!("Layer array {index}");
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{ctx}: layer array config must be a JSON object"))?;

        let input_size = req_usize(obj, "input_size", &ctx)?;
        let condition_size = req_usize(obj, "condition_size", &ctx)?;
        let channels = req_usize(obj, "channels", &ctx)?;
        let bottleneck = opt_usize_or(obj, "bottleneck", channels, &ctx)?;
        let groups_input = opt_usize_or(obj, "groups_input", 1, &ctx)?;
        let groups_input_mixin = opt_usize_or(obj, "groups_input_mixin", 1, &ctx)?;

        // Head rechannel: prefer the nested "head" object (A2/trainer
        // export); legacy files use flat head_size + head_bias (kernel 1).
        let (head_size, head_kernel_size, head_dilation, head_bias) = match non_null(obj, "head") {
            Some(v) => {
                let h = v
                    .as_object()
                    .ok_or_else(|| format!("{ctx}: 'head' must be a JSON object"))?;
                let hctx = format!("{ctx}: head");
                (
                    req_usize(h, "out_channels", &hctx)?,
                    req_usize(h, "kernel_size", &hctx)?,
                    opt_usize_or(h, "head_dilation", 1, &hctx)?,
                    req_bool(h, "bias", &hctx)?,
                )
            }
            None => {
                let head_size = match non_null(obj, "head_size") {
                    Some(v) => value_usize(v, &format!("{ctx}: head_size"))?,
                    None => {
                        return Err(format!(
                            "{ctx}: expected 'head' object with out_channels, kernel_size, and bias, or legacy 'head_size' and 'head_bias'"
                        ));
                    }
                };
                // head_bias defaults to true when absent (some A1 exports
                // omit it; matches the engine's current lenient parsing).
                (head_size, 1, 1, opt_bool_or(obj, "head_bias", true, &ctx)?)
            }
        };
        if head_kernel_size < 1 {
            return Err(format!("{ctx}: head.kernel_size must be >= 1"));
        }

        let dilations = usize_array(
            non_null(obj, "dilations")
                .ok_or_else(|| format!("{ctx}: missing required field dilations"))?,
            &format!("{ctx}: dilations"),
        )?;
        if dilations.is_empty() {
            return Err(format!("{ctx}: dilations must be non-empty"));
        }
        let num_layers = dilations.len();

        let kernel_sizes = parse_kernel_sizes(obj, num_layers, &ctx)?;
        let activations = parse_activations(obj, num_layers, &ctx)?;
        let (gating_modes, secondary_activations) = parse_gating(obj, num_layers, &ctx)?;

        let layer1x1 = match non_null(obj, "layer1x1") {
            Some(v) => {
                let l = v
                    .as_object()
                    .ok_or_else(|| format!("{ctx}: layer1x1 must be a JSON object"))?;
                let lctx = format!("{ctx}: layer1x1");
                Layer1x1Params {
                    active: req_bool(l, "active", &lctx)?,
                    groups: req_usize(l, "groups", &lctx)?,
                }
            }
            None => Layer1x1Params::default(),
        };

        let head1x1 = match non_null(obj, "head1x1") {
            Some(v) => {
                let h = v
                    .as_object()
                    .ok_or_else(|| format!("{ctx}: head1x1 must be a JSON object"))?;
                let hctx = format!("{ctx}: head1x1");
                Head1x1Params {
                    active: req_bool(h, "active", &hctx)?,
                    out_channels: req_usize(h, "out_channels", &hctx)?,
                    groups: req_usize(h, "groups", &hctx)?,
                }
            }
            None => Head1x1Params::inactive(channels),
        };

        let film = |key: &str| parse_film(obj, key, &ctx);
        let layer1x1_post_film = film("layer1x1_post_film")?;
        if layer1x1_post_film.active && !layer1x1.active {
            return Err(format!(
                "{ctx}: layer1x1_post_film cannot be active when layer1x1.active is false"
            ));
        }

        let slimmable = parse_slimmable(obj, channels, &ctx)?;

        Ok(Self {
            input_size,
            condition_size,
            channels,
            bottleneck,
            head_size,
            head_kernel_size,
            head_dilation,
            head_bias,
            dilations,
            kernel_sizes,
            activations,
            gating_modes,
            secondary_activations,
            groups_input,
            groups_input_mixin,
            layer1x1,
            head1x1,
            conv_pre_film: film("conv_pre_film")?,
            conv_post_film: film("conv_post_film")?,
            input_mixin_pre_film: film("input_mixin_pre_film")?,
            input_mixin_post_film: film("input_mixin_post_film")?,
            activation_pre_film: film("activation_pre_film")?,
            activation_post_film: film("activation_post_film")?,
            layer1x1_post_film,
            head1x1_post_film: film("head1x1_post_film")?,
            slimmable,
        })
    }
}

/// Legacy single `kernel_size` (duplicated per layer) or per-layer
/// `kernel_sizes`; exactly one may be present. Absent defaults to kernel 2
/// (matches the engine's current lenient A1 parsing).
fn parse_kernel_sizes(
    obj: &serde_json::Map<String, Value>,
    num_layers: usize,
    ctx: &str,
) -> Result<Vec<usize>, String> {
    let single = non_null(obj, "kernel_size");
    let per_layer = non_null(obj, "kernel_sizes");
    match (single, per_layer) {
        (Some(_), Some(_)) => Err(format!(
            "{ctx}: only one of kernel_size (int) or kernel_sizes (array) may be provided"
        )),
        (None, Some(v)) => {
            let ks = usize_array(v, &format!("{ctx}: kernel_sizes"))?;
            if ks.len() != num_layers {
                return Err(format!(
                    "{ctx}: kernel_sizes array size ({}) must match dilations size ({num_layers})",
                    ks.len()
                ));
            }
            Ok(ks)
        }
        (Some(v), None) => {
            let k = value_usize(v, &format!("{ctx}: kernel_size"))?;
            Ok(vec![k; num_layers])
        }
        (None, None) => Ok(vec![2; num_layers]),
    }
}

/// Primary activation: a single config (string or object, duplicated per
/// layer) or a per-layer array. Absent defaults to `"Tanh"` (A1).
fn parse_activations(
    obj: &serde_json::Map<String, Value>,
    num_layers: usize,
    ctx: &str,
) -> Result<Vec<ActivationConfig>, String> {
    parse_activation_value(non_null(obj, "activation"), num_layers, ctx)
}

/// Shared primary-activation parsing (typed A2 parser + engine config
/// parse): a single config (string or object) is duplicated per layer, a
/// per-layer array must match the layer count, absent defaults to `"Tanh"`
/// (A1).
pub(crate) fn parse_activation_value(
    value: Option<&Value>,
    num_layers: usize,
    ctx: &str,
) -> Result<Vec<ActivationConfig>, String> {
    match value {
        None => Ok(vec![
            ActivationConfig::simple(ActivationKind::Tanh);
            num_layers
        ]),
        Some(Value::Array(arr)) => {
            let configs: Vec<ActivationConfig> = arr
                .iter()
                .map(ActivationConfig::from_json)
                .collect::<Result<_, String>>()?;
            if configs.len() != num_layers {
                return Err(format!(
                    "{ctx}: activation array size ({}) must match dilations size ({num_layers})",
                    configs.len()
                ));
            }
            Ok(configs)
        }
        Some(v) => Ok(vec![ActivationConfig::from_json(v)?; num_layers]),
    }
}

/// Gating modes + per-layer secondary activations. Precedence follows the
/// reference: `gating_mode` (string or per-layer array) wins over the legacy
/// `gated` boolean; both absent means no gating. Gated/blended layers without
/// an explicit `secondary_activation` default to `Sigmoid`.
#[allow(clippy::type_complexity)]
fn parse_gating(
    obj: &serde_json::Map<String, Value>,
    num_layers: usize,
    ctx: &str,
) -> Result<(Vec<GatingMode>, Vec<Option<ActivationConfig>>), String> {
    let legacy_gated = match non_null(obj, "gated") {
        Some(v) => Some(
            v.as_bool()
                .ok_or_else(|| format!("{ctx}: gated must be a boolean"))?,
        ),
        None => None,
    };
    parse_gating_config(
        non_null(obj, "gating_mode"),
        legacy_gated,
        non_null(obj, "secondary_activation"),
        num_layers,
        ctx,
    )
}

/// Core gating-config semantics, shared between the typed A2 parse above and
/// the legacy engine config path in `parse.rs` (single source of truth —
/// wired into model construction by the gating-mode todo #1106).
///
/// `gating_mode` is the raw JSON value (string or array of strings),
/// `legacy_gated` the old boolean `gated` field, and `secondary_activation`
/// the raw secondary activation config (single value or per-layer array).
#[allow(clippy::type_complexity)]
pub(crate) fn parse_gating_config(
    gating_mode: Option<&Value>,
    legacy_gated: Option<bool>,
    secondary_json: Option<&Value>,
    num_layers: usize,
    ctx: &str,
) -> Result<(Vec<GatingMode>, Vec<Option<ActivationConfig>>), String> {

    // Secondary activation for one gated/blended layer.
    let secondary_for = |layer: usize| -> Result<ActivationConfig, String> {
        match secondary_json {
            Some(Value::Array(arr)) => {
                let v = arr.get(layer).ok_or_else(|| {
                    format!(
                        "{ctx}: secondary_activation array size ({}) must be at least {}",
                        arr.len(),
                        layer + 1
                    )
                })?;
                ActivationConfig::from_json(v)
            }
            Some(v) => ActivationConfig::from_json(v),
            None => Ok(ActivationConfig::simple(ActivationKind::Sigmoid)),
        }
    };

    match gating_mode {
        Some(Value::Array(arr)) => {
            let mut modes = Vec::with_capacity(arr.len());
            let mut secondary = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                let s = v
                    .as_str()
                    .ok_or_else(|| format!("{ctx}: gating_mode array entries must be strings"))?;
                let mode = GatingMode::from_str(s)?;
                modes.push(mode);
                secondary.push(match mode {
                    GatingMode::None => None,
                    _ => Some(secondary_for(i)?),
                });
            }
            if modes.len() != num_layers {
                return Err(format!(
                    "{ctx}: gating_mode array size ({}) must match dilations size ({num_layers})",
                    modes.len()
                ));
            }
            if let Some(Value::Array(arr)) = secondary_json {
                if arr.len() != num_layers {
                    return Err(format!(
                        "{ctx}: secondary_activation array size ({}) must match dilations size ({num_layers})",
                        arr.len()
                    ));
                }
            }
            Ok((modes, secondary))
        }
        Some(Value::String(s)) => {
            let mode = GatingMode::from_str(s)?;
            let secondary = match mode {
                GatingMode::None => None,
                _ => match secondary_json {
                    // A single gating mode takes a single secondary config
                    // (an array is invalid here, as in the reference).
                    Some(v) => Some(ActivationConfig::from_json(v)?),
                    None => Some(ActivationConfig::simple(ActivationKind::Sigmoid)),
                },
            };
            Ok((vec![mode; num_layers], vec![secondary; num_layers]))
        }
        Some(_) => Err(format!(
            "{ctx}: gating_mode must be a string or an array of strings"
        )),
        None => {
            // Legacy boolean `gated`; absent means ungated.
            if legacy_gated.unwrap_or(false) {
                Ok((
                    vec![GatingMode::Gated; num_layers],
                    vec![Some(ActivationConfig::simple(ActivationKind::Sigmoid)); num_layers],
                ))
            } else {
                Ok((vec![GatingMode::None; num_layers], vec![None; num_layers]))
            }
        }
    }
}

/// One FiLM insertion-point block: absent, `null`, or literal `false` means
/// inactive; an object defaults to `{active: true, shift: true, groups: 1}`.
fn parse_film(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    ctx: &str,
) -> Result<FilmParams, String> {
    FilmParams::from_json(non_null(obj, key), ctx, key)
}

/// Slimmable packed-weight descriptor. Only `slice_channels_uniform` is
/// defined; a missing `kwargs.allowed_channels` implies every channel count
/// from 1 to `channels` (as in the reference).
fn parse_slimmable(
    obj: &serde_json::Map<String, Value>,
    channels: usize,
    ctx: &str,
) -> Result<Option<SlimmableParams>, String> {
    let Some(v) = non_null(obj, "slimmable") else {
        return Ok(None);
    };
    let s = v
        .as_object()
        .ok_or_else(|| format!("{ctx}: slimmable must be a JSON object"))?;
    let sctx = format!("{ctx}: slimmable");
    let method = non_null(s, "method")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{sctx}: missing required string field method"))?;
    if method != "slice_channels_uniform" {
        return Err(format!("{sctx}: unsupported slimmable method '{method}'"));
    }
    let allowed_channels = match non_null(s, "kwargs")
        .and_then(Value::as_object)
        .and_then(|k| non_null(k, "allowed_channels"))
    {
        Some(v) => usize_array(v, &format!("{sctx}: kwargs.allowed_channels"))?,
        None => (1..=channels).collect(),
    };
    if allowed_channels.is_empty() {
        return Err(format!("{sctx}: allowed_channels must not be empty"));
    }
    if !allowed_channels.windows(2).all(|w| w[0] < w[1]) {
        return Err(format!(
            "{sctx}: allowed_channels must be sorted strictly ascending"
        ));
    }
    if *allowed_channels.last().expect("non-empty") != channels {
        return Err(format!(
            "{sctx}: last allowed_channels entry must equal the full channel count ({channels})"
        ));
    }
    Ok(Some(SlimmableParams { allowed_channels }))
}
