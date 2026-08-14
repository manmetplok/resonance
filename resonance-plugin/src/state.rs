/// State serialization for plugin parameters.
///
/// Default format is plain JSON: `{ "params": { "id": value, ... } }`
/// Plugins can override save_state/load_state to add custom fields at the top level.
use crate::param::Param;

/// Serialize all parameters to a JSON value.
pub fn params_to_json(params: &[&dyn Param]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for p in params {
        map.insert(p.id().to_string(), serde_json::json!(p.get_plain()));
    }
    serde_json::json!({ "params": map })
}

/// Load parameter values from a JSON value.
pub fn load_params_from_json(params: &[&dyn Param], state: &serde_json::Value) -> bool {
    let Some(param_map) = state.get("params").and_then(|v| v.as_object()) else {
        return false;
    };
    for p in params {
        if let Some(val) = param_map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(val);
        }
    }
    true
}

/// Load params from a pre-parsed JSON Value into shared atomic storage
/// (used when the plugin is in the audio processor and the bridge is
/// serving save/load from shared state).
///
/// This is the *same* on-disk format [`load_params_from_json`] reads, just
/// written into the shared atomics instead of into the plugin's own
/// params, so it must land on exactly the same value: the host reads the
/// atomics back through `params.get_value` and re-saves them through
/// `state.save` while the plugin is active, and the audio thread copies
/// them into the plugin on its next block. That means reproducing the
/// guards [`Param::set_plain`] applies, because nothing else on this path
/// does:
///
/// - non-finite values are ignored (JSON cannot carry them today, but a
///   future format could),
/// - values are clamped to the param's declared range, so a hand-edited
///   or corrupted preset can't push the DSP outside what it handles,
/// - stepped (int/bool) params are rounded to their step.
///
/// Without this, reopening the same project restored different values
/// depending on whether the plugin happened to be active at the time.
pub(crate) fn load_params_from_shared_json(
    param_metas: &[crate::clap_bridge::ParamMeta],
    param_values: &[std::sync::atomic::AtomicU64],
    state: &serde_json::Value,
) -> bool {
    let Some(param_map) = state.get("params").and_then(|v| v.as_object()) else {
        return false;
    };
    for (i, meta) in param_metas.iter().enumerate() {
        if let Some(val) = param_map.get(&meta.str_id).and_then(|v| v.as_f64()) {
            if i < param_values.len() {
                if !val.is_finite() {
                    continue;
                }
                let mut value = val.clamp(meta.min, meta.max);
                if meta.is_stepped {
                    value = value.round();
                }
                param_values[i].store(value.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    true
}
