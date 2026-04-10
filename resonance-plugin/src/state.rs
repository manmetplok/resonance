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

/// Serialize parameters to bytes.
pub fn save_params(params: &[&dyn Param]) -> Vec<u8> {
    let json = params_to_json(params);
    serde_json::to_vec(&json).unwrap_or_default()
}

/// Deserialize parameters from bytes.
pub fn load_params(params: &[&dyn Param], data: &[u8]) -> bool {
    let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) else {
        return false;
    };
    load_params_from_json(params, &state)
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
                param_values[i].store(val.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    true
}
