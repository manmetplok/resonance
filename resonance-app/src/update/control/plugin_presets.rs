//! Plugin presets over the control API, shared by all three chain
//! surfaces (ba todo #1333).
//!
//! `track.*`, `bus.*` and `master.*` each declare their own three
//! methods, because addressing a plugin differs per surface. Everything
//! after "which plugin instance is this?" is the same, and lives here.
//!
//! # One library
//!
//! Both sets come from the same `PresetLibrary` the plugin's own window
//! uses ([`crate::plugin_preset_library`]): **factory** presets from the
//! scan (read from each first-party binary's `resonance_factory_presets`
//! symbol, ids and metadata included), **user** presets from the preset
//! directory. A preset saved, starred or renamed on either side is on the
//! other.
//!
//! # The whole sound (slice P2)
//!
//! A recall sets the params through the app's own path (below) and then
//! hands the plugin the preset's state document with its identity
//! (`AudioCommand::LoadPluginPresetState`): a first-party plugin lays it
//! over its current state through `clap.state-context` (`FOR_PRESET`), so
//! a model, an IR or user wavetables come along and the plugin's bar names
//! the preset. A save captures the plugin's preset form
//! (`SavePluginPresetState`), not the whole project state.
//!
//! # Why a recall is applied parameter by parameter
//!
//! The engine has `LoadPluginState`, and handing the blob to the plugin
//! would be less code. It would also desynchronise the app: its parameter
//! mirror is filled once at instantiation and updated only for changes
//! that went through `SetPluginParam`, so a plugin-side load moves the
//! sound while `track.plugin_params` keeps reporting the old values (the
//! gap ba todo #1294 closes). Applying the preset through the app's own
//! path keeps the mirror, the engine and the plugin window agreeing, and
//! [`PluginMessage::LoadPluginPreset`] carries the whole set so it is
//! still one undo entry.
//!
//! A preset's JSON is keyed by each parameter's **string** id (`"mix"`),
//! while the app's mirror knows the CLAP numeric id. They are the same
//! thing hashed: `clap_id == stable_hash(string_id)`, which is how the
//! plugin bridge derives it, so [`resonance_plugin::stable_hash`] is the
//! translation.

use crate::message::{Message, PluginMessage};
use crate::Resonance;
use resonance_audio::types::PluginInstanceId;
use resonance_control::methods::plugin_preset::{
    PluginPresetEntry, PluginPresetSource, PluginPresetsView,
};
use resonance_control::methods::track::PluginParamView;
use resonance_control::RpcError;
use resonance_plugin::presets::{PresetBank, PresetRef, PresetSource, PRESET_STATE_KEY};

/// The bank for one plugin over the app's library.
pub(crate) fn bank_for(app: &Resonance, clap_id: &str) -> PresetBank {
    crate::plugin_preset_library::bank(app, clap_id)
}

fn wire_source(source: PresetSource) -> PluginPresetSource {
    match source {
        PresetSource::Factory => PluginPresetSource::Factory,
        PresetSource::User => PluginPresetSource::User,
    }
}

/// Every preset available for one plugin instance, factory first.
pub(crate) fn view(app: &Resonance, clap_id: &str) -> PluginPresetsView {
    let presets = bank_for(app, clap_id)
        .list()
        .into_iter()
        .map(|p| PluginPresetEntry {
            name: p.name,
            source: wire_source(p.source),
        })
        .collect();
    PluginPresetsView {
        plugin_id: clap_id.to_string(),
        presets,
        // The loaded-preset identity reaches the app with P5; until an
        // identity event has arrived, `None` is the honest answer.
        current: None,
        modified: false,
    }
}

