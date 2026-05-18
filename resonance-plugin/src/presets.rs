use crate::param::Param;

/// Parse a preset JSON blob (`{"params": {id: value, ...}}`) and apply matching
/// parameter values via `Param::set_plain`. Returns `true` when the `"params"`
/// object was found, `false` on JSON parse failure or missing top-level key.
pub fn load<'a, F>(json: &str, count: usize, param_at: F) -> bool
where
    F: Fn(usize) -> &'a dyn Param,
{
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return false;
    };
    for i in 0..count {
        let p = param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
    true
}
