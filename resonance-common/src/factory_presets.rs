//! The first-party contract for reading a plugin's baked-in factory
//! presets from its binary (ba todo #1333).
//!
//! CLAP gives a host no way to enumerate presets that are compiled into a
//! plugin: they are `include_str!`d into the cdylib, and the only standard
//! route is the `clap.preset-discovery` factory, a much larger surface
//! aimed at third-party banks stored on disk. Our own plugins therefore
//! export one extra symbol, [`FACTORY_PRESETS_SYMBOL`], which the host
//! reads out of the library it already has open.
//!
//! This module is the shared half so the two ends cannot drift:
//! `resonance-plugin` encodes with [`encode`] inside its `export_clap!`
//! macro, and `resonance-audio` decodes with [`decode`] as it scans. It
//! lives here rather than in either of them because the host has no
//! business depending on the plugin SDK, and the plugin SDK has no
//! business depending on the engine.
//!
//! Absence of the symbol is the normal case, not an error — it means the
//! plugin is not one of ours, and "no factory presets I can see" is the
//! truthful answer for a binary we know nothing about.

/// The symbol a first-party plugin exports to publish its factory bank.
///
/// `extern "C" fn() -> *const c_char`, returning either null or a
/// NUL-terminated JSON array in the shape [`encode`] produces. The pointer
/// is valid for the lifetime of the process and is never freed by the
/// caller.
pub const FACTORY_PRESETS_SYMBOL: &[u8] = b"resonance_factory_presets";

/// Encode a factory bank as `[{"name": .., "json": ..}, ..]`.
///
/// `json` carries each preset's state document parsed, not quoted: the
/// host should receive one JSON document rather than JSON with JSON
/// escaped inside it.
///
/// Returns `None` when the result cannot be represented as a C string,
/// which in practice means an interior NUL in a name or body — malformed
/// rather than merely unusual, and better reported as "no factory presets"
/// than as a bank truncated at the NUL.
pub fn encode(presets: &[(&str, &str)]) -> Option<std::ffi::CString> {
    let entries: Vec<serde_json::Value> = presets
        .iter()
        .map(|(name, body)| {
            serde_json::json!({
                "name": name,
                "json": serde_json::from_str::<serde_json::Value>(body)
                    .unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    let text = serde_json::to_string(&serde_json::Value::Array(entries)).ok()?;
    std::ffi::CString::new(text).ok()
}

/// Decode what [`encode`] produced, as `(name, state json)` pairs.
///
/// Malformed entries are skipped rather than failing the whole bank: a
/// plugin from a newer build may carry fields this host does not know, and
/// dropping all of its presets would be the worse answer.
pub fn decode(text: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let body = entry.get("json")?;
            Some((name, serde_json::to_string(body).ok()?))
        })
        .collect()
}