/// The preset a name (and optional source) addresses: user presets first
/// unless the caller asks for the factory original, names compared
/// case-insensitively (they are unique per plugin among user presets).
pub(crate) fn find(
    app: &Resonance,
    clap_id: &str,
    preset: &str,
    source: Option<PluginPresetSource>,
) -> Result<(PresetBank, PresetRef), RpcError> {
    let wanted = preset.trim();
    let bank = bank_for(app, clap_id);
    let all = bank.list();
    let pick = |s: PresetSource| {
        all.iter()
            .find(|p| p.source == s && p.name.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    let found = match source {
        Some(PluginPresetSource::User) => pick(PresetSource::User),
        Some(PluginPresetSource::Factory) => pick(PresetSource::Factory),
        None => pick(PresetSource::User).or_else(|| pick(PresetSource::Factory)),
    };
    match found {
        Some(p) => Ok((bank, p)),
        None => {
            let known: Vec<String> = all.into_iter().map(|p| p.name).collect();
            Err(RpcError::not_found(format!(
                "plugin {clap_id:?} has no preset {wanted:?} (has: [{}])",
                known.join(", ")
            )))
        }
    }
}

/// Build the one-edit recall message for a preset, or explain why not.
///
/// `params` is the plugin's parameter list as the app mirrors it, which
/// is what maps the preset's string ids onto CLAP ids and lets an unknown
/// id be reported rather than silently dropped.
pub(crate) fn load_message(
    app: &Resonance,
    clap_id: &str,
    instance_id: PluginInstanceId,
    params: &[PluginParamView],
    preset: &str,
    source: Option<PluginPresetSource>,
) -> Result<Message, RpcError> {
    let (bank, found) = find(app, clap_id, preset, source)?;
    load_message_for(&bank, &found, instance_id, params)
}

/// [`load_message`] for a preset already resolved.
pub(crate) fn load_message_for(
    bank: &PresetBank,
    found: &PresetRef,
    instance_id: PluginInstanceId,
    params: &[PluginParamView],
) -> Result<Message, RpcError> {
    let preset = found.name.as_str();
    let json = bank
        .json_for(found)
        .ok_or_else(|| RpcError::not_found(format!("preset {preset:?} is gone")))?;
    let mut document: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| RpcError::internal(format!("preset {preset:?} is not valid JSON: {e}")))?;
    let Some(map) = document.get("params").and_then(|v| v.as_object()) else {
        return Err(RpcError::internal(format!(
            "preset {preset:?} carries no params object"
        )));
    };

    // Index the mirror by CLAP id once, so a 90-parameter preset is not
    // a quadratic walk.
    let known: std::collections::HashSet<u32> = params.iter().map(|p| p.id).collect();
    let mut values = Vec::with_capacity(map.len());
    let mut unknown = Vec::new();
    for (string_id, value) in map {
        let Some(value) = value.as_f64() else { continue };
        let clap_id_hash = resonance_plugin::stable_hash(string_id);
        if known.contains(&clap_id_hash) {
            values.push((clap_id_hash, value));
        } else {
            unknown.push(string_id.clone());
        }
    }

    if values.is_empty() {
        return Err(RpcError::internal(format!(
            "preset {preset:?} names none of this plugin's parameters — it was \
             probably written for a different plugin (its keys: [{}])",
            unknown.join(", ")
        )));
    }

    // The whole sound, with the identity the plugin's own bar shows.
    if let Some(obj) = document.as_object_mut() {
        obj.insert(
            PRESET_STATE_KEY.to_string(),
            serde_json::json!({
                "id": found.id,
                "name": found.name,
                "source": found.source.as_str(),
                "modified": false,
            }),
        );
    }
    let preset_state = serde_json::to_vec(&document).ok();

    // A preset from an older build may name parameters this plugin no
    // longer has. That is exactly the case `ParamRename` migration
    // handles inside the plugin, and dropping the strays is what the
    // plugin's own loader does too, so recall the rest rather than
    // refusing the lot.
    Ok(Message::Plugin(PluginMessage::LoadPluginPreset {
        instance_id,
        values,
        preset_name: found.name.clone(),
        preset_state,
    }))
}

/// Check a save is allowed before anything is written.
///
/// Overwriting an existing user preset is destructive, so it takes an
/// explicit `overwrite`, per the control API's convention. Factory
/// presets are never touched: saving under a factory name creates a user
/// preset that shadows it, which is what the plugin's own window does.
pub(crate) fn check_save(
    app: &Resonance,
    clap_id: &str,
    name: &str,
    overwrite: bool,
) -> Result<(), RpcError> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Err(RpcError::invalid_params("a preset needs a name"));
    }
    let exists = bank_for(app, clap_id)
        .list_user()
        .into_iter()
        .any(|p| p.name.eq_ignore_ascii_case(wanted));
    if exists && !overwrite {
        return Err(RpcError::needs_confirmation(format!(
            "a user preset named {wanted:?} already exists for {clap_id:?}; \
             pass overwrite: true to replace it"
        )));
    }
    Ok(())
}

/// Write the plugin's preset state as a user preset.
///
/// Called when the engine's `PluginPresetStateSaved` echo lands: the
/// plugin is the only thing that knows its current sound (edits made in
/// its own window never reach the app's mirror). `preset_form` says the
/// plugin wrote its preset form (params that belong in a preset plus the
/// sound-bearing extra state); otherwise the blob is its whole state.
pub(crate) fn write_saved_state(
    app: &Resonance,
    clap_id: &str,
    name: &str,
    blob: &[u8],
) -> Result<(), String> {
    let text = std::str::from_utf8(blob).map_err(|e| format!("plugin state is not utf-8: {e}"))?;
    let document: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("plugin state is not JSON: {e}"))?;

    bank_for(app, clap_id)
        .write_user_preset(name.trim(), &document)
        .map(|_| ())
}
