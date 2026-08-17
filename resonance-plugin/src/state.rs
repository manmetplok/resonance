//! State serialization for plugin parameters.
//!
//! Default format is plain JSON:
//! `{ "version": <u32>, "params": { "id": value, ... } }`
//! Plugins can override save_state/load_state to add custom fields at the
//! top level (the preset identity written by
//! [`crate::presets::PresetSession`] is one such field).
//!
//! ## The version field
//!
//! Every blob written from [`params_to_json`] carries [`STATE_VERSION`].
//! It exists because parameters are matched **by string id**: rename an
//! id and every project and preset on disk silently restores that
//! parameter to its default, with nothing anywhere recording that a
//! rename happened. With a version stamped into the file, a plugin that
//! renames an id declares a [`ParamRename`] (through
//! `ResonancePlugin::param_renames`) and old files keep loading.
//!
//! State written before the field existed parses as version `0`
//! ([`version_of`]), which is exactly what a rename migration wants:
//! everything already on a user's disk is "older than version 1".

use crate::param::Param;

/// Current version of the plugin state format.
///
/// Bump this **only** together with the migration that handles the older
/// shape — see [`ParamRename`] and [`migrate`]. History:
///
/// - `0` — implicit: no `"version"` key at all. Every project saved
///   before ba todo #1332, and every hand-written factory preset.
/// - `1` — `"version"` stamped alongside `"params"`.
pub const STATE_VERSION: u32 = 1;

/// Top-level key carrying [`STATE_VERSION`].
pub const VERSION_KEY: &str = "version";

/// One parameter id rename, declared by a plugin so that state written
/// before the rename still loads.
///
/// `since_version` is the [`STATE_VERSION`] the plugin's format reached
/// **when the rename happened**: the migration runs for any blob whose
/// own version is lower than that, and is skipped for newer blobs (which
/// already use the new id). Renames are applied oldest-first, so a
/// parameter renamed twice migrates through both hops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamRename {
    /// State version in which the new id started being written.
    pub since_version: u32,
    /// The id as it appears in old files.
    pub from: &'static str,
    /// The id the plugin declares today.
    pub to: &'static str,
}

/// Serialize all parameters to a JSON value.
pub fn params_to_json(params: &[&dyn Param]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for p in params {
        map.insert(p.id().to_string(), serde_json::json!(p.get_plain()));
    }
    serde_json::json!({ VERSION_KEY: STATE_VERSION, "params": map })
}

/// The state-format version a blob was written with.
///
/// Blobs written before the field existed report `0` rather than
/// failing — they are perfectly loadable, they just predate versioning.
/// A value from a *newer* build is returned as-is so callers can decide
/// (the loaders below still apply every id they recognise, which is the
/// useful behaviour: forward-compatible on the params they share).
pub fn version_of(state: &serde_json::Value) -> u32 {
    state
        .get(VERSION_KEY)
        .and_then(|v| v.as_u64())
        .map(|v| v.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

/// Bring an older state blob up to the current format in place, and
/// report the version it was written with.
///
/// Today the only migration is parameter-id renames: for every
/// [`ParamRename`] newer than the blob, the value stored under the old
/// id is moved to the new one. An id the plugin already writes under its
/// new name is never overwritten, so re-running the migration is a no-op
/// and a blob that carries both keys keeps the current one.
///
/// Blobs from a *newer* build are left alone (nothing here knows how to
/// undo a future migration) but are still loaded key-by-key by the
/// loaders below.
pub fn migrate(state: &mut serde_json::Value, renames: &[ParamRename]) -> u32 {
    let from_version = version_of(state);
    if from_version >= STATE_VERSION || renames.is_empty() {
        // Nothing to do: either current/newer, or the plugin has never
        // renamed a param id. Still stamp the version so a re-save from
        // an untouched blob is not mistaken for pre-versioned state.
        return from_version;
    }

    let mut applicable: Vec<&ParamRename> = renames
        .iter()
        .filter(|r| r.since_version > from_version)
        .collect();
    applicable.sort_by_key(|r| r.since_version);

    if let Some(map) = state
        .get_mut("params")
        .and_then(|v| v.as_object_mut())
        .filter(|_| !applicable.is_empty())
    {
        for rename in applicable {
            if map.contains_key(rename.to) {
                // The new id is already present — the file is further
                // along than its version claims, or the plugin writes
                // both. Never clobber the current value.
                map.remove(rename.from);
                continue;
            }
            if let Some(value) = map.remove(rename.from) {
                map.insert(rename.to.to_string(), value);
            }
        }
    }

    from_version
}

/// Load parameter values from a JSON value.
///
/// Matches by string id, so a blob written by an older build loads only
/// the ids that still exist; run [`migrate`] first (the
/// `ResonancePlugin::load_state` default does) to translate ids that were
/// renamed since.
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
/// - stepped (int/bool) params are rounded to their step (and -0.0
///   normalised to +0.0),
/// - non-stepped params are demoted through f32, because that is what
///   `FloatParam::set_plain` stores.
///
/// Without this, reopening the same project restored different values
/// depending on whether the plugin happened to be active at the time.
///
/// # Renames are not migrated on this path (yet)
///
/// [`migrate`] needs the plugin's [`ParamRename`] table, which reaches
/// the inactive path through `ResonancePlugin::param_renames`. Here we
/// only have the bridge's `ParamMeta` list, which carries the *current*
/// str_id and nothing else, so a blob written before a rename loads the
/// renamed parameter at its default when the plugin happens to be active
/// at load time. No plugin declares a rename today, which is why this is
/// a documented gap rather than a live bug: closing it means giving
/// `ParamMeta` the legacy ids, which is the clap_bridge lane's change.
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
                    // `+ 0.0` normalises -0.0 to +0.0: `(-0.4).round()` is
                    // -0.0, which the inactive path never produces (it goes
                    // through i32/bool), so without this the two paths
                    // still differ in the sign bit.
                    value = value.round() + 0.0;
                } else {
                    // Demote through f32 exactly as `FloatParam::set_plain`
                    // does (`self.set_value(clamped as f32)`). Without
                    // this the two load paths still disagree for any
                    // value f32 cannot represent: `mix: 0.1` loaded while
                    // INACTIVE becomes f32 0.1 and mirrors back as
                    // 0.10000000149011612, while loading it ACTIVE leaves
                    // 0.1 in the atomics — and `get_value` reads the
                    // atomics either way, so the host reports and re-saves
                    // a different number depending on plugin state, which
                    // is the divergence this function exists to close.
                    // Stepped params hold integers, which are exact.
                    value = value as f32 as f64;
                }
                param_values[i].store(value.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    true
}
